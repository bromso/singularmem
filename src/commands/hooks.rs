//! Editor hook entry points (`hook <editor> <event>`, dispatched before the
//! store opens) and `hooks install|uninstall|status`.

use std::io::{self, Read, Write};

use singularmem_core::{Store, StoreOptions};

use crate::commands::bulk::{codex_root, cursor_user_dir};
use crate::commands::index::wire_index_hooks;
use crate::commands::wakeup::write_hook_envelope;
use crate::commands::{accumulate, resolve_store_path, Cli, HookArgs, HooksAction, HooksCommand};
use crate::CliError;

/// Entry point for `hook <editor> <event>`, dispatched from [`crate::run`]
/// *before* the store is opened. A hook must never fail the editor invoking
/// it, so this never returns an error: every failure — an unknown
/// editor/event, a store that cannot be opened, an ingest failure — is
/// logged as a warning on stderr and swallowed.
///
/// `session-start` still needs a valid envelope on stdout even when the
/// store cannot be opened, so it is special-cased to always print one (with
/// an empty context string on failure) rather than simply warning and doing
/// nothing.
#[allow(
    clippy::unnecessary_wraps,
    reason = "signature matches every other cmd_* dispatched from run_command"
)]
pub fn cmd_hook_entry(cli: &Cli, args: &HookArgs) -> Result<(), CliError> {
    use singularmem_hooks::{parse_input, Editor, Event};

    let Ok(editor) = args.editor.parse::<Editor>() else {
        tracing::warn!(editor = %args.editor, "unknown editor; hook does nothing");
        return Ok(());
    };
    let Ok(event) = args.event.parse::<Event>() else {
        tracing::warn!(event = %args.event, "unknown event; hook does nothing");
        return Ok(());
    };

    let mut raw = String::new();
    if let Err(e) = io::stdin().read_to_string(&mut raw) {
        tracing::warn!(error = %e, "could not read hook input from stdin");
        raw.clear();
    }
    let json: serde_json::Value = serde_json::from_str(&raw).unwrap_or_else(|e| {
        tracing::warn!(error = %e, "could not parse hook input JSON; proceeding with empty input");
        serde_json::Value::Null
    });
    let input = parse_input(editor, &json);

    let store_path = resolve_store_path(cli);
    let store_result = Store::open_with_options(
        &store_path,
        StoreOptions {
            read_only: cli.read_only,
        },
    );

    match event {
        Event::SessionStart => match store_result {
            Ok(store) => {
                if let Err(e) = hook_session_start(&store, editor, &input) {
                    tracing::warn!(error = %e, editor = %args.editor, event = %args.event, "hook failed; editor continues");
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    path = %store_path.display(),
                    "could not open store; emitting empty session-start context"
                );
                let mut out = io::stdout().lock();
                if let Err(e2) = write_hook_envelope(&mut out, editor, "") {
                    tracing::warn!(error = %e2, "failed to emit session-start envelope");
                }
            }
        },
        Event::Stop | Event::PreCompact | Event::SessionEnd => match store_result {
            Ok(mut store) => {
                // session-start is read-only; save events are the only ones
                // that write, so only they need the index hooks wired up.
                wire_index_hooks(&mut store, &store_path, cli.no_index);
                if let Err(e) = hook_save_event(&store, editor, &input) {
                    tracing::warn!(error = %e, editor = %args.editor, event = %args.event, "hook failed; editor continues");
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    path = %store_path.display(),
                    editor = %args.editor,
                    event = %args.event,
                    "could not open store; nothing ingested"
                );
            }
        },
    }
    Ok(())
}

