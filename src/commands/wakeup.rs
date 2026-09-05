//! `wake-up` and the session-start hook-envelope writer it shares with
//! `commands::hooks`'s `hook session-start` (including that hook's
//! store-open-failure fallback, which still must emit a valid envelope).

use std::io::{self, Write};

use singularmem_core::Store;

use crate::commands::search::find_adapter;
use crate::commands::{WakeUpArgs, WakeUpFormat};
use crate::CliError;

pub fn cmd_wake_up(store: &Store, args: &WakeUpArgs) -> Result<(), CliError> {
    use singularmem_retrieve::wakeup::{build, render, ScopeSet, WakeupOptions};

    let set = if args.scope.is_empty() {
        // Pass `--project` through **raw**. The save side derives
        // `<editor>/<basename of the cwd the editor reported>`, so
        // canonicalising here would look up a different scope than the one
        // written for a symlinked project directory. `ScopeSet::for_project`
        // canonicalises internally only when the raw basename is unusable
        // (`.`, `..`), which is what makes `--project .` work.
        let dir = match &args.project {
            Some(p) => p.clone(),
            None => std::env::current_dir()?,
        };
        ScopeSet::for_project(&dir, args.include_files)
    } else {
        ScopeSet(
            args.scope
                .iter()
                .map(|s| singularmem_core::ScopeFilter::descendants(s))
                .collect::<Result<_, _>>()?,
        )
    };
    let adapter = find_adapter(&args.adapter)?;
    let w = build(
        store,
        &set,
        &WakeupOptions {
            limit: args.limit,
            max_bytes: args.max_bytes,
        },
    )?;
    let text = render(&w, &*adapter, args.max_bytes);

    let mut out = io::stdout().lock();
    match args.format {
        WakeUpFormat::Text => write!(out, "{text}")?,
        WakeUpFormat::Json => {
            serde_json::to_writer(
                &mut out,
                &serde_json::json!({
                    "scopes": w.scopes,
                    "total": w.total,
                    "shown": w.shown,
                    "blocks": w.context.blocks,
                    "text": text,
                }),
            )?;
            writeln!(out)?;
        }
        WakeUpFormat::ClaudeHook => {
            write_hook_envelope(&mut out, singularmem_hooks::Editor::ClaudeCode, &text)?;
        }
        WakeUpFormat::CodexHook => {
            write_hook_envelope(&mut out, singularmem_hooks::Editor::Codex, &text)?;
        }
        WakeUpFormat::CursorHook => {
            write_hook_envelope(&mut out, singularmem_hooks::Editor::Cursor, &text)?;
        }
    }
    Ok(())
}

/// Write a session-start hook envelope for `editor` wrapping `text`, followed
/// by a trailing newline.
pub fn write_hook_envelope(
    out: &mut impl Write,
    editor: singularmem_hooks::Editor,
    text: &str,
) -> Result<(), CliError> {
    serde_json::to_writer(
        &mut *out,
        &singularmem_hooks::session_start_envelope(editor, text),
    )?;
    writeln!(out)?;
    Ok(())
}
