---
title: Session hooks and wake-up (Sub-project 13)
date: 2026-09-04
status: draft
sub-project: 13-hooks-wakeup
supersedes: none
---

# Session hooks and wake-up (Sub-project 13) — Design Spec

**Date:** 2026-09-04
**Status:** Draft (awaiting user review of written spec)
**Sub-project:** 13 (editor hooks for Claude Code, Codex CLI, Cursor; `wake-up`; Codex and Cursor transcript parsers)
**Builds on:** 11 (transcript ingestion, `singularmem-ingest`), 12 (scoping: `ScopeFilter`, default scopes).
**Constitution:** amended to v0.3.0 in this sub-project (editor integration moved to the open tier).
**Blocks:** 16 (MCP surface: wake-up as an MCP prompt).

## Summary

Closes the "session context" and "auto-save hooks" gaps against mempalace.
Three editors get hooks that save the live transcript on stop and before
context compaction, and inject the project's recent memory at session
start. A `wake-up` command produces that context. The hook logic lives in
the CLI itself (`singularmem hook <editor> <event>` reads the editor's
stdin JSON), so hook config entries are one-liners that the new
`hooks install` verb writes idempotently. Two new transcript sources —
Codex rollout JSONL and Cursor's SQLite chat store — bring those editors'
history into the store; the Cursor parser is something mempalace does not
have.

## Problem & motivation

After 11 and 12 a user must remember to run `ingest-transcript` and
`retrieve --scope`. Mempalace's daily-use value is that nothing has to be
remembered: hooks save, wake-up loads. Without this, the store goes stale
between sessions and the agent starts every session blind.

## Goals & non-goals

### Goals

1. `singularmem hooks install <editor>` wires save + wake-up hooks for
   Claude Code, Codex CLI, and Cursor in one command, reversibly.
2. Every stop / pre-compaction / session-end event ingests the current
   transcript idempotently without blocking the editor.
3. Every session start injects the project's recent memory, bounded in
   size, formatted the way that editor expects.
4. `singularmem ingest-codex` and `singularmem ingest-cursor` bulk-ingest
   those editors' histories with the same flags as `ingest-transcript`.
5. `singularmem wake-up` is usable standalone (text or JSON) for scripts
   and for sub-project 16.

### Non-goals

- Scheduled ingest, filesystem watchers, daemons (remain proprietary).
- Cursor's optional on-disk transcript files (`transcript_path`); the
  SQLite store is the source of truth here.
- Gemini CLI, Antigravity, or other editors.
- MCP / Node exposure of `wake-up` and the new sources (16).
- Summarisation or LLM calls inside wake-up; content is verbatim recent
  items only.

## Recommended approach

The hook is the CLI. `singularmem hook <editor> <event>` parses the
editor's JSON on stdin and dispatches: session start → `wake-up` output
wrapped in the editor's envelope; save events → the matching ingest
source. Config entries call the absolute path of the running binary.
`hooks install` merges those entries into the editor's config file
without touching other keys. Parsers are `Source` implementations in
`singularmem-ingest`, exactly like `ClaudeTranscript`.

### Approaches discarded

- **Shipped shell scripts installed by the CLI (mempalace style).** Logic
  in bash is untested and Unix-only; every editor payload change becomes
  a shell edit.
- **Prompt-based hooks** (ask the agent to summarise and save on stop).
  Spends tokens, captures paraphrase rather than verbatim history, and is
  what mempalace falls back to for Cursor because it has no parser.

## Architecture

```
editor event ──stdin JSON──► singularmem hook <editor> <event>
                                 ├── session-start ──► wake-up::build(ScopeSet) ──► envelope(editor)
                                 └── stop | pre-compact | session-end
                                        ├── claude-code ─► ClaudeTranscript(transcript_path)
                                        ├── codex ───────► CodexRollout(transcript_path | sessions dir ∩ session_id)
                                        └── cursor ──────► CursorChats(conversation_id)
                                                     └──► ingest_source(store, src) (hooks auto-wired)
singularmem hooks install|uninstall|status <editor>  ──► editor config file (merge, atomic write)
singularmem wake-up ──► Store::recent(filter, limit) ──► adapter ──► text | json | <editor>-hook
singularmem ingest-codex / ingest-cursor ──► same sources, bulk, idempotent
```