/// The `SessionStart` hook: build the project's wake-up context (scoped to
/// `input.cwd`, or the current directory when the editor sends none) and
/// print it wrapped in the editor's session-start envelope.
///
/// Cursor is the one editor that can have several workspace roots open in
/// the same window, and it reports all of them; the spec calls for the union
/// of every root's scopes there, rather than just the first one's (which is
/// all `input.cwd` carries). Duplicate scope paths — two roots with the same
/// basename — are collapsed so the scope set stays a set.
fn hook_session_start(
    store: &Store,
    editor: singularmem_hooks::Editor,
    input: &singularmem_hooks::HookInput,
) -> Result<(), CliError> {
    use singularmem_hooks::session_start_envelope;
    use singularmem_retrieve::wakeup::{build, render, ScopeSet, WakeupOptions};

    let set =
        if matches!(editor, singularmem_hooks::Editor::Cursor) && input.workspace_roots.len() > 1 {
            let mut seen = std::collections::HashSet::new();
            let mut filters = Vec::new();
            for root in &input.workspace_roots {
                for f in ScopeSet::for_project(root, false).0 {
                    if seen.insert(f.path.clone()) {
                        filters.push(f);
                    }
                }
            }
            ScopeSet(filters)
        } else {
            let dir = match &input.cwd {
                Some(p) => p.clone(),
                None => std::env::current_dir()?,
            };
            ScopeSet::for_project(&dir, false)
        };
    let opts = WakeupOptions::default();
    let text = match build(store, &set, &opts) {
        Ok(w) => render(&w, &singularmem_retrieve::PlainAdapter, opts.max_bytes),
        Err(e) => {
            tracing::warn!(error = %e, "wake-up failed; emitting empty context");
            String::new()
        }
    };
    let mut out = io::stdout().lock();
    serde_json::to_writer(&mut out, &session_start_envelope(editor, &text))?;
    writeln!(out)?;
    Ok(())
}

/// The `Stop`/`PreCompact`/`SessionEnd` hooks: ingest whatever transcript(s)
/// the event identifies, per editor.
fn hook_save_event(
    store: &Store,
    editor: singularmem_hooks::Editor,
    input: &singularmem_hooks::HookInput,
) -> Result<(), CliError> {
    use singularmem_hooks::Editor;

    let report = match editor {
        Editor::ClaudeCode => hook_ingest_claude(store, input)?,
        Editor::Codex => hook_ingest_codex(store, input)?,
        Editor::Cursor => hook_ingest_cursor(store, input)?,
    };
    tracing::info!(
        ingested = report.ingested,
        skipped = report.skipped_existing,
        failed = report.failed,
        "hook ingest complete"
    );
    Ok(())
}

/// Claude Code always sends `transcript_path` on `Stop`/`PreCompact`/
/// `SessionEnd`; without it there is nothing to ingest.
fn hook_ingest_claude(
    store: &Store,
    input: &singularmem_hooks::HookInput,
) -> Result<singularmem_ingest::Report, CliError> {
    use singularmem_ingest::{ingest_source, ClaudeTranscript};

    let Some(p) = &input.transcript_path else {
        return Err(CliError::Usage("hook input has no transcript_path".into()));
    };
    let src = ClaudeTranscript::open(p)?;
    Ok(ingest_source(store, &src, false)?)
}

/// Codex sends `transcript_path` when it has one; otherwise scan the Codex
/// root (see [`codex_root`]) for rollout files whose filename contains
/// `session_id`. Without a `transcript_path` *or* a non-empty `session_id`
/// there is nothing to scope the scan to, so nothing is ingested rather than
/// scanning the whole root with an empty filter (which would match every
/// session).
fn hook_ingest_codex(
    store: &Store,
    input: &singularmem_hooks::HookInput,
) -> Result<singularmem_ingest::Report, CliError> {
    use singularmem_ingest::{discover_codex_sessions, ingest_source, CodexRollout, Report};
    use std::path::PathBuf;

    let files: Vec<PathBuf> = if let Some(p) = &input.transcript_path {
        vec![p.clone()]
    } else {
        let want = input.session_id.clone().unwrap_or_default();
        if want.is_empty() {
            tracing::warn!(
                "codex hook payload has no transcript_path or session_id; nothing ingested"
            );
            return Ok(Report::default());
        }
        let root = codex_root()
            .ok_or_else(|| CliError::Usage("cannot determine home directory".into()))?;
        discover_codex_sessions(&root)?
            .into_iter()
            .filter(|f| f.to_string_lossy().contains(&want))
            .collect()
    };

    let mut total = Report::default();
    for f in files {
        let src = CodexRollout::open(&f)?;
        accumulate(&mut total, ingest_source(store, &src, false)?);
    }
    Ok(total)
}

