//! Knowledge-graph verbs: `graph add/query/predicate/invalidate/supersede/
//! timeline/stats/entities/history`, thin CLI wrappers over
//! `singularmem_core::graph`.

use std::io::{self, Write};

use clap::{Args, Subcommand, ValueEnum};
use singularmem_core::graph::time::parse_point;
use singularmem_core::graph::{Direction, Fact, GraphQuery, NewFact, NewObject};
use singularmem_core::{Error, FactId, ItemId, Store};

use crate::commands::ScopeArgs;
use crate::CliError;

#[derive(Args, Debug)]
pub struct GraphCommand {
    #[command(subcommand)]
    pub action: GraphAction,
}

#[derive(Subcommand, Debug)]
pub enum GraphAction {
    /// Record a fact: SUBJECT PREDICATE OBJECT (entities are created on demand).
    Add {
        /// The fact's subject entity name.
        subject: String,
        /// Normalised predicate, e.g. `uses`.
        predicate: String,
        /// The fact's object: an entity name, or a literal value with `--value`.
        object: String,
        /// Treat OBJECT as a literal value, not an entity.
        #[arg(long)]
        value: bool,
        /// Kind to set on SUBJECT if it is newly created by this call.
        #[arg(long)]
        subject_kind: Option<String>,
        /// Kind to set on OBJECT if it is newly created by this call (ignored with `--value`).
        #[arg(long)]
        object_kind: Option<String>,
        /// Start of the validity window (`YYYY-MM-DD` or RFC 3339); omitted means "since unknown".
        #[arg(long, value_name = "TS")]
        from: Option<String>,
        /// End of the validity window (`YYYY-MM-DD` or RFC 3339); omitted means "still valid".
        #[arg(long, value_name = "TS")]
        to: Option<String>,
        /// Confidence in the fact, in the range 0..1.
        #[arg(long, default_value_t = 1.0)]
        confidence: f32,
        /// Item ID this fact was extracted from.
        #[arg(long, value_name = "ITEM_ID")]
        source: Option<String>,
        /// Scope of the fact.
        #[arg(long, value_name = "PATH")]
        scope: Option<String>,
        /// Print the full Fact as JSON instead of just its id.
        #[arg(long)]
        json: bool,
    },
    /// Facts about an entity.
    Query {
        /// Entity name to look up facts for.
        entity: String,
        /// Which side of the fact ENTITY must match.
        #[arg(long, value_enum, default_value_t = DirectionArg::Both)]
        direction: DirectionArg,
        /// Facts valid at this time (`YYYY-MM-DD` or RFC 3339).
        #[arg(long, value_name = "TS")]
        as_of: Option<String>,
        /// What the store believed at this time (`YYYY-MM-DD` or RFC 3339).
        #[arg(long, value_name = "TS")]
        recorded_at: Option<String>,
        #[command(flatten)]
        scope: ScopeArgs,
        /// Also print each fact's source item's first line (ignored with `--json`).
        #[arg(long)]
        with_sources: bool,
        /// Print facts as a JSON array instead of one line each.
        #[arg(long)]
        json: bool,
    },
    /// Facts with a predicate.
    Predicate {
        /// Predicate to look up facts for.
        predicate: String,
        /// Facts valid at this time (`YYYY-MM-DD` or RFC 3339).
        #[arg(long, value_name = "TS")]
        as_of: Option<String>,
        /// What the store believed at this time (`YYYY-MM-DD` or RFC 3339).
        #[arg(long, value_name = "TS")]
        recorded_at: Option<String>,
        #[command(flatten)]
        scope: ScopeArgs,
        /// Print facts as a JSON array instead of one line each.
        #[arg(long)]
        json: bool,
    },
    /// Close an open fact (append-only: writes a new revision).
    Invalidate {
        /// Subject of the fact to close.
        subject: String,
        /// Predicate of the fact to close.
        predicate: String,
        /// Object of the fact to close: an entity name, or a literal value with `--value`.
        object: String,
        /// Treat OBJECT as a literal value, not an entity.
        #[arg(long)]
        value: bool,
        /// When to close the fact (`YYYY-MM-DD` or RFC 3339); defaults to now.
        #[arg(long, value_name = "TS")]
        at: Option<String>,
        /// Scope of the fact.
        #[arg(long, value_name = "PATH")]
        scope: Option<String>,
    },
    /// Replace OLD with NEW at one instant, in one transaction.
    Supersede {
        /// Subject of the fact being replaced.
        subject: String,
        /// Predicate of the fact being replaced.
        predicate: String,
        /// Current object to close: an entity name, or a literal value with `--value`.
        old: String,
        /// New object to open in its place: an entity name, or a literal value with `--value`.
        new: String,
        /// Treat OLD and NEW as literal values, not entities.
        #[arg(long)]
        value: bool,
        /// When to switch from OLD to NEW (`YYYY-MM-DD` or RFC 3339); defaults to now.
        #[arg(long, value_name = "TS")]
        at: Option<String>,
        /// Scope of the fact.
        #[arg(long, value_name = "PATH")]
        scope: Option<String>,
    },
    /// Chronological facts, optionally for one entity.
    Timeline {
        /// Restrict to this entity's facts; every entity when omitted.
        entity: Option<String>,
        #[command(flatten)]
        scope: ScopeArgs,
        /// Print entries as a JSON array instead of one line each.
        #[arg(long)]
        json: bool,
    },
    /// Counts.
    Stats {
        #[command(flatten)]
        scope: ScopeArgs,
        /// Print counts as JSON instead of plain text.
        #[arg(long)]
        json: bool,
    },
    /// Entities with fact counts.
    Entities {
        /// Restrict to entities of this kind.
        #[arg(long)]
        kind: Option<String>,
        #[command(flatten)]
        scope: ScopeArgs,
        /// Print entities as a JSON array instead of one line each.
        #[arg(long)]
        json: bool,
    },
    /// All revisions of a fact, oldest first.
    History {
        /// Fact ID (26-char ULID) whose revision chain to print.
        fact_id: String,
        /// Output shape: table, ids, or json.
        #[arg(long, value_enum, default_value_t = HistoryFormat::Table, conflicts_with = "json")]
        format: HistoryFormat,
        /// Shortcut for `--format json`; conflicts with `--format ids`/`--format table`.
        #[arg(long, conflicts_with = "format")]
        json: bool,
    },
}

