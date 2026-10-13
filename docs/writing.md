# Documentation writing guide

Write for someone trying to complete a task or understand a contract.
[Documentation index](README.md) · [Contribution requirements](../CONTRIBUTING.md)

## Structure

- Keep the root README to purpose, installation, quick start, and links.
- Put a task's prerequisites and commands before implementation details.
- Use one title, sentence-case headings, and a short contents list for long guides.
- Use numbered steps for sequences and tables for choices, fields, or commands.
- Keep one canonical explanation of a contract; link to it from related pages.
- Link every public Markdown document from the documentation index. Add a return
  link from guides so readers can continue browsing.
- Label historical behavior with its branch and revision. Verify claims against
  the current source before describing them as supported behavior.

## Language

Use familiar words, active voice, and concrete verbs. State what an operation does
and what the reader needs to do. Explain a technical term when introducing it.

Avoid em dashes, rhetorical contrasts, slogan-like fragments, generic praise,
buzzwords, and filler such as “seamlessly,” “leverage,” “delve,” and “it's worth
noting.” Keep sentences connected and remove repeated explanations. Claims about
speed, security, automation, or reliability need evidence or a stated limit.

Keep distinctions readers need: a checksum detects changed data, while sender
authentication requires a separate trust mechanism. Describe error conditions,
permissions, and data loss risks next to the relevant operation.

## Examples and verification

- State whether commands run from a checkout, an installed CLI, or a container.
- Use shell-valid replacement values such as `YOUR_TOKEN` in executable examples.
  Reserve angle-bracket placeholders for inline reference syntax.
- Make JSON examples parseable. State which values are illustrative and which
  fields or formats are part of the contract.
- Check relative links, heading anchors, code fences, shell syntax, command flags,
  and tool signatures. Check that the index reaches every public Markdown file.
- Preserve useful information when shortening text; move it into its canonical
  guide and update links. Record intentionally removed or obsolete material.

This policy follows [Google's guidance on tone](https://developers.google.com/style/tone),
[Google's heading guidance](https://developers.google.com/style/headings), and
[Microsoft's advice on concise language](https://learn.microsoft.com/en-us/style-guide/word-choice/use-simple-words-concise-sentences).
The punctuation and rhetorical-contrast rules are this project's preferences.
