# Implementation plans

One plan per sub-project. Filename convention:
`YYYY-MM-DD-<topic>.md`. The plan implements the spec of the same
date and topic in
[`../specs/`](../specs/).

Each plan follows
[`.specify/templates/plan-template.md`](../../../.specify/templates/plan-template.md)
and uses
[`.specify/templates/tasks-template.md`](../../../.specify/templates/tasks-template.md)
for the shape of every task entry.

A plan is **executable**. It contains the exact files, code, commands,
and expected outputs needed for a fresh engineer (human or agent) to
implement the sub-project. Vague tasks ("add appropriate error
handling", "implement later") are bugs in the plan.

When a plan completes — every task done, PR merged — its frontmatter
`status` becomes `merged`. The plan stays on disk as historical
record.
