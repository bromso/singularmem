//! Single-item verbs: `ingest`, `get`, `list`, `revisions`, `export`, `scope`.

use std::io::{self, Read, Write};

use singularmem_core::{ItemId, NewItem, Store};

use crate::commands::{
    GetArgs, GetFormat, IngestArgs, IngestFormat, ListArgs, ListFormat, RevisionsArgs, ScopeAction,
    ScopeCommand,
};
use crate::CliError;

pub fn cmd_ingest(store: &Store, args: IngestArgs) -> Result<(), CliError> {
    let content = match (args.content, args.file, args.stdin) {
        (Some(s), None, false) => s,
        (None, Some(p), false) => std::fs::read_to_string(&p)?,
        (None, None, true) => {
            let mut s = String::new();
            io::stdin().read_to_string(&mut s)?;
            s
        }
        _ => {
            return Err(CliError::Usage(
                "exactly one of --content, --file, --stdin must be provided".into(),
            ))
        }
    };

    let mut item = NewItem::text(content);
    item.tags = args.tags;
    item.source = args.source;
    item.scope = args.scope;
    if let Some(s) = args.supersedes {
        item.supersedes = Some(s.parse::<ItemId>()?);
    }
    if let Some(meta_text) = args.metadata {
        item.metadata = serde_json::from_str(&meta_text)?;
    }

    let stored = store.ingest(item)?;
    let mut out = io::stdout().lock();
    match args.format {
        IngestFormat::Id => writeln!(out, "{}", stored.id)?,
        IngestFormat::Json => {
            serde_json::to_writer(&mut out, &stored)?;
            writeln!(out)?;
        }
    }
    Ok(())
}

pub fn cmd_get(store: &Store, args: &GetArgs) -> Result<(), CliError> {
    let id = args.id.parse::<ItemId>()?;
    let item = store.get(id)?;
    let mut out = io::stdout().lock();
    match args.format {
        GetFormat::Text => write!(out, "{}", item.content)?,
        GetFormat::Json => {
            serde_json::to_writer(&mut out, &item)?;
            writeln!(out)?;
        }
    }
    Ok(())
}

pub fn cmd_list(store: &Store, args: &ListArgs) -> Result<(), CliError> {
    let tag_refs: Vec<&str> = args.tags.iter().map(String::as_str).collect();
    let filter = args.scope.to_filter()?;
    let iter: Box<dyn Iterator<Item = singularmem_core::Result<singularmem_core::Item>>> =
        Box::new(store.list_by_tags_scoped(&tag_refs, filter.as_ref())?);

    let iter: Box<dyn Iterator<Item = singularmem_core::Result<singularmem_core::Item>>> =
        if let Some(limit) = args.limit {
            Box::new(iter.take(limit))
        } else {
            iter
        };

    let mut out = io::stdout().lock();
    match args.format {
        ListFormat::Ids => {
            for r in iter {
                let item = r?;
                writeln!(out, "{}", item.id)?;
            }
        }
        ListFormat::Jsonl => {
            for r in iter {
                let item = r?;
                serde_json::to_writer(&mut out, &item)?;
                writeln!(out)?;
            }
        }
        ListFormat::Table => {
            // Two columns: ID  CONTENT (truncated to 80 chars).
            for r in iter {
                let item = r?;
                let snippet: String = item.content.chars().take(80).collect();
                writeln!(out, "{}\t{}", item.id, snippet.replace('\n', " "))?;
            }
        }
    }
    Ok(())
}

pub fn cmd_revisions(store: &Store, args: &RevisionsArgs) -> Result<(), CliError> {
    let id = args.id.parse::<ItemId>()?;
    let history = store.revision_history(id)?;
    let mut out = io::stdout().lock();
    for item in history {
        match args.format {
            ListFormat::Ids => writeln!(out, "{}", item.id)?,
            ListFormat::Jsonl => {
                serde_json::to_writer(&mut out, &item)?;
                writeln!(out)?;
            }
            ListFormat::Table => {
                let snippet: String = item.content.chars().take(80).collect();
                writeln!(out, "{}\t{}", item.id, snippet.replace('\n', " "))?;
            }
        }
    }
    Ok(())
}

pub fn cmd_export(store: &Store) -> Result<(), CliError> {
    let mut out = io::stdout().lock();
    store.export(&mut out)?;
    Ok(())
}

pub fn cmd_scope(store: &Store, cmd: &ScopeCommand) -> Result<(), CliError> {
    let mut out = io::stdout().lock();
    match &cmd.action {
        ScopeAction::List => {
            for (path, count) in store.scopes()? {
                writeln!(out, "{path}\t{count}")?;
            }
        }
        ScopeAction::Move { id, path } => {
            let id = id.parse::<ItemId>()?;
            let scope = if path == "-" {
                None
            } else {
                Some(path.as_str())
            };
            let item = store.set_scope(id, scope)?;
            writeln!(out, "{}", item.id)?;
            eprintln!("note: search indexes keep the previous scope until `singularmem reindex`");
        }
    }
    Ok(())
}