New module layout: `crates/singularmem-ingest/src/{codex.rs, cursor.rs}`;
`crates/singularmem-wakeup/` is **not** a new crate — wake-up is a
function in `singularmem-retrieve` (`wakeup::build`) because it produces
a `RetrievedContext` for the existing adapters; hook envelopes and config
merging live in a new small crate `singularmem-hooks` (pure functions
over JSON, no I/O except the config file read/write) so they are testable
without the binary.

## Data model

### Codex rollout source (`CodexRollout`)

Files: `~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl` (also
`~/.codex/archived_sessions/`). Line schema (community-documented, not
official — the parser is defensive):

- `{"type":"session_meta","payload":{"id":…,"cwd":…}}` — first line;
  session id and cwd. If absent, session id = file stem, cwd = `None`.
- `{"type":"response_item","payload":{"type":"message","role":"user"|"assistant","content":[{"type":"input_text"|"output_text","text":…}]}}`
  — kept. Text = all `text` fields joined by a blank line.
- `response_item` with `payload.type` in `function_call`,
  `function_call_output`, `reasoning`, and every `event_msg` /
  `turn_context` line — skipped (counted as filtered only for message-like
  lines without text).

Item: `external_id` `codex:<session_id>:<line_no>` (+`#<n>` per chunk);
`source` `codex:<session_id>`; tags `transcript`, `codex`, `role:<role>`;
metadata `session_id`, `line`, `role`, `cwd`, `occurred_at` (line
`timestamp`), `chunk_index`, `chunk_count`; default scope
`codex/<basename of cwd>`; chunking via `chunk_text`.
`discover_codex_sessions(root)` finds `rollout-*.jsonl` recursively,
sorted; `--project` filters on `cwd` like `ClaudeTranscript`.

### Cursor chat source (`CursorChats`)

Cursor user dir: macOS `~/Library/Application Support/Cursor/User`,
Linux `~/.config/Cursor/User`, Windows `%APPDATA%\Cursor\User`;
`--cursor-dir` overrides. Read paths:

1. `workspaceStorage/<hash>/workspace.json` → `folder` (`file://` URI)
   — the project. Workspaces without it are skipped (counted).
2. `workspaceStorage/<hash>/state.vscdb`, table `ItemTable`, key
   `composer.composerData` → JSON `allComposers[] { composerId, createdAt,
   name? }`.
3. `globalStorage/state.vscdb`, table `cursorDiskKV`:
   `composerData:<composerId>` → `{ name, createdAt, lastUpdatedAt,
   fullConversationHeadersOnly[] { bubbleId, type } }` (falls back to
   `conversation[]` for old records); `bubbleId:<composerId>:<bubbleId>`
   → `{ type: 1 (user) | 2 (assistant), text }`.

Databases are opened read-only with SQLite's `immutable=1` URI flag so a
running Cursor instance holding the write lock does not block; if the
open fails, the file is copied to a temp path and opened there. Bubbles
with empty `text` are filtered.

Item: `external_id` `cursor:<composerId>:<bubbleId>` (+`#<n>`); `source`
`cursor:<composerId>`; tags `transcript`, `cursor`, `role:<role>`;
metadata `composer_id`, `bubble_id`, `index` (position in the header
list), `role`, `title` (composer name), `workspace` (folder path),
`composer_created_at` (RFC 3339 from the ms epoch), `chunk_index`,
`chunk_count`; default scope `cursor/<basename of workspace folder>`.
`--project` filters on the workspace folder; the `hook` verb filters on
`conversation_id` (= composerId).

### Wake-up context

`Store::recent(filter: Option<&ScopeFilter>, limit: usize) -> Vec<Item>`
— `ORDER BY created_at DESC LIMIT ?`, then reversed so output reads
oldest → newest. A `ScopeSet` is one or more `ScopeFilter`s OR-ed; the
default set for a project directory with basename `b` is
`[claude-code/b, codex/b, cursor/b, files/b]` (descendants). Items from
`files/*` are included only when `--include-files` is given (defaults to
off: source files are large and rarely the right session context).

