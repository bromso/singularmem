# Editor hooks

Singularmem can wire itself into an editor's session lifecycle so that
every session gets ingested automatically (no manual `ingest-*` runs)
and every new session opens with the project's recent context already
loaded (no manual `wake-up` call). This is `singularmem hook` (the
per-event handler an editor invokes) and `singularmem hooks` (the
installer that wires it up).

`claude-code` is Anthropic's Claude Code CLI, and its hook contract is
documented and stable. Codex's and Cursor's hook formats are **not**
publicly documented; the shapes below were reverse-engineered from a
live install of each and may need updating as those editors evolve.
`hooks status` and `RUST_LOG=info` (see [Troubleshooting](#troubleshooting))
are the fastest way to notice drift.

## Install, inspect, remove

```bash
singularmem hooks install claude-code   # ~/.claude/settings.json
singularmem hooks install codex         # ~/.codex/hooks.json
singularmem hooks install cursor        # ~/.cursor/hooks.json

singularmem hooks status                # one line per editor
singularmem hooks status claude-code    # just that editor
singularmem hooks status --project      # reads ./.claude/settings.json etc.

singularmem hooks uninstall claude-code
```

- `--project` writes to (or reads from) `./.claude/settings.json` etc.
  in the current directory instead of the user's home directory —
  useful for a repo-local hook that shouldn't apply to every project.
  `install`, `uninstall`, and `status` all support it.
- `--print` (install only) prints the merged config to stdout instead
  of writing it — useful for reviewing the diff before committing to
  it, or for wiring the config through some other mechanism entirely.
- Install is **idempotent and additive**: it replaces only the hook
  entries Singularmem itself previously wrote (detected structurally,
  by parsing the command string — see `singularmem-hooks`' `config`
  module), leaving every other hook and setting in the file untouched.
  Re-running it after a binary move updates the recorded path.
- Uninstall only rewrites the file when it actually finds one of our
  hook entries to remove; a config with no Singularmem entries (or no
  file at all) is left byte-for-byte untouched.
- `hooks status` never opens the store; it only reads the editor's
  config file. It reports, per editor: whether installed (`installed`,
  `absent`, or `invalid` — see below), the config path, and whether
  the binary path recorded there still exists on disk (`bin ok` /
  `bin missing`).
- A config file that exists but is not valid JSON is never
  overwritten — `install`/`uninstall` fail loudly (exit 1) rather than
  risk destroying whatever the file was supposed to contain. `status`
  instead reports `invalid` for that editor (still exit 0) and prints
  a warning naming the file to stderr, since `status` never writes
  anything and has no destructive action to refuse.

## Per-editor event table

| Editor | Config file | `SessionStart` | `Stop` | `PreCompact` | `SessionEnd` |
|---|---|---|---|---|---|
| `claude-code` | `~/.claude/settings.json` | `SessionStart` (matcher `startup\|resume\|clear\|compact`) | `Stop` (async) | `PreCompact` | `SessionEnd` |
| `codex` | `~/.codex/hooks.json` | `SessionStart` | `Stop` (async) | `PreCompact` | — (not sent) |
| `cursor` | `~/.cursor/hooks.json` | `sessionStart` | `stop` | `preCompact` | `sessionEnd` |

Every hook entry runs `"<path to singularmem>" hook <editor> <event>`
with the editor's JSON payload on stdin. `session-start` prints a
session-start envelope on stdout (the shape each editor expects for
"additional context"); the other three ingest the session's transcript
and print nothing. **A hook always exits 0** — see
[Error handling](#error-handling).

`(async)` on Claude Code's and Codex's `Stop` hook means the editor
fires it and moves on without waiting for it to finish — it never adds
latency to the turn, but it also means the editor's own UI may not
surface anything the hook writes to stderr. If ingest doesn't seem to
be happening, run the same command by hand (or via a small wrapper
that captures stderr) with `RUST_LOG=info`, as described in
[Error handling](#error-handling), rather than looking for the warning
in the editor itself.

## Wiring, in your own words

If you'd rather write the config yourself (or template it out for a
fleet of machines), `hooks install <editor> --print` shows the exact
JSON: for Claude Code and Codex, a `hooks` object keyed by event name,
each holding a list of `{"hooks": [{"type": "command", "command": ...}]}`
groups; for Cursor, `{"version": 1, "hooks": {"sessionStart": [...], ...}}`
with flat `{"command": ..., "timeout": ...}` entries. The commands are
plain shell strings — nothing here depends on `hooks install` having
run.

## `hook <editor> <event>`

Reads one JSON payload from stdin (the exact shape the editor sends
for that event) and either:

- **`session-start`** — resolves the project directory (`cwd` from the
  payload; for Cursor, the first entry of `workspace_roots` when `cwd`
  is absent; otherwise the current directory), builds the wake-up
  context for that project's `claude-code/<project>`, `codex/<project>`,
  and `cursor/<project>` scopes with the `plain` adapter and default
  wake-up options (20 items, 8192-byte budget), and prints it wrapped
  in the editor's `SessionStart` envelope.
- **`stop` / `pre-compact` / `session-end`** — ingests the session
  transcript:
  - **Claude Code** ingests `transcript_path` from the payload.
    Missing it is a warning (nothing to ingest).
  - **Codex** ingests `transcript_path` when the payload has one;
    otherwise it scans the Codex root (`~/.codex/sessions`, or
    `SINGULARMEM_CODEX_ROOT` below) for rollout files whose filename
    contains the payload's `session_id`. A payload with neither
    `transcript_path` nor a non-empty `session_id` is a warning and
    ingests nothing — it never scans the whole root unfiltered.
  - **Cursor** filters by `conversation_id` when the payload has one,
    and by `cwd` as the project whenever the payload has one — both
    together when both are present, since a conversation open in more
    than one window is listed by each of those workspaces and the id
    alone doesn't say which one fired the hook (without the project
    filter the scan also has to open every workspace database to find
    out). A payload with neither is a warning and ingests nothing, for
    the same reason as Codex. See `SINGULARMEM_CURSOR_DIR` below for
    where it looks.

## Environment variables

- **`SINGULARMEM_STORE`** — overrides the store path, same as `--store`
  on any other `singularmem` command. Since an installed hook's command
  line is fixed (no flags), this is the only way to point a hook at a
  non-default store — set it once in your shell profile (or in the
  editor's own environment configuration) if you don't use the default
  per-user XDG data dir.
- **`SINGULARMEM_CURSOR_DIR`** — overrides Cursor's per-user directory
  (which normally defaults per OS: `~/Library/Application
  Support/Cursor/User` on macOS, `%APPDATA%\Cursor\User` on Windows,
  `~/.config/Cursor/User` on Linux). Read by the `hook` command and by
  `ingest-cursor`'s `--cursor-dir` default; an explicit `--cursor-dir`
  still wins over it.
- **`SINGULARMEM_CODEX_ROOT`** — overrides the default Codex sessions
  root (`~/.codex/sessions`). Read by the `hook` command's fallback
  scan (used when the payload has no `transcript_path`) and by
  `ingest-codex`'s default root when no paths are given on the command
  line; `ingest-codex` run with an explicit path argument ignores it.

## Error handling

Hooks must never block the editor they're wired into, so `singularmem
hook` always exits 0 regardless of what happens inside it — parse
failures, missing files, a store that can't be opened, an ingest that
partially fails. Every failure is logged as a `tracing::warn!` to
stderr instead. `session-start` goes one step further: even when the
store can't be opened at all, it still prints a valid session-start
envelope with an empty context string, so the editor never sees a
malformed (or missing) hook response. To see the warnings:

```bash
RUST_LOG=info singularmem hook claude-code stop < payload.json
```

`info` also surfaces the ingest summary (`ingested`, `skipped`,
`failed`) on a successful run. If wake-up context isn't showing up at
session start, or a session doesn't seem to have been ingested, this
is the first thing to check, followed by `singularmem hooks status`
to confirm the hook is actually installed and its binary path still
exists.

**If search misses recent sessions, run `singularmem reindex`.** The
Tantivy sidecar allows a single writer at a time, so two hooks firing
at once — a `Stop` in two editor windows, or a `Stop` racing a manual
`ingest-*` — contend for its write lock. The hook retries the open
with a bounded backoff (five attempts over ~750 ms), which serialises
the common case, but a longer burst can still exhaust it. When that
happens the item is still written to the store (nothing is lost) and
the hook logs `could not open Tantivy index` at `warn` before
continuing without the index hook — the session simply won't appear
in `search` results until the sidecar is rebuilt. `singularmem
reindex` rebuilds it from `SQLite`.
