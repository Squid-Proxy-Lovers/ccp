# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Fail when a change removes tests or assertions, or skips tests, relative to its base.

Making a failing suite pass by deleting or disabling checks is a common way for
changes to hide regressions. Counts are compared per file against the merge base.
A reviewed exception is allowed when CCP_ALLOW_TEST_REDUCTION=1, which CI sets
only for pull requests carrying the `allow-test-reduction` label.

Python uses `ast` when it parses, otherwise a conservative `tokenize` fallback.
Comments and string contents (including f-strings and triple-quoted strings)
are removed before counting. Rust and `.mjs` are
counted with regexes run over a hand-written lexer's output, since their
richer grammars make a full parser too large for this script; the lexer's
job is only to classify comments, strings, template literals and regex
literals so the regexes never see one by mistake.

Known limits (this is a counter, not a language interpreter):
- Rust only recognizes the literal false cfgs `any()` and `not(all())` in the
  test's own attribute group; platform/feature predicates and enclosing module
  cfgs are not evaluated. Macro-generated tests/assertions are not expanded.
- Python handles literal `if False` bodies and simple module-level skip-factory
  assignments, not general control flow, import aliases, reassignment/data flow,
  deeply aliased or dynamic skips. Removing TestCase inheritance is not detected.
  Tokenize fallback counts syntax patterns, not AST semantics; it cannot fully
  interpret unsupported or incomplete syntax or resolve aliases/dead branches.
- JS recognizes direct test/it calls and t.test with a literal first argument;
  other subtest receiver names and aliased/destructured test/assert APIs are not
  resolved. Options must be inline objects in the first two arguments; dynamic
  keys, spreads, shared objects and computed truthiness are not evaluated.
  Unknown option values and shorthand skip/todo keys conservatively count.
- JS regex/division uses preceding tokens, not a full grammar: the closing-paren
  ambiguity in `if (cond) /regex/` remains. Only static string/template keys
  with ordinary escapes are decoded, not interpolated or computed expressions.
- Shell only recognizes standalone colon heredocs, not arbitrary shell grammar.
- Equal counts do not prove assertions are meaningful or tests still run.

