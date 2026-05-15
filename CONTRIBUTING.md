# Contributing to Singularmem

Thank you for considering a contribution. Singularmem is a
constitution-governed project; before opening a non-trivial PR, please
read [`.specify/memory/constitution.md`](.specify/memory/constitution.md).

## Development environment

You will need:

- Rust stable (the toolchain is pinned via `rust-toolchain.toml`; if
  `rustup` is installed, the right channel is selected automatically).
- `cargo fmt`, `cargo clippy`, and `cargo test` (installed with the
  `rustfmt` and `clippy` components, also pinned).
- Git 2.40 or newer.

Quick sanity check:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo build --release
./target/release/singularmem
# → singularmem 0.0.0
```

## Sub-project workflow

Singularmem work is organised as **sub-projects**. Each sub-project
goes through the full cycle:

1. **Brainstorm** — invoke the `brainstorming` skill to refine intent
   and pick an approach.
2. **Spec** — write the design to
   `docs/superpowers/specs/YYYY-MM-DD-<topic>-design.md` following
   [`.specify/templates/spec-template.md`](.specify/templates/spec-template.md).
3. **Plan** — write the implementation plan to
   `docs/superpowers/plans/YYYY-MM-DD-<topic>.md` following
   [`.specify/templates/plan-template.md`](.specify/templates/plan-template.md)
   and using
   [`.specify/templates/tasks-template.md`](.specify/templates/tasks-template.md)
   for task entries.
4. **Tasks** — every task in the plan has an `assigned-skill` field.
5. **Implementation** — execute the plan task by task, committing
   frequently.
6. **Review** — invoke `requesting-code-review` before opening the
   PR.

## Pull request requirements

Every PR must:

- Pass `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-targets`.
- Include a **Constitution Check** in the linked plan addressing
  Principles I, II, III, V, VI, and X (per the constitution's
  governance section).
- Respect the performance budgets in Principle X (sub-project 1 and
  later — bootstrap is exempt).
- Add or update tests where the change touches testable code.
- Have **every commit signed off** (see DCO below).

## Developer Certificate of Origin (DCO), not a CLA

Singularmem uses the
[Developer Certificate of Origin 1.1](https://developercertificate.org/).
There is **no Contributor License Agreement**.

Every commit must include a `Signed-off-by` trailer:

```bash
git commit -s -m "your message"
```

The `-s` flag adds the trailer using `git config user.email`. CI
rejects unsigned commits.

The DCO is sufficient (a CLA is not necessary) because the open-core
model's structural rule — that proprietary code never *ingests* open
contributor code, only *links* to it via the Apache-2.0 library
boundary — means Apache-2.0 already grants the proprietary tier
everything it needs.

## Code of conduct

By participating in this project you agree to abide by the
[Contributor Covenant 2.1](CODE_OF_CONDUCT.md). Report violations to
`security@singularmem.dev`.

## Security

See [SECURITY.md](SECURITY.md) for the coordinated disclosure process.
