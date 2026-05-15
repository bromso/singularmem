---
title: <Title of the sub-project or feature>
date: YYYY-MM-DD
status: draft | ready-for-implementation | superseded
sub-project: <e.g. 1-memory-store-v0>
supersedes: <path to prior spec, or 'none'>
---

# <Title of the sub-project or feature>

One short paragraph describing what this sub-project ships and why it
matters now.

## Problem & motivation

What problem does this solve? What is blocked until this lands? Why is
this the right time to do it?

## Goals & non-goals

### Goals

1. ...
2. ...

### Non-goals

- ...

## Recommended approach

The approach the spec adopts. One paragraph of summary, then any
necessary detail.

### Approaches discarded

- **Approach B — ...** Rejected because ...
- **Approach C — ...** Rejected because ...

## Architecture

Components, their responsibilities, and how they compose. Each component
must be a standalone library with a documented public API (Principle V).

## Data model

If the sub-project introduces or modifies a data model, describe it
here. On-disk formats must be documented (Principle III hard boundary
rule on private formats).

## Interfaces

- **CLI**: commands, flags, output shape, exit codes.
- **Library**: public API surface (function signatures, types).
- **Wire (MCP / HTTP / IPC)**: protocol contracts.

## Error handling

How failures are surfaced. Per Principle VII: errors must report what
operation failed, what was attempted, and what state was preserved or
rolled back. No silent fallbacks.

## Testing strategy

What is tested at which level (unit, integration, end-to-end).
Per Principle VI: tests must pass with networking disabled. Per
Principle III.b: tests must cover every end-to-end memory operation
using only open components.

## Open questions

Items the spec author could not resolve and which the implementation
plan or a brainstorming follow-up must address.

## Acceptance criteria

A numbered, observable, testable list. Each item names a verification
command or measurable outcome. The sub-project is done when all items
are observable on `main`.

## Constitution Check

| Principle | How this design complies |
|---|---|
| **I — Local-First and Sovereign** | ... |
| **II — Provider-Agnostic by Contract** | ... |
| **III — Open Core with a Stable Boundary** | ... |
| **V — Composable Library Architecture** | ... |
| **VI — Deterministic and Offline-Testable** | ... |
| **X — Performance Budgets, Enforced in CI** | ... |

Principles IV (CLI-First), VII (Honest Failure Modes), VIII (Privacy
Telemetry), and IX (Accessible by Default) are re-checked in any
sub-project that touches their surfaces.