Output is a `RetrievedContext` whose blocks carry `score = 0.0`,
`score_kind = Rrf`, and `query = "wake-up:<scopes>"`, so every existing
adapter renders it unchanged. A header line precedes the adapter output
in `text` format: `# Singularmem wake-up — <scopes joined by ", "> — <total>
items, showing last <n>`. The byte budget (`--max-bytes`, default 8192)
drops oldest blocks first until the rendered output fits; the header
always survives. With zero items the output is the header with `0 items`
and no blocks — exit 0.

### Hook envelopes

| Editor | Session-start output | Save events |
|---|---|---|
| Claude Code | `{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"<text>"}}` | `Stop` (async), `PreCompact` (sync, 60 s), `SessionEnd` (sync, 60 s) → ingest `transcript_path` |
| Codex CLI | same shape, `hookEventName: "SessionStart"` | `Stop` (async), `PreCompact` (sync, 60 s) → ingest `transcript_path`, else sessions dir filtered by `session_id` |
| Cursor | `{"additional_context":"<text>"}` | `stop` (sync, 60 s), `preCompact` (sync, 60 s), `sessionEnd` (sync, 60 s) → Cursor source filtered by `conversation_id` |

Project directory for scope derivation: Claude Code and Codex `cwd`;
Cursor `workspace_roots[0]` (all roots are unioned when several).

### Hook config entries

Claude Code `settings.json` (`~/.claude/` by default, `.claude/` with
`--project`):

```json
{ "hooks": {
  "SessionStart": [{ "matcher": "startup|resume|clear|compact",
    "hooks": [{ "type": "command", "command": "<bin> hook claude-code session-start", "timeout": 30 }] }],
  "Stop":       [{ "hooks": [{ "type": "command", "command": "<bin> hook claude-code stop", "async": true }] }],
  "PreCompact": [{ "hooks": [{ "type": "command", "command": "<bin> hook claude-code pre-compact", "timeout": 60 }] }],
  "SessionEnd": [{ "hooks": [{ "type": "command", "command": "<bin> hook claude-code session-end", "timeout": 60 }] }]
} }
```

Codex `~/.codex/hooks.json` (or `<repo>/.codex/hooks.json`): same
structure with `SessionStart`/`Stop`/`PreCompact`, `matcher: "*"`,
`timeout` in seconds, `async: true` on `Stop`. Cursor `~/.cursor/hooks.json`
(or `.cursor/hooks.json`): `{"version":1,"hooks":{"sessionStart":[{"command":"<bin> hook cursor session-start","timeout":30}],"stop":[{"command":"<bin> hook cursor stop","timeout":60,"loop_limit":1}],"preCompact":[…],"sessionEnd":[…]}}`.

`<bin>` is the absolute path of the running executable, quoted. An entry
is "ours" iff its command string contains `singularmem hook `; install
replaces ours and preserves everything else; uninstall removes ours only.
Files are written atomically (write temp, rename) with the original's
indentation width (2 spaces if new). A file that is not valid JSON is an
error, never overwritten.

## Interfaces

### Library

