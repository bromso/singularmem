//! `memory_wakeup` tool — the project's recent memory, as the editor
//! hooks inject it at session start.

use std::path::{Path, PathBuf};

use rmcp::model::{Tool, ToolAnnotations};
use serde::Deserialize;
use singularmem_retrieve::wakeup::{build, render, ScopeSet, WakeupOptions};
use singularmem_retrieve::Adapter;

use crate::tools::util::open_store_for_reading;
use crate::{Config, Error, Result};

const DEFAULT_LIMIT: usize = 20;
const DEFAULT_MAX_BYTES: usize = 8192;

/// Arguments for `memory_wakeup`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MemoryWakeupArgs {
    /// Project directory. Defaults to the server's `--project`, then its cwd.
    pub project: Option<String>,
    /// Also read `files/<basename>`. Default false.
    pub include_files: Option<bool>,
    /// Most recent items to consider. Default 20.
    pub limit: Option<usize>,
    /// Output budget in bytes; oldest blocks are dropped first. Default 8192.
    pub max_bytes: Option<usize>,
    /// Adapter name. Defaults to the server's default adapter.
    pub adapter: Option<String>,
}

/// Handler output.
#[derive(Debug, Clone)]
pub struct MemoryWakeupOutput {
    pub text: String,
    pub total: usize,
    pub shown: usize,
    pub scopes: Vec<String>,
}

/// Build the rmcp tool descriptor.
///
/// # Panics
/// Never: the schema literal is an object.
#[must_use]
pub fn tool_descriptor() -> Tool {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "project": { "type": "string", "description": "Project directory. Defaults to the server's --project, then its working directory." },
            "include_files": { "type": "boolean", "default": false, "description": "Also include files/<basename> items (ingest-dir output)." },
            "limit": { "type": "integer", "minimum": 1, "default": 20, "description": "Most recent items to consider." },
            "max_bytes": { "type": "integer", "minimum": 256, "default": 8192, "description": "Output budget; oldest blocks are dropped first." },
            "adapter": { "type": "string", "enum": ["plain", "claude", "openai", "gemini"], "description": "Prompt formatter. Defaults to the server's default adapter." }
        },
        "required": []
    });
    Tool::new(
        "memory_wakeup",
        "Call at the start of a session to load the project's recent memory. \
         Returns the same context the editor hooks inject. \
         Prefer memory_retrieve for a specific question.",
        schema.as_object().expect("schema is object").clone(),
    )
    .annotate(ToolAnnotations::new().read_only(true))
}

/// Resolve the project directory per the spec's order and validate it.
///
/// # Errors
/// [`Error::InvalidProject`] when the path is not an existing directory.
pub fn resolve_project(arg: Option<&str>, config: &Config) -> Result<PathBuf> {
    let dir: PathBuf = match arg {
        Some(p) => PathBuf::from(p),
        None => match &config.project {
            Some(p) => p.clone(),
            None => {
                std::env::current_dir().map_err(|e| Error::InvalidProject(format!("<cwd>: {e}")))?
            }
        },
    };
    if !dir.is_dir() {
        return Err(Error::InvalidProject(dir.display().to_string()));
    }
    Ok(dir)
}

fn find_adapter<'a>(config: &'a Config, name: Option<&str>) -> Result<&'a dyn Adapter> {
    let wanted = name.unwrap_or(&config.default_adapter);
    config
        .known_adapters
        .iter()
        .map(AsRef::as_ref)
        .find(|a| a.name() == wanted)
        .ok_or_else(|| Error::UnknownAdapter(wanted.to_string()))
}

