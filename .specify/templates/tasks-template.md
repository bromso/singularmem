# Tasks Template

This document defines the required shape of a task entry inside a
`docs/superpowers/plans/*.md` plan. Every task in every plan **MUST**
follow this shape.

The `assigned-skill` field is load-bearing: it is the mechanism by
which the constitutional instruction "assign relevant skills to the
tasks you create" is enforced. A task without an `assigned-skill` is
incomplete.

## Required fields

| Field | Description |
|---|---|
| `id` | Stable identifier, monotonic within the plan (e.g. `Task 1`, `Task 2`). |
| `subject` | Imperative title (e.g. "Add CI workflow", "Write the failing test"). |
| `description` | One paragraph naming the files touched and what they accomplish. |
| `acceptance-criteria` | A short numbered or bulleted list of observable outcomes that prove this task is done. |
| `assigned-skill` | A free-form string naming the most relevant skill (e.g. `rust-best-practices`, `test-driven-development`, `vitest-testing`, `biome-linting`, `accessibility-auditor`, `systematic-debugging`, `verification-before-completion`). |
| `blocks` | List of task IDs that cannot start until this task completes. May be empty. |
| `blocked-by` | List of task IDs that must complete before this task can start. May be empty. |
| `owner` | The agent or human responsible for executing this task. May be unassigned at planning time. |

## Task entry shape

```markdown
### Task N: <subject>

**Files:**

- Create: `<path>`
- Modify: `<path>:<line-range>`
- Delete: `<path>`

**Assigned skill:** `<skill-name>`

**Blocked-by:** Task K, Task M (or "none")
**Blocks:** Task P (or "none")
**Owner:** <unassigned | agent-id | name>

**Description:** One paragraph explaining what this task does and why.

**Acceptance criteria:**

1. ...
2. ...

- [ ] **Step 1: <action>**

(exact command, exact code, expected output)

- [ ] **Step 2: <action>**

...
```

## Choosing an `assigned-skill`

Match the task to the skill that most directly governs its work:

- **Writing Rust code** → `rust-best-practices`
- **Writing tests first** → `test-driven-development`
- **Writing TypeScript/JS tests** → `vitest-testing`
- **Writing/linting TypeScript** → `biome-linting`
- **Auditing accessibility** → `accessibility-auditor`
- **Diagnosing failing tests or bugs** → `systematic-debugging`
- **Confirming acceptance criteria** → `verification-before-completion`
- **Reviewing finished work** → `requesting-code-review`

If no skill fits, `verification-before-completion` is the conservative
default. If multiple skills fit, pick the one whose checklist is most
load-bearing for the task's primary action.

## Free-form vs. enum

The `assigned-skill` field is free-form in v0 because the skill catalog
is itself evolving. A future amendment may constrain it to an enum once
the catalog stabilises.
