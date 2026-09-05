//! Knowledge-graph verbs: `graph add/query/predicate/invalidate/supersede/
//! timeline/stats/entities/history`, thin CLI wrappers over
//! `singularmem_core::graph`.

use std::io::{self, Write};

use clap::{Args, Subcommand, ValueEnum};
use singularmem_core::graph::time::parse_point;
use singularmem_core::graph::{Direction, Fact, GraphQuery, NewFact, NewObject};
use singularmem_core::{FactId, ItemId, Store};

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
        subject: String,
        predicate: String,
        object: String,
        /// OBJECT is a literal value, not an entity.
        #[arg(long)]
        value: bool,
        #[arg(long)]
        subject_kind: Option<String>,
        #[arg(long)]
        object_kind: Option<String>,
        #[arg(long, value_name = "TS")]
        from: Option<String>,
        #[arg(long, value_name = "TS")]
        to: Option<String>,
        #[arg(long, default_value_t = 1.0)]
        confidence: f32,
        #[arg(long, value_name = "ITEM_ID")]
        source: Option<String>,
        #[arg(long, value_name = "PATH")]
        scope: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Facts about an entity.
    Query {
        entity: String,
        #[arg(long, value_enum, default_value_t = DirectionArg::Both)]
        direction: DirectionArg,
        #[arg(long, value_name = "TS")]
        as_of: Option<String>,
        #[arg(long, value_name = "TS")]
        recorded_at: Option<String>,
        #[command(flatten)]
        scope: ScopeArgs,
        #[arg(long)]
        with_sources: bool,
        #[arg(long)]
        json: bool,
    },
    /// Facts with a predicate.
    Predicate {
        predicate: String,
        #[arg(long, value_name = "TS")]
        as_of: Option<String>,
        #[arg(long, value_name = "TS")]
        recorded_at: Option<String>,
        #[command(flatten)]
        scope: ScopeArgs,
        #[arg(long)]
        json: bool,
    },
    /// Close an open fact (append-only: writes a new revision).
    Invalidate {
        subject: String,
        predicate: String,
        object: String,
        #[arg(long)]
        value: bool,
        #[arg(long, value_name = "TS")]
        at: Option<String>,
        #[arg(long, value_name = "PATH")]
        scope: Option<String>,
    },
    /// Replace OLD with NEW at one instant, in one transaction.
    Supersede {
        subject: String,
        predicate: String,
        old: String,
        new: String,
        #[arg(long)]
        value: bool,
        #[arg(long, value_name = "TS")]
        at: Option<String>,
        #[arg(long, value_name = "PATH")]
        scope: Option<String>,
    },
    /// Chronological facts, optionally for one entity.
    Timeline {
        entity: Option<String>,
        #[command(flatten)]
        scope: ScopeArgs,
        #[arg(long)]
        json: bool,
    },
    /// Counts.
    Stats {
        #[command(flatten)]
        scope: ScopeArgs,
        #[arg(long)]
        json: bool,
    },
    /// Entities with fact counts.
    Entities {
        #[arg(long)]
        kind: Option<String>,
        #[command(flatten)]
        scope: ScopeArgs,
        #[arg(long)]
        json: bool,
    },
    /// All revisions of a fact, oldest first.
    History {
        fact_id: String,
        #[arg(long, value_enum, default_value_t = HistoryFormat::Table)]
        format: HistoryFormat,
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
/// what `query`, `predicate`, and (with its direction overwritten) `query`
/// again all start from.
fn window_query(
    as_of: Option<&str>,
    recorded_at: Option<&str>,
    scope: &ScopeArgs,
) -> Result<GraphQuery, CliError> {
    Ok(GraphQuery {
        scope: scope.to_filter()?,
        as_of: as_of.map(parse_point).transpose()?,
        recorded_at: recorded_at.map(parse_point).transpose()?,
        direction: Direction::default(),
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
    let valid_from = from.as_deref().map(parse_point).transpose()?;
    let valid_to = to.as_deref().map(parse_point).transpose()?;
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
    let at = at.as_deref().map(parse_point).transpose()?;
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
    let at = at.as_deref().map(parse_point).transpose()?;
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
        let marker = if entry.current { "  current" } else { "" };
        writeln!(out, "{}{marker}", render_fact(&entry.fact))?;
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
    let GraphAction::History { fact_id, format } = action else {
        unreachable!("cmd_history only ever receives GraphAction::History")
    };
    let id = fact_id.parse::<FactId>()?;
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