/// Handle `tools/call` for `memory_wakeup`.
///
/// # Errors
/// [`Error::InvalidProject`], [`Error::UnknownAdapter`], or store errors.
pub fn handle_memory_wakeup(
    args: &MemoryWakeupArgs,
    config: &Config,
) -> Result<MemoryWakeupOutput> {
    let dir = resolve_project(args.project.as_deref(), config)?;
    let adapter = find_adapter(config, args.adapter.as_deref())?;
    let set = ScopeSet::for_project(Path::new(&dir), args.include_files.unwrap_or(false));
    let opts = WakeupOptions {
        limit: args.limit.unwrap_or(DEFAULT_LIMIT),
        max_bytes: args.max_bytes.unwrap_or(DEFAULT_MAX_BYTES),
    };
    let store = open_store_for_reading(config)?;
    let w = build(&store, &set, &opts)?;
    let text = render(&w, adapter, opts.max_bytes);
    Ok(MemoryWakeupOutput {
        text,
        total: w.total,
        shown: w.shown,
        scopes: w.scopes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use singularmem_core::{NewItem, Store};
    use tempfile::TempDir;

    /// Store with items under claude-code/proj-a, claude-code/proj-b and
    /// files/proj-a; returns the temp root, a real directory named proj-a
    /// inside it, and a Config.
    fn seeded() -> (TempDir, std::path::PathBuf, Config) {
        let dir = TempDir::new().unwrap();
        let store_path = dir.path().join("store.db");
        let store = Store::open(&store_path).unwrap();
        for (content, scope) in [
            ("alpha decision", "claude-code/proj-a"),
            ("beta decision", "claude-code/proj-a"),
            ("other project note", "claude-code/proj-b"),
            ("readme chunk", "files/proj-a"),
        ] {
            let mut item = NewItem::text(content.to_string());
            item.scope = Some(scope.to_string());
            store.ingest(item).unwrap();
        }
        drop(store);
        let project = dir.path().join("proj-a");
        std::fs::create_dir(&project).unwrap();
        let config = Config::new(store_path, "plain".to_string(), false);
        (dir, project, config)
    }

    #[test]
    fn wakeup_returns_only_the_projects_items() {
        let (_d, project, config) = seeded();
        let args = MemoryWakeupArgs {
            project: Some(project.display().to_string()),
            ..MemoryWakeupArgs::default()
        };
        let out = handle_memory_wakeup(&args, &config).unwrap();
        assert!(out.text.starts_with("# Singularmem wake-up — claude-code/proj-a, codex/proj-a, cursor/proj-a — 2 items, showing last 2"), "{}", out.text);
        assert!(out.text.contains("alpha decision"));
        assert!(out.text.contains("beta decision"));
        assert!(!out.text.contains("other project note"));
        assert!(!out.text.contains("readme chunk"));
    }

    #[test]
    fn include_files_adds_the_files_scope() {
        let (_d, project, config) = seeded();
        let args = MemoryWakeupArgs {
            project: Some(project.display().to_string()),
            include_files: Some(true),
            ..MemoryWakeupArgs::default()
        };
        let out = handle_memory_wakeup(&args, &config).unwrap();
        assert!(out.text.contains("files/proj-a — 3 items"), "{}", out.text);
        assert!(out.text.contains("readme chunk"));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_project_matches_the_same_scopes() {
        let (d, project, config) = seeded();
        let link = d.path().join("link-to-a");
        std::os::unix::fs::symlink(&project, &link).unwrap();
        // The raw basename is what the hooks wrote, so a symlink named
        // differently reads a different scope set — exactly like the CLI.
        let args = MemoryWakeupArgs {
            project: Some(link.display().to_string()),
            ..MemoryWakeupArgs::default()
        };
        let out = handle_memory_wakeup(&args, &config).unwrap();
        assert!(out.text.contains("claude-code/link-to-a"), "{}", out.text);
        assert!(out.text.contains("0 items"), "{}", out.text);
    }

    #[test]
    fn max_bytes_drops_oldest_blocks_and_keeps_header() {
        let (_d, project, config) = seeded();
        let args = MemoryWakeupArgs {
            project: Some(project.display().to_string()),
            max_bytes: Some(256),
            ..MemoryWakeupArgs::default()
        };
        let out = handle_memory_wakeup(&args, &config).unwrap();
        assert!(out.text.starts_with("# Singularmem wake-up"));
        assert!(out.text.len() <= 256, "{}", out.text.len());
    }

    #[test]
    fn config_project_is_the_default() {
        let (_d, project, config) = seeded();
        let config = config.with_project(Some(project));
        let out = handle_memory_wakeup(&MemoryWakeupArgs::default(), &config).unwrap();
        assert!(out.text.contains("claude-code/proj-a"), "{}", out.text);
    }

    #[test]
    fn missing_directory_is_invalid_project() {
        let (_d, _p, config) = seeded();
        let args = MemoryWakeupArgs {
            project: Some("/definitely/not/here".into()),
            ..MemoryWakeupArgs::default()
        };
        let err = handle_memory_wakeup(&args, &config).unwrap_err();
        assert!(matches!(err, Error::InvalidProject(_)), "{err:?}");
    }

    #[test]
    fn unknown_adapter_is_rejected() {
        let (_d, project, config) = seeded();
        let args = MemoryWakeupArgs {
            project: Some(project.display().to_string()),
            adapter: Some("gpt".into()),
            ..MemoryWakeupArgs::default()
        };
        let err = handle_memory_wakeup(&args, &config).unwrap_err();
        assert!(
            matches!(err, Error::UnknownAdapter(ref n) if n == "gpt"),
            "{err:?}"
        );
    }

    #[test]
    fn read_only_server_still_serves_wakeup() {
        let (_d, project, config) = seeded();
        let config = Config::new(config.store_path, "plain".into(), true);
        let args = MemoryWakeupArgs {
            project: Some(project.display().to_string()),
            ..MemoryWakeupArgs::default()
        };
        assert!(handle_memory_wakeup(&args, &config).is_ok());
    }
}
