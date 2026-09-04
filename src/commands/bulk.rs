//! Bulk-ingest verbs: `ingest-transcript`, `ingest-codex`, `ingest-cursor`,
//! `ingest-dir`, plus the file-discovery/progress-reporting helpers they
//! share. `codex_root`/`cursor_user_dir` are also read by `commands::hooks`'s
//! own Codex/Cursor ingest paths, mirroring these verbs' defaults.

use std::path::{Path, PathBuf};

use singularmem_core::Store;

use crate::commands::{
    accumulate, canonicalize_project, IngestCodexArgs, IngestCursorArgs, IngestDirArgs,
    IngestTranscriptArgs,
};
use crate::CliError;

/// Ingest each file in `files` by opening it with `open` (which also
/// configures the resulting source), accumulating counts into `total` and
/// unopenable files into `failed_files`. Per-file open failures are warned
/// and counted; a store-level failure returns immediately.
fn ingest_files<S: singularmem_ingest::Source>(
    store: &Store,
    files: &[PathBuf],
    dry_run: bool,
    quiet: bool,
    open: impl Fn(&Path) -> singularmem_ingest::Result<S>,
    total: &mut singularmem_ingest::Report,
    failed_files: &mut usize,
) -> Result<(), CliError> {
    use singularmem_ingest::ingest_source;

    for file in files {
        let src = match open(file) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(path = %file.display(), error = %e, "cannot open source");
                *failed_files += 1;
                continue;
            }
        };
        let r = ingest_source(store, &src, dry_run)?;
        if !quiet {
            eprintln!(
                "{}: +{} ingested, {} skipped",
                file.display(),
                r.ingested,
                r.skipped_existing + r.skipped_filtered
            );
        }
        accumulate(total, r);
    }
    Ok(())
}

/// Resolve the directories/files a bulk-ingest command should scan: `paths`
/// if non-empty, otherwise a single default root. Each root is expanded
/// to files via `discover`; a root that is neither a directory nor a file
/// is `Error::NotFound`.
///
/// `default_root` is a closure rather than an eager `PathBuf` and is called
/// only when `paths` is empty, so a command given explicit paths never needs
/// `HOME` (or whatever the default root depends on) to be resolvable.
fn resolve_ingest_files(
    paths: &[PathBuf],
    default_root: impl FnOnce() -> Result<PathBuf, CliError>,
    discover: impl Fn(&Path) -> singularmem_ingest::Result<Vec<PathBuf>>,
) -> Result<Vec<PathBuf>, CliError> {
    let roots: Vec<PathBuf> = if paths.is_empty() {
        vec![default_root()?]
    } else {
        paths.to_vec()
    };

    let mut files: Vec<PathBuf> = Vec::new();
    for root in &roots {
        if root.is_dir() {
            files.extend(discover(root)?);
        } else if root.is_file() {
            files.push(root.clone());
        } else {
            return Err(singularmem_ingest::Error::NotFound { path: root.clone() }.into());
        }
    }
    Ok(files)
}

pub fn cmd_ingest_transcript(store: &Store, args: &IngestTranscriptArgs) -> Result<(), CliError> {
    use singularmem_ingest::{discover_transcripts, ClaudeTranscript, Report};

    // Validate --scope up front so a typo is a usage error before any
    // parsing or filesystem work happens.
    let scope_override = args
        .scope
        .as_deref()
        .map(singularmem_core::scope::validate)
        .transpose()?;

    let files = resolve_ingest_files(
        &args.paths,
        || {
            Ok(dirs::home_dir()
                .ok_or_else(|| CliError::Usage("cannot determine home directory".into()))?
                .join(".claude")
                .join("projects"))
        },
        |p| discover_transcripts(p),
    )?;

    let project = canonicalize_project(args.project.as_ref());
    let mut total = Report::default();
    let mut failed_files = 0usize;
    // A store-level failure mid-loop still gets a summary for the files
    // already processed before it propagates.
    let outcome = ingest_files(
        store,
        &files,
        args.dry_run,
        args.quiet,
        |file| {
            let mut s = ClaudeTranscript::open(file)?;
            s.include_sidechains = args.include_sidechains;
            s.project_filter.clone_from(&project);
            s.scope_override.clone_from(&scope_override);
            Ok(s)
        },
        &mut total,
        &mut failed_files,
    );
    total.failed += failed_files;
    print_summary(&total, files.len());
    outcome?;
    if total.failed > 0 {
        return Err(CliError::IngestPartial {
            failed: total.failed,
        });
    }
    Ok(())
}

