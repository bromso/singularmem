---
spec: docs/superpowers/specs/YYYY-MM-DD-<topic>-design.md
sub-project: <e.g. 1-memory-store-v0>
status: draft | ready-for-execution | in-progress | merged
target-release: <e.g. v0.1.0>
---

# <Sub-project name> Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task.

**Goal:** One sentence.

**Architecture:** Two or three sentences describing the shape of the
implementation.

**Tech Stack:** Key technologies / libraries.

---

## Approach summary

One paragraph lifted from the spec, describing how this plan delivers
the spec's recommended approach.

## Step-by-step implementation milestones

A bullet list of the major milestones in order. Each milestone maps to
one or more tasks below.

- M1 — ...
- M2 — ...

## Task list

The bite-sized tasks. Each task uses the `tasks-template.md` entry
shape. Tasks are checkbox-tracked and ordered for sequential execution
unless explicitly marked parallel.

### Task N: <Task title>

**Files:**

- Create: `path/to/file`
- Modify: `path/to/existing:line-range`

**Assigned skill:** `<skill-name>`

- [ ] **Step 1: ...**

(Repeat per step. Each step is one action of 2–5 minutes. Include
exact code, exact commands, expected output.)

## Constitution Check

| Principle | How this plan complies |
|---|---|
| **I — Local-First and Sovereign** | ... |
| **II — Provider-Agnostic by Contract** | ... |
| **III — Open Core with a Stable Boundary** | ... |
| **V — Composable Library Architecture** | ... |
| **VI — Deterministic and Offline-Testable** | ... |
| **X — Performance Budgets, Enforced in CI** | ... |

## Risks & mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| ... | ... | ... | ... |

## Verification plan

How we will know the sub-project succeeded:

- Build / lint / test commands and expected outputs.
- Acceptance criteria verification commands (one per criterion from
  the spec).
- Performance budget measurements where Principle X applies.

## Rollback plan

If applicable: how to revert this sub-project's changes if a
post-merge issue requires it. For purely additive sub-projects,
`git revert <merge-commit>` is usually sufficient and this section
may say so.