/// Cursor identifies the conversation to ingest by `conversation_id` when
/// present; without one it falls back to filtering by `cwd` as the project.
/// Without either, there is nothing to scope the ingest to, so nothing is
/// ingested rather than pulling in every conversation.
///
/// `cwd` is applied as a project filter **whenever it is present**, even
/// alongside a `conversation_id`: a conversation open in more than one
/// window is listed by every one of those workspaces, so the id alone does
/// not say which workspace this hook fired from — and without a project
/// filter the scan has to open every workspace database (hundreds, on a
/// real install) to find out.
fn hook_ingest_cursor(
    store: &Store,
    input: &singularmem_hooks::HookInput,
) -> Result<singularmem_ingest::Report, CliError> {
    use singularmem_ingest::{ingest_source, CursorChats, Report};

    if input.conversation_id.is_none() && input.cwd.is_none() {
        tracing::warn!("cursor hook payload has no conversation_id or cwd; nothing ingested");
        return Ok(Report::default());
    }

    let user = cursor_user_dir()
        .ok_or_else(|| CliError::Usage("cannot determine Cursor user directory".into()))?;
    let mut src = CursorChats::open(&user)?;
    src.conversation_filter.clone_from(&input.conversation_id);
    src.project_filter.clone_from(&input.cwd);
    let report = ingest_source(store, &src, false)?;

    // A known `conversation_id` whose project-filtered scan turned up
    // nothing at all (not even an existing item skipped as a duplicate)
    // means `cwd` named the wrong workspace — e.g. the payload only carried
    // `workspace_roots`, whose first entry became `cwd` (see
    // `singularmem_hooks::input::parse_input`), but the conversation lives
    // under a different one of those roots. Retry once across every
    // workspace rather than losing the transcript.
    if input.conversation_id.is_some() && report.ingested == 0 && report.skipped_existing == 0 {
        tracing::warn!("cursor conversation not found under cwd; retrying across all workspaces");
        let mut retry = CursorChats::open(&user)?;
        retry.conversation_filter.clone_from(&input.conversation_id);
        return Ok(ingest_source(store, &retry, false)?);
    }

    Ok(report)
}

/// `hooks install|uninstall|status`. Never opens the store — see the
/// dispatch in [`crate::run`].
pub fn cmd_hooks(cmd: &HooksCommand) -> Result<(), CliError> {
    use singularmem_hooks::{
        config_path, merge, read_config, remove, status, write_config, Editor,
    };

    let bin = std::env::current_exe()?;
    let project_dir = std::env::current_dir()?;
    let parse = |s: &str| {
        s.parse::<Editor>().map_err(|_| {
            CliError::Usage(format!(
                "unknown editor '{s}'; expected claude-code, codex, or cursor"
            ))
        })
    };
    let mut out = io::stdout().lock();

    match &cmd.action {
        HooksAction::Install {
            editor,
            project,
            print,
        } => {
            let editor = parse(editor)?;
            let path = config_path(editor, project.then_some(project_dir.as_path()))?;
            let existing = read_config(&path)?;
            let merged = merge(editor, &existing, &bin);
            if *print {
                serde_json::to_writer_pretty(&mut out, &merged)?;
                writeln!(out)?;
            } else {
                write_config(&path, &merged)?;
                writeln!(out, "{}", path.display())?;
            }
        }
        HooksAction::Uninstall { editor, project } => {
            let editor = parse(editor)?;
            let path = config_path(editor, project.then_some(project_dir.as_path()))?;
            let existing = read_config(&path)?;
            // Only rewrite the file when we actually have something to
            // remove — an uninstall over a config that never had our hooks
            // (foreign-only, or simply absent) must leave it byte-for-byte
            // untouched rather than reformatting it.
            if status(editor, &existing).installed {
                write_config(&path, &remove(editor, &existing))?;
            }
            writeln!(out, "{}", path.display())?;
        }
        HooksAction::Status { editor, project } => {
            let editors: Vec<Editor> = match editor {
                Some(e) => vec![parse(e)?],
                None => vec![Editor::ClaudeCode, Editor::Codex, Editor::Cursor],
            };
            for e in editors {
                let path = config_path(e, project.then_some(project_dir.as_path()))?;
                match read_config(&path) {
                    Ok(cfg) => {
                        let s = status(e, &cfg);
                        writeln!(
                            out,
                            "{e}\t{}\t{}\t{}",
                            if s.installed { "installed" } else { "absent" },
                            path.display(),
                            if s.installed && s.bin_exists {
                                "bin ok"
                            } else if s.installed {
                                "bin missing"
                            } else {
                                "-"
                            }
                        )?;
                    }
                    Err(err) => {
                        eprintln!("warning: {err}");
                        writeln!(out, "{e}\tinvalid\t{}\t-", path.display())?;
                    }
                }
            }
        }
    }
    Ok(())
}