"""

from __future__ import annotations

import argparse
import ast
import io
import os
import re
import shutil
import subprocess
import token
import tokenize
from collections import Counter
from collections.abc import Iterator
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE_SUFFIXES = (".rs", ".py", ".sh", ".mjs")

# ── Shell: strip heredoc-as-comment bodies (`: <<'EOF' ... EOF`) ────────────

_SHELL_HEREDOC_START = re.compile(
    r"""^[ \t]*:\s*<<(?P<tabs>-)?\s*(?P<quote>['"]?)\\?(?P<delimiter>[A-Za-z_][A-Za-z0-9_]*)(?P=quote)[ \t]*$"""
)

SHELL_PATTERNS = {
    "skips": re.compile(r"^\s*skip\s", re.MULTILINE),
    "assertions": re.compile(r"^\s*expect_(?:success|contains|not_contains)\b", re.MULTILINE),
}


def _strip_shell_heredoc_comments(text: str) -> str:
    lines = text.split("\n")
    out: list[str] = []
    i, n = 0, len(lines)
    while i < n:
        match = _SHELL_HEREDOC_START.match(lines[i])
        if match is None:
            out.append(lines[i])
            i += 1
            continue
        delimiter = match.group("delimiter")
        out.append("")
        i += 1
        while i < n and (lines[i].lstrip("\t") if match.group("tabs") else lines[i]) != delimiter:
            out.append("")
            i += 1
        if i < n:
            out.append("")
            i += 1
    return "\n".join(out)


def count_shell(text: str) -> Counter[str]:
    code = _strip_shell_heredoc_comments(text)
    return Counter(
        {category: len(pattern.findall(code)) for category, pattern in SHELL_PATTERNS.items()}
    )


# ── Python: exact counting with ast ─────────────────────────────────────────

_PY_SKIP_DECORATOR_NAMES = {"skip", "skipIf", "skipUnless", "expectedFailure"}
_PY_SKIP_MARK_NAMES = {"skip", "skipif", "xfail"}


def _py_decorator_root_and_attrs(node: ast.expr) -> tuple[ast.expr, list[str]]:
    """Walk an Attribute chain down to its root, collecting attr names."""
    attrs: list[str] = []
    while isinstance(node, ast.Attribute):
        attrs.append(node.attr)
        node = node.value
    attrs.reverse()
    return node, attrs


def _py_is_skip_decorator(decorator: ast.expr, aliases: frozenset[str] = frozenset()) -> bool:
    target = decorator.func if isinstance(decorator, ast.Call) else decorator
    if isinstance(target, ast.Name):
        return target.id in _PY_SKIP_DECORATOR_NAMES or target.id in aliases
    if not isinstance(target, ast.Attribute):
        return False
    root, attrs = _py_decorator_root_and_attrs(target)
    if (
        isinstance(root, ast.Name)
        and root.id == "unittest"
        and attrs
        and attrs[-1] in _PY_SKIP_DECORATOR_NAMES
        and attrs[:-1] in ([], ["case"])
    ):
        return True
    if attrs and attrs[-1] in _PY_SKIP_MARK_NAMES:
        # pytest.mark.skip(...)/pytest.mark.skipif(...), or `mark.skip(...)`
        # when `mark` was imported directly from pytest.
        prior = attrs[-2] if len(attrs) >= 2 else (root.id if isinstance(root, ast.Name) else None)
        return prior == "mark"
    return False


def _py_is_skip_call(call: ast.Call) -> bool:
    func = call.func
    if isinstance(func, ast.Attribute):
        if func.attr == "skipTest":
            return True
        if func.attr == "skip" and isinstance(func.value, ast.Name) and func.value.id == "pytest":
            return True
    return False


def _py_is_skip_test_reference(node: ast.expr) -> bool:
    if isinstance(node, ast.Name):
        return node.id == "SkipTest"
    if isinstance(node, ast.Attribute):
        return node.attr == "SkipTest"
    if isinstance(node, ast.Call):
        return _py_is_skip_test_reference(node.func)
    return False


def _py_is_assert_call(call: ast.Call) -> bool:
    func = call.func
    if not isinstance(func, ast.Attribute):
        return False
    if func.attr.startswith("assert") and isinstance(func.value, ast.Name):
        return func.value.id == "self"
    return func.attr == "raises" and isinstance(func.value, ast.Name) and func.value.id == "pytest"


def _py_skip_flag(node: ast.AST) -> bool:
    if not isinstance(node, (ast.Assign, ast.AnnAssign)):
        return False
    if not isinstance(node.value, ast.Constant) or node.value.value is not True:
        return False
    targets = node.targets if isinstance(node, ast.Assign) else [node.target]
    return any(
        (isinstance(t, ast.Name) and t.id == "__unittest_skip__")
        or (isinstance(t, ast.Attribute) and t.attr == "__unittest_skip__")
        for t in targets
    )


def _py_skip_aliases(tree: ast.Module) -> frozenset[str]:
    aliases: set[str] = set()
    for node in tree.body:
        if isinstance(node, ast.Assign) and _py_is_skip_decorator(node.value):
            aliases.update(t.id for t in node.targets if isinstance(t, ast.Name))
    return frozenset(aliases)


def _py_live_nodes(node: ast.AST) -> Iterator[ast.AST]:
    yield node
    children = ast.iter_child_nodes(node)
    if (
        isinstance(node, ast.If)
        and isinstance(node.test, ast.Constant)
        and node.test.value is False
    ):
        children = iter(node.orelse)
    for child in children:
        yield from _py_live_nodes(child)


def _py_node_counts(node: ast.AST, aliases: frozenset[str]) -> Counter[str]:
    totals: Counter[str] = Counter()
    if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
        if not isinstance(node, ast.ClassDef) and node.name.startswith("test_"):
            totals["tests"] += 1
        totals["skips"] += sum(_py_is_skip_decorator(d, aliases) for d in node.decorator_list)
    elif isinstance(node, ast.Assert):
        totals["assertions"] += 1
    elif isinstance(node, ast.Call):
        totals["assertions"] += _py_is_assert_call(node)
        totals["skips"] += _py_is_skip_call(node)
    elif isinstance(node, ast.Raise) and node.exc is not None:
        totals["skips"] += _py_is_skip_test_reference(node.exc)
    totals["skips"] += _py_skip_flag(node)
    return totals


_PY_FALLBACK_PATTERNS = {
    "tests": re.compile(r"\bdef\s+test_\w+"),
    "assertions": re.compile(
        r"\bassert\b|\bself\s*\.\s*assert\w*\s*\(|\bpytest\s*\.\s*raises\s*\("
    ),
    "skips": re.compile(
        r"@\s*(?:(?:unittest\s*\.\s*)?(?:skip(?:If|Unless)?|expectedFailure)"
        r"|(?:pytest\s*\.\s*)?mark\s*\.\s*(?:skipif|skip|xfail))\b"
        r"|\bpytest\s*\.\s*skip\s*\(|\bself\s*\.\s*skipTest\s*\("
        r"|\braise\s+(?:unittest\s*\.\s*)?SkipTest\b"
    ),
}


def _python_tokens(text: str) -> str:
    """Keep only code tokens, dropping whole f-strings including interpolations."""
    out: list[tuple[int, str]] = []
    depth = 0
    for item in tokenize.generate_tokens(io.StringIO(text).readline):
        name = token.tok_name[item.type]
        if name == "FSTRING_START":
            depth += 1
        elif name == "FSTRING_END":
            depth -= 1
        elif not depth and item.type not in (tokenize.COMMENT, tokenize.STRING):
            out.append((item.type, item.string))
    return tokenize.untokenize(out)


def count_python(text: str) -> Counter[str]:
    try:
        tree = ast.parse(text)
    except SyntaxError:
        try:
            code = _python_tokens(text)
        except (tokenize.TokenError, IndentationError) as error:
            raise SyntaxError(f"cannot tokenize Python: {error}") from error
        return Counter(
            {key: len(pattern.findall(code)) for key, pattern in _PY_FALLBACK_PATTERNS.items()}
        )
    totals: Counter[str] = Counter()
    aliases = _py_skip_aliases(tree)
    for node in _py_live_nodes(tree):
        totals.update(_py_node_counts(node, aliases))
    return totals


# ── Rust: hand-written lexer, then regex over the stripped result ──────────

RUST_PATTERNS = {
    "tests": re.compile(r"#\s*\[\s*(?:tokio\s*::\s*)?test\b"),
    "skips": re.compile(r"#\s*\[\s*ignore\b|#\s*\[\s*cfg_attr\b[^\]]*\bignore\b"),
    "assertions": re.compile(r"\b(?:debug_)?assert(?:_eq|_ne)?\s*!"),
}

_RUST_ATTRIBUTE_GROUP = re.compile(r"(?:#\s*\[[^\]]*\]\s*)+")
_RUST_FALSE_CFG = re.compile(
    r"#\s*\[\s*cfg\s*\(\s*(?:any\s*\(\s*\)|not\s*\(\s*all\s*\(\s*\)\s*\))\s*\)\s*\]"
)


_HEX_DIGITS = "0123456789abcdefABCDEF"


def _rust_unicode_end(text: str, j: int) -> int | None:
    match = re.match(r"\{([0-9a-fA-F][0-9a-fA-F_]*)\}", text[j:])
    if match is None:
        return None
    digits = match.group(1).replace("_", "")
    if len(digits) > 6:
        return None
    scalar = int(digits, 16)
    if scalar > 0x10FFFF or 0xD800 <= scalar <= 0xDFFF:
        return None
    return j + match.end()


def _rust_escape_end(text: str, j: int, *, byte: bool) -> int | None:
    """j points at a backslash; return the index after a valid escape."""
    n = len(text)
    if j + 1 >= n:
        return None
    esc = text[j + 1]
    if esc in "nrt\\0'\"":
        return j + 2
    if esc == "x" and j + 4 <= n:
        digits = text[j + 2 : j + 4]
        if all(c in _HEX_DIGITS for c in digits) and (byte or int(digits, 16) < 128):
            return j + 4
    if esc == "u" and not byte:
        return _rust_unicode_end(text, j + 2)
    return None


def _rust_char_literal_end(text: str, i: int, *, byte: bool = False) -> int | None:
    """i points at the opening quote of a possible char/byte-char literal."""
    n = len(text)
    j = i + 1
    if j >= n or text[j] in "'\n\r\t":
        return None
    if text[j] == "\\":
        escaped = _rust_escape_end(text, j, byte=byte)
        if escaped is None:
            return None
        j = escaped
    else:
        if byte and ord(text[j]) > 127:
            return None
        j += 1
    return j + 1 if j < n and text[j] == "'" else None


def _rust_quoted_end(text: str, content_start: int, *, raw: bool, hashes: int) -> int:
    """content_start is just past the opening quote; return index past the close."""
    n = len(text)
    closer = '"' + "#" * hashes
    j = content_start
    while j < n:
        if raw:
            if text[j : j + len(closer)] == closer:
                return j + len(closer)
            j += 1
        elif text[j] == "\\":
            j += 2
        elif text[j] == '"':
            return j + 1
        else:
            j += 1
    return n


def _rust_prefixed_literal(text: str, i: int) -> int | None:
    """i points at 'b' or 'r'; try b"..", r"..", r#".."#, br"..", br#".."#, b'.'."""
    n = len(text)
    j = i
    is_byte = text[j] == "b"
    if is_byte:
        j += 1
    is_raw = j < n and text[j] == "r"
    if is_raw:
        j += 1
    if not is_byte and not is_raw:
        return None
    hashes = 0
    if is_raw:
        while j < n and text[j] == "#":
            hashes += 1
            j += 1
    if j < n and text[j] == '"':
        return _rust_quoted_end(text, j + 1, raw=is_raw, hashes=hashes)
    if j < n and text[j] == "'" and is_byte and not is_raw:
        return _rust_char_literal_end(text, j, byte=True)
    return None


def _line_comment_end(text: str, i: int) -> int:
    n = len(text)
    while i < n and text[i] != "\n":
        i += 1
    return i


def _rust_skip_block_comment(text: str, i: int) -> int:
    """i points at the opening '/*'; handles arbitrary nesting."""
    n = len(text)
    depth = 1
    i += 2
    while i < n and depth:
        two = text[i : i + 2]
        if two == "/*":
            depth += 1
            i += 2
        elif two == "*/":
            depth -= 1
            i += 2
        else:
            i += 1
    return i


def _blank(text: str) -> str:
    """Preserve offsets and line breaks without joining neighboring tokens."""
    return re.sub(r"[^\n]", " ", text)


def _rust_token_end(text: str, i: int) -> int | None:
    ch = text[i]
    if text.startswith("//", i):
        return _line_comment_end(text, i)
    if text.startswith("/*", i):
        return _rust_skip_block_comment(text, i)
    if ch == '"':
        return _rust_quoted_end(text, i + 1, raw=False, hashes=0)
    if ch == "'":
        return _rust_char_literal_end(text, i)
    if ch in "br":
        return _rust_prefixed_literal(text, i)
    return None


def strip_rust(text: str) -> str:
    out: list[str] = []
    i = 0
    while i < len(text):
        end = _rust_token_end(text, i)
        if end is None:
            out.append(text[i])
            i += 1
        else:
            out.append(_blank(text[i:end]))
            i = end
    return "".join(out)


def count_rust(text: str) -> Counter[str]:
    code = strip_rust(text)
    totals = Counter(
        {category: len(pattern.findall(code)) for category, pattern in RUST_PATTERNS.items()}
    )
    totals["skips"] += sum(
        bool(RUST_PATTERNS["tests"].search(group[0]) and _RUST_FALSE_CFG.search(group[0]))
        for group in _RUST_ATTRIBUTE_GROUP.finditer(code)
    )
    return totals


# ── JavaScript (.mjs): hand-written lexer, then regex ───────────────────────

_JS_TEST_NAME = r"(?<![\w.])(?:t\s*\.\s*)?(?:test|it)"
_JS_MEMBER = r"(?:\s*\.\s*\w+|\s*\[\s*['\"`](?:skip|todo)['\"`]\s*\])"
MJS_PATTERNS = {
    "tests": re.compile(_JS_TEST_NAME + _JS_MEMBER + r"*\s*\("),
    "assertions": re.compile(r"\bassert(?:\s*\.\s*\w+)*\s*\("),
}
_JS_SKIP_ACCESS = re.compile(
    _JS_TEST_NAME + r"(?:\s*\.\s*\w+)*?\s*"
    r"(?:\.\s*(?:skip|todo)\b|\[\s*['\"`](?:skip|todo)['\"`]\s*\])"
)
_JS_TOKENS = re.compile(r"\"[^\"\n]*\"|'[^'\n]*'|`[^`]*`|\w+|[^\s]", re.DOTALL)


_JS_REGEX_PRECEDING_KEYWORDS = {
    "return",
    "typeof",
    "case",
    "do",
    "else",
    "in",
    "of",
    "new",
    "delete",
    "void",
    "throw",
    "instanceof",
    "yield",
}


def _js_skip_block_comment(text: str, i: int) -> int:
    """JS block comments do not nest."""
    n = len(text)
    end = text.find("*/", i + 2)
    stop = end + 2 if end != -1 else n
    return stop


def _js_decode_key(content: str) -> str:
    """Decode static key escapes; unknown escapes retain their escaped character."""

    def decode(match: re.Match[str]) -> str:
        escape = match[0][1:]
        if escape.startswith("x"):
            return chr(int(escape[1:], 16))
        if escape.startswith("u"):
            digits = escape[1:].strip("{}")
            scalar = int(digits, 16)
            return chr(scalar) if scalar <= 0x10FFFF else ""
        return {"n": "\n", "r": "\r", "t": "\t", "b": "\b", "f": "\f", "v": "\v", "0": "\0"}.get(
            escape, escape
        )

    return re.sub(r"\\(?:x[0-9a-fA-F]{2}|u[0-9a-fA-F]{4}|u\{[0-9a-fA-F]+\}|.)", decode, content)


def _js_key_literal(text: str, i: int, end: int) -> str:
    quote = text[i]
    value = _js_decode_key(text[i + 1 : end - 1])
    if value in ("skip", "todo"):
        literal = quote + value + quote
        return literal + " " * (end - i - len(literal))
    return quote + _blank(text[i + 1 : end - 1]) + quote


def _js_consume_quoted(text: str, i: int, quote: str, out: list[str]) -> int:
    j = i + 1
    while j < len(text) and text[j] not in (quote, "\n", "\r"):
        j += 2 if text[j] == "\\" else 1
    if j < len(text) and text[j] == quote:
        out.append(_js_key_literal(text, i, j + 1))
        return j + 1
    end = min(j, len(text))
    out.append(quote + _blank(text[i + 1 : end]))
    return end


def _js_scan_word(text: str, i: int) -> int:
    n = len(text)
    j = i
    while j < n and (text[j].isalnum() or text[j] in "_$"):
        j += 1
    return j


def _js_scan_number(text: str, i: int) -> int:
    n = len(text)
    j = i
    while j < n and (text[j].isalnum() or text[j] in "._"):
        j += 1
    return j


def _js_regex_end(text: str, i: int) -> int | None:
    """i points at the opening '/'; honors character classes and escapes."""
    n = len(text)
    j = i + 1
    in_class = False
    if j >= n:
        return None
    while j < n:
        ch = text[j]
        if ch == "\\":
            j += 2
            continue
        if ch == "\n":
            return None
        if ch == "[":
            in_class = True
        elif ch == "]":
            in_class = False
        elif ch == "/" and not in_class:
            j += 1
            while j < n and text[j].isalpha():
                j += 1
            return j
        j += 1
    return None


def _js_consume_slash(text: str, i: int, out: list[str]) -> tuple[int, bool]:
    end = _js_regex_end(text, i)
    if end is None:
        out.append(text[i])
        return i + 1, True
    out.append(_blank(text[i:end]))
    return end, False


def _js_code_step(text: str, i: int, out: list[str], regex_ok: bool) -> tuple[int, bool, int]:
    ch = text[i]
    if text[i : i + 2] in ("++", "--"):
        out.append(text[i : i + 2])
        return i + 2, False, 0
    if ch == "/" and regex_ok:
        new_i, ok = _js_consume_slash(text, i, out)
        return new_i, ok, 0
    if ch.isalpha() or ch in "_$":
        j = _js_scan_word(text, i)
        word = text[i:j]
        out.append(word)
        return j, word in _JS_REGEX_PRECEDING_KEYWORDS, 0
    if ch.isdigit():
        j = _js_scan_number(text, i)
        out.append(text[i:j])
        return j, False, 0
    out.append(ch)
    if ch.isspace():
        return i + 1, regex_ok, 0
    delta = {"{": 1, "}": -1}.get(ch, 0)
    return i + 1, ch in "(,=:[!&|?{};+-*%~^<>/", delta


def _js_step(text: str, i: int, out: list[str], regex_ok: bool) -> tuple[int, bool, int]:
    """Consume one token; return (end, regex_allowed, brace_delta)."""
    ch = text[i]
    end = None
    if text.startswith("//", i):
        end = _line_comment_end(text, i)
    elif text.startswith("/*", i):
        end = _js_skip_block_comment(text, i)
    if end is not None:
        out.append(_blank(text[i:end]))
        return end, regex_ok, 0
    if ch in "\"'":
        return _js_consume_quoted(text, i, ch, out), False, 0
    if ch == "`":
        return _js_consume_template(text, i, out), False, 0
    return _js_code_step(text, i, out, regex_ok)


def _js_consume_interpolation(text: str, i: int, out: list[str]) -> int:
    """i is just past '${'; consumes up to and including the matching '}'."""
    n = len(text)
    depth = 1
    regex_ok = True
    while i < n and depth > 0:
        i, regex_ok, delta = _js_step(text, i, out, regex_ok)
        depth += delta
    return i


def _js_consume_template(text: str, i: int, out: list[str]) -> int:
    start = len(out)
    dynamic = False
    out.append("`")
    j = i + 1
    while j < len(text):
        ch = text[j]
        if ch == "`":
            out.append("`")
            if not dynamic:
                out[start:] = [_js_key_literal(text, i, j + 1)]
            return j + 1
        if text.startswith("${", j):
            dynamic = True
            out.append("${")
            j = _js_consume_interpolation(text, j + 2, out)
            continue
        end = min(j + 2, len(text)) if ch == "\\" else j + 1
        out.append(_blank(text[j:end]))
        j = end
    return j


def strip_mjs(text: str) -> str:
    out: list[str] = []
    i, n = 0, len(text)
    regex_ok = True
    while i < n:
        i, regex_ok, _ = _js_step(text, i, out, regex_ok)
    return "".join(out)


def mjs_test_matches(code: str) -> Iterator[re.Match[str]]:
    for match in MJS_PATTERNS["tests"].finditer(code):
        if code[: match.start()].rstrip().endswith("."):
            continue
        # t is the supported TestContext receiver; no arbitrary regex.test calls.
        if re.match(r"t\s*\.", match[0]) and not code[match.end() :].lstrip().startswith(
            ("'", '"', "`")
        ):
            continue
        yield match


def _js_parts(tokens: Iterator[str], closing: str) -> Iterator[list[str]]:
    """Split direct arguments/properties, leaving nested groups in their part."""
    depth = 0
    part: list[str] = []
    for value in tokens:
        if not depth and value == closing:
            yield part
            return
        if not depth and value == ",":
            yield part
            part = []
            continue
        depth += (value in "([{") - (value in ")]}")
        part.append(value)
    yield part


def _js_skip_property(part: list[str]) -> bool:
    if not part:
        return False
    key_end = 1
    key = part[0]
    if key == "[" and len(part) >= 3 and part[2] == "]":
        key, key_end = part[1], 3
    if key.strip("'\"`") not in ("skip", "todo"):
        return False
    value = part[key_end:]
    return value not in ([":", "false"], [":", "0"], [":", "''"], [":", '""'], [":", "``"])


def _js_option_skips(code: str, match: re.Match[str]) -> int:
    tokens = (token[0] for token in _JS_TOKENS.finditer(code, match.end()))
    total = 0
    for index, argument in enumerate(_js_parts(tokens, ")")):
        if index > 1:
            break
        if argument and argument[0] == "{":
            total += sum(_js_skip_property(part) for part in _js_parts(iter(argument[1:]), "}"))
    return total


def count_mjs(text: str) -> Counter[str]:
    code = strip_mjs(text)
    tests = list(mjs_test_matches(code))
    return Counter(
        {
            "tests": len(tests),
            "assertions": len(MJS_PATTERNS["assertions"].findall(code)),
            "skips": len(_JS_SKIP_ACCESS.findall(code))
            + sum(_js_option_skips(code, match) for match in tests),
        }
    )


# ── Dispatch, snapshotting and comparison ───────────────────────────────────

_COUNTERS = {
    ".rs": count_rust,
    ".py": count_python,
    ".mjs": count_mjs,
    ".sh": count_shell,
}


def git(*args: str) -> bytes:
    executable = shutil.which("git")
    if executable is None:
        raise SystemExit("git is required to compare test counts")
    # The executable is fixed and arguments are Git revisions or tracked paths.
    return subprocess.run(  # noqa: S603
        [executable, *args], cwd=ROOT, capture_output=True, check=True
    ).stdout


def is_source(path: str) -> bool:
    # The checker itself defines the patterns it counts.
    return path.endswith(SOURCE_SUFFIXES) and path != "scripts/check-test-integrity.py"


def count(path: str, text: str) -> Counter[str]:
    suffix = Path(path).suffix
    counter = _COUNTERS.get(suffix)
    if counter is None:
        return Counter()
    try:
        return counter(text)
    except SyntaxError as error:
        raise SystemExit(f"{path}: failed to parse for test-integrity counting: {error}") from error


def read(revision: str | None, path: str) -> str:
    if revision is None:
        return (ROOT / path).read_text(errors="replace")
    return git("show", f"{revision}:{path}").decode(errors="replace")


def snapshot(revision: str | None) -> dict[str, Counter[str]]:
    """Count per file at a revision, or in the working tree when revision is None."""
    if revision is None:
        names = git("ls-files", "-z", "--cached", "--others", "--exclude-standard")
    else:
        names = git("ls-tree", "-r", "-z", "--name-only", revision)
    paths = [name.decode() for name in names.split(b"\0") if name]
    return {
        path: count(path, read(revision, path))
        for path in paths
        if is_source(path) and (revision is not None or (ROOT / path).is_file())
    }


CATEGORIES = ("tests", "skips", "assertions")


def compare(base: dict[str, Counter[str]], head: dict[str, Counter[str]]) -> list[str]:
    problems = []
    for category in CATEGORIES:
        before = sum(counts[category] for counts in base.values())
        after = sum(counts[category] for counts in head.values())
        print(f"{category}: {before} -> {after}")
        for path in sorted(set(base) | set(head)):
            old = base.get(path, Counter())[category]
            new = head.get(path, Counter())[category]
            if (category == "skips" and new > old) or (category != "skips" and new < old):
                problems.append(f"{path}: {category} {old} -> {new}")
    return problems


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", default="origin/main", help="branch the change merges into")
    args = parser.parse_args()
    merge_base = git("merge-base", args.base, "HEAD").decode().strip()
    problems = compare(snapshot(merge_base), snapshot(None))
    if not problems:
        return
    print("Tests, assertions, or skips changed in a way that weakens the suite:")
    for problem in problems:
        print(f"  {problem}")
    if os.environ.get("CCP_ALLOW_TEST_REDUCTION") == "1":
        print("Allowed by the reviewed allow-test-reduction label.")
        return
    raise SystemExit(
        "Restore the checks, or explain the reduction in the PR and ask a maintainer "
        "to apply the allow-test-reduction label."
    )


if __name__ == "__main__":
    main()