/// Which side of a fact `graph query`'s `--direction` should match.
#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum DirectionArg {
    Outgoing,
    Incoming,
    Both,
}

impl From<DirectionArg> for Direction {
    fn from(d: DirectionArg) -> Self {
        match d {
            DirectionArg::Outgoing => Self::Outgoing,
            DirectionArg::Incoming => Self::Incoming,
            DirectionArg::Both => Self::Both,
        }
    }
}

/// Output shape for `graph history`.
#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum HistoryFormat {
    Table,
    Ids,
    Json,
}

/// Dispatch one `graph` subcommand. Each arm hands the whole (still-owned)
/// [`GraphAction`] to its handler, which destructures its own variant —
/// keeping this function short and every handler's argument list to one.
pub fn cmd_graph(store: &Store, cmd: GraphCommand) -> Result<(), CliError> {
    match cmd.action {
        action @ GraphAction::Add { .. } => cmd_add(store, action),
        action @ GraphAction::Query { .. } => cmd_query(store, action),
        action @ GraphAction::Predicate { .. } => cmd_predicate(store, action),
        action @ GraphAction::Invalidate { .. } => cmd_invalidate(store, action),
        action @ GraphAction::Supersede { .. } => cmd_supersede(store, action),
        action @ GraphAction::Timeline { .. } => cmd_timeline(store, action),
        action @ GraphAction::Stats { .. } => cmd_stats(store, action),
        action @ GraphAction::Entities { .. } => cmd_entities(store, action),
        action @ GraphAction::History { .. } => cmd_history(store, action),
    }
}

/// Build a fact's object side: a literal value with `--value`, otherwise an
/// entity (created on demand by the store).
fn object_arg(value: bool, name: String, kind: Option<String>) -> NewObject {
    if value {
        NewObject::Value(name)
    } else {
        NewObject::Entity { name, kind }
    }
}

/// A [`GraphQuery`] carrying only the two time axes and a scope filter —
/// what `cmd_query` (which overwrites `direction` afterwards) and
/// `cmd_predicate` (which leaves it at the default, `Both`) both build from.
fn window_query(
    as_of: Option<&str>,
    recorded_at: Option<&str>,
    scope: &ScopeArgs,
) -> Result<GraphQuery, CliError> {
    Ok(GraphQuery {
        scope: scope.to_filter()?,
        as_of: as_of
            .map(|s| point("--as-of", s, parse_point))
            .transpose()?,
        recorded_at: recorded_at
            .map(|s| point("--recorded-at", s, parse_point))
            .transpose()?,
        direction: Direction::default(),
    })
}

/// Parse a `--flag`'s raw value via `parse`, naming `flag` in the error so
/// e.g. `graph add a p b --from notadate` says which flag was bad instead of
/// the bare "validation failed for timestamp: …".
///
/// Generic over the parsed type (rather than naming `jiff::Timestamp`
/// directly) because `jiff` is not a direct dependency of this crate; every
/// call site passes [`parse_point`], which fixes `T` to `jiff::Timestamp` by
/// inference.
fn point<T>(
    flag: &str,
    raw: &str,
    parse: impl FnOnce(&str) -> Result<T, Error>,
) -> Result<T, CliError> {
    parse(raw).map_err(|e| {
        let reason = match e {
            Error::Validation { reason, .. } => reason,
            other => other.to_string(),
        };
        CliError::Usage(format!("{flag}: {reason}"))
    })
}