pub fn cmd_ingest_codex(store: &Store, args: &IngestCodexArgs) -> Result<(), CliError> {
    use singularmem_ingest::{discover_codex_sessions, CodexRollout, Report};

    // Validate --scope up front so a typo is a usage error before any
    // parsing or filesystem work happens.
    let scope_override = args
        .scope
        .as_deref()
        .map(singularmem_core::scope::validate)
        .transpose()?;

    let files = resolve_ingest_files(
        &args.paths,
        || codex_root().ok_or_else(|| CliError::Usage("cannot determine home directory".into())),
        |p| discover_codex_sessions(p),
    )?;

    let project = canonicalize_project(args.project.as_ref());
    let mut total = Report::default();
    let mut failed_files = 0usize;
    let outcome = ingest_files(
        store,
        &files,
        args.dry_run,
        args.quiet,
        |file| {
            let mut s = CodexRollout::open(file)?;
            s.project_filter.clone_from(&project);
            s.scope_override.clone_from(&scope_override);
            Ok(s)
        },
        &mut total,
        &mut failed_files,
    );
    total.failed += failed_files;
    print_summary(&total, files.len());
    outcome?;
    if total.failed > 0 {
        return Err(CliError::IngestPartial {
            failed: total.failed,
        });
    }
    Ok(())
}

pub fn cmd_ingest_cursor(store: &Store, args: &IngestCursorArgs) -> Result<(), CliError> {
    use singularmem_ingest::{ingest_source, CursorChats};

    // Validate --scope up front so a typo is a usage error before any
    // parsing or filesystem work happens.
    let scope_override = args
        .scope
        .as_deref()
        .map(singularmem_core::scope::validate)
        .transpose()?;

    let dir = match &args.cursor_dir {
        Some(d) => d.clone(),
        None => cursor_user_dir()
            .ok_or_else(|| CliError::Usage("cannot determine home directory".into()))?,
    };

    let mut src = CursorChats::open(&dir)?;
    src.project_filter = canonicalize_project(args.project.as_ref());
    src.conversation_filter.clone_from(&args.conversation);
    src.scope_override = scope_override;

    let r = ingest_source(store, &src, args.dry_run)?;
    if !args.quiet {
        eprintln!(
            "{}: +{} ingested, {} skipped",
            dir.display(),
            r.ingested,
            r.skipped_existing + r.skipped_filtered
        );
    }
    print_summary(&r, 1);
    if r.failed > 0 {
        return Err(CliError::IngestPartial { failed: r.failed });
    }
    Ok(())
}

pub fn cmd_ingest_dir(store: &Store, args: &IngestDirArgs) -> Result<(), CliError> {
    use singularmem_ingest::{ingest_source, DirectoryWalker};

    // Validate --scope up front so a typo is a usage error before any
    // filesystem walking happens.
    let scope_override = args
        .scope
        .as_deref()
        .map(singularmem_core::scope::validate)
        .transpose()?;

    let mut src = DirectoryWalker::new(&args.path)?;
    src.max_file_bytes = args.max_file_bytes;
    src.scope_override = scope_override;
    let r = ingest_source(store, &src, args.dry_run)?;
    if !args.quiet {
        eprintln!(
            "{}: +{} ingested, {} skipped",
            src.root.display(),
            r.ingested,
            r.skipped_existing + r.skipped_filtered
        );
    }
    print_summary(&r, src.visited_files());
    if r.failed > 0 {
        return Err(CliError::IngestPartial { failed: r.failed });
    }
    Ok(())
}

fn print_summary(r: &singularmem_ingest::Report, files: usize) {
    eprintln!(
        "ingested {}, skipped {} existing, {} filtered, {} failed across {} files",
        r.ingested, r.skipped_existing, r.skipped_filtered, r.failed, files
    );
}

/// Resolve the Codex sessions root: the `SINGULARMEM_CODEX_ROOT` environment
/// variable when set, otherwise `default_codex_root()` (`~/.codex/sessions`).
/// Read by both the `hook`'s Codex fallback scan and `ingest-codex`'s
/// default root, mirroring how `SINGULARMEM_CURSOR_DIR` overrides Cursor's
/// per-user directory for the hook.
pub fn codex_root() -> Option<PathBuf> {
    std::env::var_os("SINGULARMEM_CODEX_ROOT")
        .map(PathBuf::from)
        .or_else(singularmem_ingest::default_codex_root)
}

/// Resolve Cursor's per-user directory: the `SINGULARMEM_CURSOR_DIR`
/// environment variable when set, otherwise `default_cursor_user_dir()`
/// (the per-OS Cursor `User` directory). Read by both the `hook`'s Cursor
/// ingest and `ingest-cursor`'s `--cursor-dir` default, mirroring how
/// `SINGULARMEM_CODEX_ROOT` overrides Codex's sessions root. Documented in
/// `docs/hooks.md`.
pub fn cursor_user_dir() -> Option<PathBuf> {
    std::env::var_os("SINGULARMEM_CURSOR_DIR")
        .map(PathBuf::from)
        .or_else(singularmem_ingest::default_cursor_user_dir)
}
