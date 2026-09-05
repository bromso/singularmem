//! `Store.wakeup` — a project's recent memory across its default scopes,
//! rendered through an adapter under a byte budget.
//!
//! Spec: `docs/superpowers/specs/2026-09-05-mcp-surface-16-design.md`
//! § "Node binding". Delegates straight to
//! `singularmem_retrieve::wakeup::{build, render}` — the same functions the
//! CLI's `wake-up` command and the MCP server's `memory_wakeup` tool call,
//! so the three surfaces produce identical output for identical inputs.
//!
//! # Design note — deferred argument errors
//!
//! Like `graph.rs`, project/adapter resolution happens on the JS thread
//! *before* the task is queued, but a failure is stashed as `pre_error`
//! rather than returned from the method, so `store.wakeup({...})` rejects
//! its Promise instead of throwing synchronously. See `graph.rs`'s module
//! doc for the full rationale.

use std::path::PathBuf;
use std::sync::Arc;

use napi::bindgen_prelude::AsyncTask;
use napi::{Env, Error as NapiError, Task};
use singularmem_core::{Error as CoreError, Store as CoreStore};
use singularmem_retrieve::wakeup::{build, render, ScopeSet, WakeupOptions as CoreWakeupOptions};
use singularmem_retrieve::Adapter;

use crate::error::NodeError;
use crate::graph::reject_coded;
use crate::store::Store;
use crate::types::{Wakeup, WakeupOptions};

const DEFAULT_LIMIT: usize = 20;
const DEFAULT_MAX_BYTES: usize = 8192;

/// Look up a `singularmem_retrieve::Adapter` by its stable name.
///
/// # Errors
/// A `Validation` coded error naming the four known adapters when `name`
/// matches none of them.
fn adapter_by_name(name: &str) -> Result<Box<dyn Adapter>, NapiError<&'static str>> {
    let adapter: Box<dyn Adapter> = match name {
        "plain" => Box::new(singularmem_retrieve::PlainAdapter),
        "claude" => Box::new(singularmem_adapter_claude::ClaudeAdapter),
        "openai" => Box::new(singularmem_adapter_openai::OpenAiAdapter),
        "gemini" => Box::new(singularmem_adapter_gemini::GeminiAdapter),
        other => {
            return Err(NodeError::from(CoreError::Validation {
                field: "adapter",
                reason: format!("unknown adapter {other}; known: plain, claude, openai, gemini"),
            })
            .into());
        }
    };
    Ok(adapter)
}

/// Resolve the project directory: `project` if given, else the process's
/// current directory.
///
/// # Errors
/// A `Validation` coded error naming the path when it is not a directory.
fn resolve_project(project: Option<String>) -> Result<PathBuf, NapiError<&'static str>> {
    let dir = match project {
        Some(p) => PathBuf::from(p),
        None => std::env::current_dir()
            .map_err(|e| -> NapiError<&'static str> { NodeError::from(CoreError::Io(e)).into() })?,
    };
    if !dir.is_dir() {
        return Err(NodeError::from(CoreError::Validation {
            field: "project",
            reason: format!("{} is not a directory", dir.display()),
        })
        .into());
    }
    Ok(dir)
}

// ── WakeupTask ───────────────────────────────────────────────────────────────

pub struct WakeupTask {
    store: Arc<CoreStore>,
    project: PathBuf,
    include_files: bool,
    limit: usize,
    max_bytes: usize,
    adapter: Box<dyn Adapter>,
    pre_error: Option<NapiError<&'static str>>,
    failed: Option<NapiError<&'static str>>,
}

#[napi]
impl Task for WakeupTask {
    type Output = (singularmem_retrieve::wakeup::Wakeup, String);
    type JsValue = Wakeup;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        if self.pre_error.is_some() {
            return Err(NapiError::new(
                napi::Status::GenericFailure,
                "pre-validation failed",
            ));
        }
        let set = ScopeSet::for_project(&self.project, self.include_files);
        let opts = CoreWakeupOptions {
            limit: self.limit,
            max_bytes: self.max_bytes,
        };
        match build(&self.store, &set, &opts) {
            Ok(w) => {
                let text = render(&w, self.adapter.as_ref(), self.max_bytes);
                Ok((w, text))
            }
            Err(e) => {
                self.failed = Some(crate::error::from_retrieve_error(e));
                Err(NapiError::new(
                    napi::Status::GenericFailure,
                    "wakeup failed",
                ))
            }
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        let (w, text) = output;
        Ok(Wakeup {
            text,
            total: u32::try_from(w.total).unwrap_or(u32::MAX),
            shown: u32::try_from(w.shown).unwrap_or(u32::MAX),
            scopes: w.scopes,
        })
    }

    fn reject(&mut self, env: Env, _trigger: NapiError) -> napi::Result<Self::JsValue> {
        Err(reject_coded(
            env,
            self.pre_error.take(),
            self.failed.take(),
            "wakeup",
        ))
    }
}

#[napi]
impl Store {
    /// The project's recent memory across its default scopes, rendered as a
    /// prompt-ready string — the same context the editor hooks inject at
    /// session start.
    ///
    /// Scopes are `claude-code/<basename>`, `codex/<basename>` and
    /// `cursor/<basename>` (plus `files/<basename>` when `includeFiles`),
    /// where `<basename>` is `options.project`'s raw (uncanonicalised)
    /// final path component.
    ///
    /// @param options Project, scope, budget and adapter options (see `WakeupOptions`).
    /// @returns The rendered text plus the counts and scopes behind it.
    /// @throws `{ code: "Validation" }` — `options.project` is missing/not a directory, or `options.adapter` is unknown.
    /// @throws `{ code: "Io" }` — `options.project` is omitted and looking up the current directory fails.
    /// @throws `{ code: "Sqlite" }` — underlying `SQLite` error.
    #[napi]
    #[allow(clippy::missing_errors_doc)]
    pub fn wakeup(&self, options: Option<WakeupOptions>) -> napi::Result<AsyncTask<WakeupTask>> {
        let o = options.unwrap_or_default();
        let mut pre_error = None;
        let project = match resolve_project(o.project) {
            Ok(p) => p,
            Err(e) => {
                pre_error = Some(e);
                PathBuf::new()
            }
        };
        let adapter: Box<dyn Adapter> =
            match adapter_by_name(o.adapter.as_deref().unwrap_or("plain")) {
                Ok(a) => a,
                Err(e) => {
                    // A bad project already wins the rejection; keep the
                    // first error found rather than clobbering it.
                    pre_error.get_or_insert(e);
                    Box::new(singularmem_retrieve::PlainAdapter)
                }
            };
        let limit = o.limit.map_or(DEFAULT_LIMIT, |n| n as usize);
        let max_bytes = o.max_bytes.map_or(DEFAULT_MAX_BYTES, |n| n as usize);
        Ok(AsyncTask::new(WakeupTask {
            store: self.inner.clone(),
            project,
            include_files: o.include_files.unwrap_or(false),
            limit,
            max_bytes,
            adapter,
            pre_error,
            failed: None,
        }))
    }
}