fn cmd_add(store: &Store, action: GraphAction) -> Result<(), CliError> {
    let GraphAction::Add {
        subject,
        predicate,
        object,
        value,
        subject_kind,
        object_kind,
        from,
        to,
        confidence,
        source,
        scope,
        json,
    } = action
    else {
        unreachable!("cmd_add only ever receives GraphAction::Add")
    };
    let object = object_arg(value, object, object_kind);
    let valid_from = from
        .as_deref()
        .map(|s| point("--from", s, parse_point))
        .transpose()?;
    let valid_to = to
        .as_deref()
        .map(|s| point("--to", s, parse_point))
        .transpose()?;
    let source_item_id = source.map(|s| s.parse::<ItemId>()).transpose()?;

    let fact = store.add_fact(NewFact {
        subject,
        subject_kind,
        predicate,
        object,
        valid_from,
        valid_to,
        confidence,
        source_item_id,
        scope,
    })?;

    let mut out = io::stdout().lock();
    if json {
        serde_json::to_writer(&mut out, &fact)?;
        writeln!(out)?;
    } else {
        writeln!(out, "{}", fact.id)?;
    }
    Ok(())
}

fn cmd_query(store: &Store, action: GraphAction) -> Result<(), CliError> {
    let GraphAction::Query {
        entity,
        direction,
        as_of,
        recorded_at,
        scope,
        with_sources,
        json,
    } = action
    else {
        unreachable!("cmd_query only ever receives GraphAction::Query")
    };
    let mut query = window_query(as_of.as_deref(), recorded_at.as_deref(), &scope)?;
    query.direction = direction.into();
    let facts = store.query_entity(&entity, &query)?;
    render_facts(store, &facts, with_sources, json)
}

fn cmd_predicate(store: &Store, action: GraphAction) -> Result<(), CliError> {
    let GraphAction::Predicate {
        predicate,
        as_of,
        recorded_at,
        scope,
        json,
    } = action
    else {
        unreachable!("cmd_predicate only ever receives GraphAction::Predicate")
    };
    let query = window_query(as_of.as_deref(), recorded_at.as_deref(), &scope)?;
    let facts = store.query_predicate(&predicate, &query)?;
    render_facts(store, &facts, false, json)
}

fn cmd_invalidate(store: &Store, action: GraphAction) -> Result<(), CliError> {
    let GraphAction::Invalidate {
        subject,
        predicate,
        object,
        value,
        at,
        scope,
    } = action
    else {
        unreachable!("cmd_invalidate only ever receives GraphAction::Invalidate")
    };
    let object = object_arg(value, object, None);
    let at = at
        .as_deref()
        .map(|s| point("--at", s, parse_point))
        .transpose()?;
    let closed = store.invalidate_fact(&subject, &predicate, &object, scope.as_deref(), at)?;
    let mut out = io::stdout().lock();
    writeln!(out, "{}", render_fact(&closed))?;
    Ok(())
}

fn cmd_supersede(store: &Store, action: GraphAction) -> Result<(), CliError> {
    let GraphAction::Supersede {
        subject,
        predicate,
        old,
        new,
        value,
        at,
        scope,
    } = action
    else {
        unreachable!("cmd_supersede only ever receives GraphAction::Supersede")
    };
    let old = object_arg(value, old, None);
    let new = object_arg(value, new, None);
    let at = at
        .as_deref()
        .map(|s| point("--at", s, parse_point))
        .transpose()?;
    let (closed, replacement) =
        store.supersede_fact(&subject, &predicate, &old, new, scope.as_deref(), at)?;

    let mut out = io::stdout().lock();
    if let Some(closed) = closed {
        writeln!(out, "{}", render_fact(&closed))?;
    }
    writeln!(out, "{}", render_fact(&replacement))?;
    Ok(())
}

fn cmd_timeline(store: &Store, action: GraphAction) -> Result<(), CliError> {
    let GraphAction::Timeline {
        entity,
        scope,
        json,
    } = action
    else {
        unreachable!("cmd_timeline only ever receives GraphAction::Timeline")
    };
    let filter = scope.to_filter()?;
    let entries = store.timeline(entity.as_deref(), filter.as_ref())?;

    let mut out = io::stdout().lock();
    if json {
        serde_json::to_writer(&mut out, &entries)?;
        writeln!(out)?;
        return Ok(());
    }
    for entry in &entries {
        let marker = if entry.current {
            "[current]"
        } else {
            "[closed]"
        };
        writeln!(out, "{marker} {}", render_fact(&entry.fact))?;
    }
    Ok(())
}

