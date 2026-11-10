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

Known limits:
- This does not evaluate `cfg`/`cfg_attr` predicates; a Rust test behind
  `#[cfg(target_os = "windows")]` still counts on every platform.
- JS destructured or aliased imports (`const {equal} = assert`) are not
  tracked; this repo's `.mjs` suites always call through `assert.*` or
  `test.*` directly.
- Python import aliases are not resolved; fallback counts syntax patterns, not
  semantics, and cannot fully interpret unsupported or incomplete syntax.
- Shell only recognizes standalone colon heredocs, not arbitrary shell grammar.
- Rust macro-generated tests and custom assertion macros are not expanded.
- JS supports node:test call forms, not aliased test/assert APIs or it/describe.
  Skip-shaped properties are conservatively counted even outside test options.
- JS regex-vs-division disambiguation uses only the previous significant
  token, not a full parse; this matches real engines closely enough for
  the straight-line test code in this repo.
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

_SHELL_HEREDOC_START = re.compile(r"^[ \t]*:\s*<<-?\s*'?([A-Za-z_][A-Za-z0-9_]*)'?[ \t]*$")

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
        delimiter = match.group(1)
        out.append("")
        i += 1
        while i < n and lines[i].lstrip("\t") != delimiter:
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


def _py_is_skip_decorator(decorator: ast.expr) -> bool:
    target = decorator.func if isinstance(decorator, ast.Call) else decorator
    if isinstance(target, ast.Name):
        return target.id in _PY_SKIP_DECORATOR_NAMES
    if not isinstance(target, ast.Attribute):
        return False
    root, attrs = _py_decorator_root_and_attrs(target)
    if (
        isinstance(root, ast.Name)
        and root.id == "unittest"
        and attrs
        and attrs[0] in _PY_SKIP_DECORATOR_NAMES
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


def _py_node_counts(node: ast.AST) -> Counter[str]:
    totals: Counter[str] = Counter()
    if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
        if not isinstance(node, ast.ClassDef) and node.name.startswith("test_"):
            totals["tests"] += 1
        totals["skips"] += sum(_py_is_skip_decorator(d) for d in node.decorator_list)
    elif isinstance(node, ast.Assert):
        totals["assertions"] += 1
    elif isinstance(node, ast.Call):
        totals["assertions"] += _py_is_assert_call(node)
        totals["skips"] += _py_is_skip_call(node)
    elif isinstance(node, ast.Raise) and node.exc is not None:
        totals["skips"] += _py_is_skip_test_reference(node.exc)
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
    for node in ast.walk(tree):
        totals.update(_py_node_counts(node))
    return totals


# ── Rust: hand-written lexer, then regex over the stripped result ──────────

RUST_PATTERNS = {
    "tests": re.compile(r"#\[(?:tokio::)?test\b"),
    "skips": re.compile(r"#\s*\[\s*ignore\b|#\s*\[\s*cfg_attr\b[^\]]*\bignore\b"),
    "assertions": re.compile(r"\b(?:debug_)?assert(?:_eq|_ne)?!"),
}

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


def _rust_skip_line_comment(text: str, i: int) -> int:
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
        return _rust_skip_line_comment(text, i)
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
    return Counter(
        {category: len(pattern.findall(code)) for category, pattern in RUST_PATTERNS.items()}
    )


# ── JavaScript (.mjs): hand-written lexer, then regex ───────────────────────

MJS_PATTERNS = {
    # Bare test(...)/test.skip(...)/test.todo(...); a subtest invoked
    # through its context parameter (t.test(...)) only counts when its
    # first argument is a string/template literal, so re.test(s) (the
    # RegExp method) is not mistaken for a node:test subtest.
    "tests": re.compile(
        r"(?<![\w.])test(?:\s*\.\s*\w+|\[\s*[\'\"](?:skip|todo)[\'\"]\s*\])*\s*\("
        r"|\b\w+\s*\.\s*test(?:\s*\.\s*\w+)*\s*\(\s*['\"`]"
    ),
    # .skip/.todo access (call, property read, or alias assignment such as
    # `const s = test.skip;`), a skip/todo key anywhere in an options
    # object (first position or not, quoted or bare, keyed or shorthand),
    # and a computed/bracket key or access (`test['skip']`, `{['skip']: 1}`).
    "skips": re.compile(
        r"\.\s*(?:skip|todo)\b"
        r"|[{,]\s*['\"]?(?:skip|todo)['\"]?\s*[:,}]"
        r"|\[\s*['\"](?:skip|todo)['\"]\s*\]"
    ),
    # Bare assert(x), assert.ok(x), and any chain depth, e.g.
    # assert.strict.equal(...), with optional whitespace around each dot.
    "assertions": re.compile(r"\bassert(?:\s*\.\s*\w+)*\s*\("),
}

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


def _js_skip_line_comment(text: str, i: int) -> int:
    n = len(text)
    while i < n and text[i] != "\n":
        i += 1
    return i


def _js_skip_block_comment(text: str, i: int, out: list[str]) -> int:
    """JS block comments do not nest."""
    n = len(text)
    end = text.find("*/", i + 2)
    stop = end + 2 if end != -1 else n
    out.append(_blank(text[i:stop]))
    return stop


def _js_consume_quoted(text: str, i: int, quote: str, out: list[str]) -> int:
    n = len(text)
    j = i + 1
    while j < n and text[j] != quote:
        j += 2 if text[j] == "\\" else 1
    end = min(j + 1, n)
    content = text[i + 1 : j]
    # Keep "skip"/"todo" verbatim: a bracket-indexed key (test['skip'],
    # {['skip']: true}) must still be visible to the skip pattern. Every
    # other string has its contents blanked, preserving quotes and offsets.
    out.append(
        text[i:end] if content in ("skip", "todo") else quote + _blank(text[i + 1 : j]) + quote
    )
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
    if text.startswith("//", i):
        end = _js_skip_line_comment(text, i)
        out.append(_blank(text[i:end]))
        return end, regex_ok, 0
    if text.startswith("/*", i):
        return _js_skip_block_comment(text, i, out), regex_ok, 0
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
    out.append("`")
    j = i + 1
    while j < len(text):
        ch = text[j]
        if ch == "`":
            out.append("`")
            return j + 1
        if text.startswith("${", j):
            out.append("  ")
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
        # Do not retry a rejected receiver.test(variable) as a bare test call
        # merely because there is whitespace between the dot and test.
        if not code[: match.start()].rstrip().endswith("."):
            yield match


def count_mjs(text: str) -> Counter[str]:
    code = strip_mjs(text)
    totals = Counter(
        {
            category: len(pattern.findall(code))
            for category, pattern in MJS_PATTERNS.items()
            if category != "tests"
        }
    )
    totals["tests"] = sum(1 for _ in mjs_test_matches(code))
    return totals


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