```rust
// singularmem-core
impl Store { pub fn recent(&self, filter: Option<&ScopeFilter>, limit: usize) -> Result<Vec<Item>>; }

// singularmem-ingest
pub struct CodexRollout { pub path: PathBuf, pub project_filter: Option<PathBuf>, pub scope_override: Option<String>, pub chunk_bytes: usize, .. }
impl CodexRollout { pub fn open(path) -> Result<Self>; }  impl Source for CodexRollout {}
pub fn discover_codex_sessions(root: impl AsRef<Path>) -> Result<Vec<PathBuf>>;
pub fn default_codex_root() -> Option<PathBuf>;           // ~/.codex/sessions

pub struct CursorChats { pub user_dir: PathBuf, pub project_filter: Option<PathBuf>, pub conversation_filter: Option<String>, pub scope_override: Option<String>, pub chunk_bytes: usize, .. }
impl CursorChats { pub fn open(user_dir) -> Result<Self>; }  impl Source for CursorChats {}
pub fn default_cursor_user_dir() -> Option<PathBuf>;

// singularmem-retrieve
pub mod wakeup {
    pub struct ScopeSet(pub Vec<ScopeFilter>);
    impl ScopeSet { pub fn for_project(dir: &Path, include_files: bool) -> Self; }
    pub struct WakeupOptions { pub limit: usize /*20*/, pub max_bytes: usize /*8192*/ }
    pub struct Wakeup { pub context: RetrievedContext, pub total: usize, pub scopes: Vec<String> }
    pub fn build(store: &Store, scopes: &ScopeSet, opts: &WakeupOptions) -> Result<Wakeup>;
    pub fn render(w: &Wakeup, adapter: &dyn Adapter, max_bytes: usize) -> String; // header + blocks, budgeted
}

// singularmem-hooks (new crate)
pub enum Editor { ClaudeCode, Codex, Cursor }
pub enum Event { SessionStart, Stop, PreCompact, SessionEnd }
pub struct HookInput { pub cwd: Option<PathBuf>, pub workspace_roots: Vec<PathBuf>, pub transcript_path: Option<PathBuf>, pub session_id: Option<String>, pub conversation_id: Option<String> }
pub fn parse_input(editor: Editor, json: &serde_json::Value) -> HookInput;
pub fn session_start_envelope(editor: Editor, text: &str) -> serde_json::Value;
pub fn entries(editor: Editor, bin: &Path) -> serde_json::Value;          // our hook entries for that editor
pub fn merge(editor: Editor, existing: &serde_json::Value, ours: &serde_json::Value) -> serde_json::Value;
pub fn remove(editor: Editor, existing: &serde_json::Value) -> serde_json::Value;
pub fn config_path(editor: Editor, project: Option<&Path>) -> Result<PathBuf>;
pub fn status(editor: Editor, existing: &serde_json::Value) -> HookStatus; // installed?, bin path exists?
```

`Source` gains nothing new; `default_scope` and `scope_override` follow
the 12 pattern.

### CLI

```
singularmem ingest-codex  [PATH ...] [--project DIR] [--scope PATH] [--dry-run] [--quiet]
singularmem ingest-cursor [--cursor-dir DIR] [--project DIR] [--conversation ID] [--scope PATH] [--dry-run] [--quiet]
singularmem wake-up [--scope PATH ...] [--project DIR] [--include-files] [--limit N] [--max-bytes N]
                    [--adapter plain|claude|openai|gemini] [--format text|json|claude-hook|codex-hook|cursor-hook]
singularmem hook <claude-code|codex|cursor> <session-start|stop|pre-compact|session-end>   # stdin: editor JSON
singularmem hooks install   <editor> [--project] [--print]
singularmem hooks uninstall <editor> [--project]
singularmem hooks status    [<editor>]
```

`--scope` may repeat on `wake-up` (OR-ed); with neither `--scope` nor
`--project`, the project is the current directory. `hook` always exits 0
(errors to stderr); it honours `--store` and `SINGULARMEM_STORE`. The
bulk verbs share `ingest-transcript`'s exit codes and summary line.
`hooks install` prints the config path it wrote; `--print` prints the
merged JSON to stdout and writes nothing. `hooks status` prints one line
per editor: `<editor>\t<installed|absent>\t<config path>\t<bin ok|bin missing>`.

### Wire

None. (16 adds `wake-up` as an MCP prompt.)

## Error handling

- Hook JSON that fails to parse: warn, exit 0. Missing `transcript_path`
  for Claude Code: warn, exit 0. Ingest errors inside a hook: warn with
  the `Report`, exit 0. The editor must never be blocked by memory.
- `wake-up` with a store that cannot be opened: normal CLI error, exit 1
  (`hook session-start` swallows it and emits an empty envelope).
- Cursor DB open failure after the temp-copy fallback: `Error::Io`
  naming the path; in bulk mode that workspace counts as failed and the
  run continues.
- A Codex file without `session_meta`: parsed with fallbacks, warn once.
- `hooks install` on a config file with invalid JSON: error naming the
  file, nothing written. On a file we cannot write: error, exit 1.
- `hooks status` never errors on a missing file (reports `absent`).

## Testing strategy

- **Codex parser:** fixture `tests/fixtures/codex-rollout.jsonl` covering
  `session_meta`, user/assistant `response_item` messages, function
  call/output, reasoning, `event_msg`, a legacy file without
  `session_meta`, a malformed line. Assert kept set, ids, scope default.
