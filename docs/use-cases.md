# Collaboration examples

Use CCP to keep findings available to other enrolled clients and to later agents.
Each scenario below uses the current shelf/book/entry model.

[Documentation index](README.md) · [Client commands](client.md#cli-reference)

## CTF collaboration

Give each challenge or research topic a shelf and group related findings in books.
One agent records a partial result; others search for it, read the entry, and append
what they learn. Labels such as `challenge:web` help the team find related work.

The archived README described this workflow in a six-person, 48-hour CTF. Its
[temporary team-status feature](archive-notes.md#branch-specific-features) belongs
to that archived branch. Current main can store progress in ordinary entries.

## Parallel code review

Use separate books for security, architecture, and test coverage. Reviewers write
findings with labels such as `severity:high` or `area:auth`. A summarizing agent
searches the shared session and creates a prioritized report that cites entry paths.

Agree on names and locations before creating entries. Multiple agents can append
to the same entry; history records the authenticated client and supplied attribution.

## Persistent research and handoffs

An agent maps an API and records endpoints, assumptions, and open questions. A later
agent enrolls in the same CCP session, searches the stored context, and appends new
results. Keep enough detail to resume the work without the earlier conversation.

Use [droplets](droplet-format.md) to hand selected research to another CCP session.
Use `brief-me` for a session overview and `history` to inspect how an entry changed.

## Two people working on one feature

Separate agent-host conversations use the same enrolled server session. Store the
approach, decisions, failed attempts, and next action in a shared book. Read existing
entries before starting a parallel task, then append progress with agent and host
attribution. Text entries record coordination decisions; collaborators still need
to agree on task ownership and completion.
