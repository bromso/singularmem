# Security Policy

## Reporting a vulnerability

Email `security@singularmem.dev` with a description of the
vulnerability and (if possible) a reproduction. Please do **not** file
a public GitHub issue for security reports.

If you prefer, GitHub Security Advisories' private vulnerability
reporting flow is also accepted; open a draft advisory via
`Security → Advisories → Report a vulnerability` on the repository.

## Scope

This policy covers the **open-source components** of Singularmem:
the memory engine, on-disk format, indexes, embedding pipeline, LLM
provider adapters, CLI, MCP server, and library SDK.

The proprietary components (desktop GUI, premium visualisations,
sync) are not part of this open-source distribution and are not
covered here.

## Response SLA

- **Acknowledgement:** within 7 calendar days of receiving the
  report.
- **Triage and fix-or-coordinate-disclosure:** within 90 calendar
  days of acknowledgement.

We will keep reporters informed throughout. If we cannot meet either
deadline, we will say so explicitly and propose a new one.

## What we ask of reporters

- Give us a reasonable opportunity to fix the issue before any public
  disclosure.
- Do not exploit the vulnerability beyond what is necessary to
  demonstrate it.
- Do not attempt to access data you do not own.

## What we will not do

- We will not pursue legal action against good-faith security
  researchers who follow this policy.
- We will not silently fix issues without crediting reporters who
  request acknowledgement.

## PGP

A PGP key for `security@singularmem.dev` is not yet published. This
is tracked as a follow-up; for the moment, transport-layer security
(TLS to the receiving mail server) is the assumed protection.
