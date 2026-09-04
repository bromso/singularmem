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

singularmem hooks uninstall claude-code
```

- `--project` writes to (or reads from) `./.claude/settings.json` etc.
  in the current directory instead of the user's home directory —
  useful for a repo-local hook that shouldn't apply to every project.
- `--print` (install only) prints the merged config to stdout instead
  of writing it — useful for reviewing the diff before committing to
  it, or for wiring the config through some other mechanism entirely.
- Install is **idempotent and additive**: it replaces only the hook
  entries Singularmem itself previously wrote (detected structurally,
  by parsing the command string — see `singularmem-hooks`' `config`
  module), leaving every other hook and setting in the file untouched.
  Re-running it after a binary move updates the recorded path.
- `hooks status` never opens the store; it only reads the editor's
  config file. It reports, per editor: whether installed, the config
  path, and whether the binary path recorded there still exists on
  disk (`bin ok` / `bin missing`).
- A config file that exists but is not valid JSON is never
  overwritten — `install`/`uninstall` fail loudly (exit 1) rather than
  risk destroying whatever the file was supposed to contain.

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
    otherwise it scans the default Codex root
    (`~/.codex/sessions`) for rollout files whose filename contains
    the payload's `session_id`. Zero matches is a warning, not a
    failure.
  - **Cursor** filters by `conversation_id` when the payload has one;
    otherwise it filters by `cwd` as the project. See
    `SINGULARMEM_CURSOR_DIR` below for where it looks.

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
  `~/.config/Cursor/User` on Linux). Also accepted by `ingest-cursor
  --cursor-dir`; primarily useful for pointing at a non-standard
  install or a test fixture.

## Error handling

Hooks must never block the editor they're wired into, so `singularmem
hook` always exits 0 regardless of what happens inside it — parse
failures, missing files, a store that can't be opened, an ingest that
partially fails. Every failure is logged as a `tracing::warn!` to
stderr instead. To see them:

```bash
RUST_LOG=info singularmem hook claude-code stop < payload.json
```

`info` also surfaces the ingest summary (`ingested`, `skipped`,
`failed`) on a successful run. If wake-up context isn't showing up at
session start, or a session doesn't seem to have been ingested, this
is the first thing to check, followed by `singularmem hooks status`
to confirm the hook is actually installed and its binary path still
exists.