fn cmd_stats(store: &Store, action: GraphAction) -> Result<(), CliError> {
    let GraphAction::Stats { scope, json } = action else {
        unreachable!("cmd_stats only ever receives GraphAction::Stats")
    };
    let filter = scope.to_filter()?;
    let stats = store.graph_stats(filter.as_ref())?;

    let mut out = io::stdout().lock();
    if json {
        serde_json::to_writer(&mut out, &stats)?;
        writeln!(out)?;
    } else {
        writeln!(out, "entities: {}", stats.entities)?;
        writeln!(out, "open facts: {}", stats.open_facts)?;
        writeln!(out, "closed facts: {}", stats.closed_facts)?;
        writeln!(out, "predicates: {}", stats.predicates)?;
    }
    Ok(())
}

fn cmd_entities(store: &Store, action: GraphAction) -> Result<(), CliError> {
    let GraphAction::Entities { kind, scope, json } = action else {
        unreachable!("cmd_entities only ever receives GraphAction::Entities")
    };
    let filter = scope.to_filter()?;
    let summaries = store.entities(filter.as_ref(), kind.as_deref())?;

    let mut out = io::stdout().lock();
    if json {
        serde_json::to_writer(&mut out, &summaries)?;
        writeln!(out)?;
        return Ok(());
    }
    for s in &summaries {
        let kind = s.entity.kind.as_deref().unwrap_or("-");
        writeln!(
            out,
            "{}\t{}\t{}\t{}",
            s.entity.id, s.entity.name, kind, s.fact_count
        )?;
    }
    Ok(())
}

fn cmd_history(store: &Store, action: GraphAction) -> Result<(), CliError> {
    let GraphAction::History {
        fact_id,
        format,
        json,
    } = action
    else {
        unreachable!("cmd_history only ever receives GraphAction::History")
    };
    // `--json` is a shortcut for `--format json`; `conflicts_with` on both
    // fields already rejects `--json` paired with an explicit `--format
    // ids`/`--format table`.
    let format = if json { HistoryFormat::Json } else { format };
    let id = fact_id.parse::<FactId>().map_err(CliError::InvalidFactId)?;
    let chain = store.fact_history(id)?;

    let mut out = io::stdout().lock();
    match format {
        HistoryFormat::Ids => {
            for fact in &chain {
                writeln!(out, "{}", fact.id)?;
            }
        }
        HistoryFormat::Table => {
            for fact in &chain {
                writeln!(out, "{}", render_fact(fact))?;
            }
        }
        HistoryFormat::Json => {
            serde_json::to_writer(&mut out, &chain)?;
            writeln!(out)?;
        }
    }
    Ok(())
}

/// Print `facts`, one per line via [`render_fact`] (each optionally followed
/// by its source item's first line), or as a single JSON array.
fn render_facts(
    store: &Store,
    facts: &[Fact],
    with_sources: bool,
    json: bool,
) -> Result<(), CliError> {
    let mut out = io::stdout().lock();
    if json {
        serde_json::to_writer(&mut out, facts)?;
        writeln!(out)?;
        return Ok(());
    }
    for fact in facts {
        writeln!(out, "{}", render_fact(fact))?;
        if with_sources {
            if let Some(line) = source_first_line(store, fact)? {
                writeln!(out, "    \u{21b3} {line}")?;
            }
        }
    }
    Ok(())
}

/// The first line of `fact`'s source item's content, if it has one.
fn source_first_line(store: &Store, fact: &Fact) -> Result<Option<String>, CliError> {
    let Some(id) = fact.source_item_id else {
        return Ok(None);
    };
    let item = store.get(id)?;
    Ok(Some(item.content.lines().next().unwrap_or("").to_string()))
}

/// `"{id}  {subject} —{predicate}→ {object}  [{from}, {to})  conf={:.2}
/// scope={}  src={}"` — `open` for a null `valid_to`, `?` for a null
/// `valid_from`, `-` for an absent scope/source.
fn render_fact(fact: &Fact) -> String {
    use singularmem_core::graph::FactObject;

    let object = match &fact.object {
        FactObject::Entity(e) => e.name.as_str(),
        FactObject::Value(v) => v.as_str(),
    };
    let from = fact
        .valid_from
        .map_or_else(|| "?".to_string(), |t| t.to_string());
    let to = fact
        .valid_to
        .map_or_else(|| "open".to_string(), |t| t.to_string());
    let scope = fact.scope.as_deref().unwrap_or("-");
    let src = fact
        .source_item_id
        .map_or_else(|| "-".to_string(), |i| i.to_string());
    format!(
        "{}  {} \u{2014}{}\u{2192} {}  [{}, {})  conf={:.2}  scope={}  src={}",
        fact.id, fact.subject.name, fact.predicate, object, from, to, fact.confidence, scope, src
    )
}