- **Cursor parser:** the test builds a miniature Cursor user dir with
  `rusqlite` — one workspace (`workspace.json` + `state.vscdb` with
  `composer.composerData`), a global `state.vscdb` with `composerData:`
  and `bubbleId:` rows using the real key and JSON shapes captured from a
  live install — and asserts items, roles, ordering, scope default,
  `conversation_filter`, and that a workspace without `workspace.json` is
  skipped and counted. `immutable=1` open path exercised; the copy
  fallback covered by pointing at a non-openable path.
- **`Store::recent`:** ordering, limit, scope filter, empty.
- **Wake-up:** scope-set union, `--include-files`, byte budget drops
  oldest first and keeps the header, zero-item output, each `--format`
  envelope is valid JSON with the right field.
- **Hooks crate:** merge/remove are idempotent and preserve foreign
  entries and key order; `status` detection; `config_path` per editor and
  per OS (`HOME` / `APPDATA` from env).
- **CLI:** `hook claude-code stop` with a fixture JSON on stdin ingests
  the fixture transcript and prints nothing on stdout; `hook … session-start`
  prints the envelope; `hooks install claude-code` against `HOME=<tmp>`
  writes the entries, is idempotent, keeps a pre-existing foreign hook,
  and `uninstall` restores the original bytes modulo formatting;
  `ingest-codex` / `ingest-cursor` on fixtures are idempotent.
- All offline, `SINGULARMEM_TEST_EMBEDDER=mock`.

## Open questions

None blocking. Whether `wake-up` should prefer a hand-written project
brief (a pinned item) over recency is left to sub-project 16, which can
add a `wake-up` MCP prompt with that behaviour.

## Acceptance criteria

1. `cargo test --workspace --all-targets` passes offline; clippy/fmt clean.
2. `singularmem hooks install claude-code && singularmem hooks status`
   reports Claude Code installed with `bin ok`; a real Claude Code
   session in this repo then shows the wake-up context at start (manual
   check) and `scope list` grows after the session's first stop.
3. `singularmem hooks install codex` / `cursor` write valid config that
   the respective editor accepts (manual check on this machine for
   Cursor; Codex config validated against its documented schema).
4. `singularmem ingest-cursor --dry-run` on this machine reports items for
   the ~188 workspaces with `0 failed`.
5. `singularmem wake-up --project .` prints a header and the most recent
   items from `claude-code/singularmem`, within 8 KiB.
6. `hooks uninstall` leaves a foreign hook entry byte-identical.
7. The constitution is at v0.3.0 with the Sync Impact Report entry and
   the Open/Closed Split changes; the Constitution Check below passes.

## Constitution Check

| Principle | How this design complies |
|---|---|
| **I — Local-First and Sovereign** | Hooks call a local binary; no network. Cursor/Codex data is read from the user's own machine. |
| **II — Provider-Agnostic by Contract** | Three editors behind one `Editor` enum; envelopes and parsers are per-editor adapters over shared logic. |
| **III — Open Core with a Stable Boundary** | Amendment v0.3.0 moves editor integration to open (III.a: proprietary → open). Scheduled ingest and watchers stay proprietary. |
| **IV — CLI-First, GUI-Visible** | Every capability is a CLI verb; the hook verb is the CLI. |
| **V — Composable Library Architecture** | Parsers in `singularmem-ingest`, wake-up in `singularmem-retrieve`, envelopes/merging in `singularmem-hooks`; `src/main.rs` is dispatch only. |
| **VI — Deterministic and Offline-Testable** | Fixtures for both parsers; an in-test Cursor database; `HOME` redirected for install tests. |
| **VII — Honest Failure Modes** | Hooks never block the editor but always report to stderr; install refuses to overwrite invalid JSON; missing directories are reported. |
| **VIII — Privacy Telemetry Boundary** | No telemetry. Cursor `user_email` in hook payloads is ignored and never stored. |
| **IX — Accessible by Default** | CLI plain text. |
| **X — Performance Budgets, Enforced in CI** | Ingest path unchanged; wake-up is one indexed query. |
