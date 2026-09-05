//! `memory_graph_*` tools — the temporal knowledge graph's MCP surface.
//!
//! Eight tools mirror the CLI's `graph` verbs: `add`/`invalidate`/`supersede`
//! are writers (hidden and rejected in `--read-only` mode, exactly like
//! `memory_ingest`); `query`/`timeline`/`stats`/`entities`/`history` are
//! readers, always listed. Spec:
//! `docs/superpowers/specs/2026-09-05-knowledge-graph-14-design.md`
//! § "Wire (MCP)" and
//! `docs/superpowers/specs/2026-09-05-mcp-surface-16-design.md`
//! §§ "`memory_graph_entities` tool" / "`memory_graph_history` tool".

use std::fmt::Write as _;
use std::str::FromStr;

use jiff::Timestamp;
use rmcp::model::{Tool, ToolAnnotations};
use serde::{Deserialize, Serialize};

use singularmem_core::graph::time::parse_point;
use singularmem_core::graph::{
    Direction, EntitySummary, Fact, FactObject, GraphQuery, GraphStats, NewFact, NewObject,
    TimelineEntry,
};
use singularmem_core::{FactId, ItemId};

use crate::tools::util::{open_store_for_reading, open_store_for_writing, scope_filter};
use crate::{Config, Error, Result};

/// Shared handler output: a single text block. All eight `memory_graph_*`
/// handlers use this — none needs structured fields beyond the rendered
/// text.
#[derive(Debug, Clone)]
pub struct MemoryGraphOutput {
    /// Formatted text block per the spec.
    pub text: String,
}

/// JSON-deserialised arguments for the `memory_graph_add` tool.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemoryGraphAddArgs {
    /// Subject entity name (created if it doesn't exist).
    pub subject: String,
    /// Predicate, e.g. `"uses"` or `"works_at"`.
    pub predicate: String,
    /// Object: an entity name, or a literal value when `object_is_value`.
    pub object: String,
    /// When `true`, `object` is a literal string value rather than an
    /// entity name. Default `false`.
    #[serde(default)]
    pub object_is_value: Option<bool>,
    /// Kind to set on the subject if it is being created for the first time.
    #[serde(default)]
    pub subject_kind: Option<String>,
    /// Kind to set on the object entity if it is being created for the
    /// first time. Ignored when `object_is_value` is `true`.
    #[serde(default)]
    pub object_kind: Option<String>,
    /// Start of the validity window (`YYYY-MM-DD` or RFC 3339).
    #[serde(default)]
    pub valid_from: Option<String>,
    /// End of the validity window (`YYYY-MM-DD` or RFC 3339).
    #[serde(default)]
    pub valid_to: Option<String>,
    /// Confidence in `[0.0, 1.0]`. Default `1.0`.
    #[serde(default)]
    pub confidence: Option<f32>,
    /// ULID of the item this fact was extracted from, if any.
    #[serde(default)]
    pub source_item_id: Option<String>,
    /// Scope path to record this fact under, if any.
    #[serde(default)]
    pub scope: Option<String>,
}

/// JSON-deserialised arguments for the `memory_graph_query` tool.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemoryGraphQueryArgs {
    /// Entity name to query facts about. Exactly one of `entity`/`predicate`
    /// is required.
    #[serde(default)]
    pub entity: Option<String>,
    /// Predicate to query facts by. Exactly one of `entity`/`predicate` is
    /// required.
    #[serde(default)]
    pub predicate: Option<String>,
    /// With `entity`, which side of the fact to match: `"outgoing"`
    /// (entity is subject), `"incoming"` (entity is object), or `"both"`
    /// (default).
    #[serde(default)]
    pub direction: Option<String>,
    /// Restrict to facts valid at this instant (`YYYY-MM-DD` or RFC 3339).
    #[serde(default)]
    pub as_of: Option<String>,
    /// Restrict to facts believed as of this record time (`YYYY-MM-DD` or
    /// RFC 3339).
    #[serde(default)]
    pub recorded_at: Option<String>,
    /// Restrict to this scope path and its descendants (or, with
    /// `scope_exact`, only this exact scope).
    #[serde(default)]
    pub scope: Option<String>,
    /// When `true`, match only the exact scope given in `scope`. Default
    /// `false`.
    #[serde(default)]
    pub scope_exact: Option<bool>,
}

/// JSON-deserialised arguments for the `memory_graph_invalidate` tool.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemoryGraphInvalidateArgs {
    /// Subject entity name.
    pub subject: String,
    /// Predicate.
    pub predicate: String,
    /// Object: an entity name, or a literal value when `object_is_value`.
    pub object: String,
    /// When `true`, `object` is a literal string value rather than an
    /// entity name. Default `false`.
    #[serde(default)]
    pub object_is_value: Option<bool>,
    /// Instant the fact ended (`YYYY-MM-DD` or RFC 3339). Default: now.
    #[serde(default)]
    pub at: Option<String>,
    /// Scope the fact was recorded under, if any.
    #[serde(default)]
    pub scope: Option<String>,
}

/// JSON-deserialised arguments for the `memory_graph_supersede` tool.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemoryGraphSupersedeArgs {
    /// Subject entity name.
    pub subject: String,
    /// Predicate.
    pub predicate: String,
    /// The fact's current object: an entity name, or a literal value when
    /// `object_is_value`. Tolerated if no open fact matches — the
    /// response reports `closed: none`.
    pub old_object: String,
    /// The fact's replacement object, same shape as `old_object`.
    pub new_object: String,
    /// When `true`, both `old_object` and `new_object` are literal string
    /// values rather than entity names. Default `false`.
    #[serde(default)]
    pub object_is_value: Option<bool>,
    /// Instant the change took effect (`YYYY-MM-DD` or RFC 3339). Default:
    /// now.
    #[serde(default)]
    pub at: Option<String>,
    /// Scope the fact was recorded under, if any.
    #[serde(default)]
    pub scope: Option<String>,
}

/// JSON-deserialised arguments for the `memory_graph_timeline` tool.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemoryGraphTimelineArgs {
    /// Restrict to facts touching this entity. Omit for the whole graph.
    #[serde(default)]
    pub entity: Option<String>,
    /// Restrict to this scope path and its descendants (or, with
    /// `scope_exact`, only this exact scope).
    #[serde(default)]
    pub scope: Option<String>,
    /// When `true`, match only the exact scope given in `scope`. Default
    /// `false`.
    #[serde(default)]
    pub scope_exact: Option<bool>,
}

/// JSON-deserialised arguments for the `memory_graph_stats` tool.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemoryGraphStatsArgs {
    /// Restrict counts to this scope path and its descendants, if any.
    #[serde(default)]
    pub scope: Option<String>,
}

/// JSON-deserialised arguments for the `memory_graph_entities` tool.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct MemoryGraphEntitiesArgs {
    /// Restrict to entities of this kind.
    #[serde(default)]
    pub kind: Option<String>,
    /// Restrict to entities with at least one fact in this scope path and
    /// its descendants (or, with `scope_exact`, only this exact scope).
    #[serde(default)]
    pub scope: Option<String>,
    /// When `true`, match only the exact scope given in `scope`. Default
    /// `false`.
    #[serde(default)]
    pub scope_exact: Option<bool>,
}

/// JSON-deserialised arguments for the `memory_graph_history` tool.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemoryGraphHistoryArgs {
    /// ULID of the fact whose revision chain to show.
    pub fact_id: String,
}

/// Render one fact the way the CLI's `graph` verbs do (duplicated
/// minimally here; see Task 6 brief for why a shared home in
/// `singularmem_core` is a follow-up rather than done now).
fn render_fact(f: &Fact) -> String {
    let from = f
        .valid_from
        .map_or_else(|| "?".to_string(), |t| t.to_string());
    let to = f
        .valid_to
        .map_or_else(|| "open".to_string(), |t| t.to_string());
    let object = match &f.object {
        FactObject::Entity(e) => e.name.clone(),
        FactObject::Value(v) => v.clone(),
    };
    let scope = f.scope.as_deref().unwrap_or("-");
    let src = f
        .source_item_id
        .map_or_else(|| "-".to_string(), |id| id.to_string());
    format!(
        "{}  {} —{}→ {}  [{from}, {to})  conf={:.2}  scope={scope}  src={src}",
        f.id, f.subject.name, f.predicate, object, f.confidence
    )
}

/// Render a list of facts, one per line, or `"No facts."` when empty. No
/// trailing newline, matching the other handlers' text output.
fn render_facts(facts: &[Fact]) -> String {
    if facts.is_empty() {
        return "No facts.".to_string();
    }
    let mut text = String::new();
    for fact in facts {
        writeln!(text, "{}", render_fact(fact)).expect("write to String is infallible");
    }
    text.pop(); // drop the trailing '\n' left by the last writeln!
    text
}

/// Render a timeline, one entry per line prefixed `[current]`/`[closed]`,
/// or `"No facts."` when empty. No trailing newline.
fn render_timeline(entries: &[TimelineEntry]) -> String {
    if entries.is_empty() {
        return "No facts.".to_string();
    }
    let mut text = String::new();
    for entry in entries {
        let tag = if entry.current { "current" } else { "closed" };
        writeln!(text, "[{tag}] {}", render_fact(&entry.fact))
            .expect("write to String is infallible");
    }
    text.pop(); // drop the trailing '\n' left by the last writeln!
    text
}

/// Render graph stats as a four-line block, mirroring the CLI's `graph
/// stats` human format exactly (`src/commands/graph.rs`'s `cmd_stats`). No
/// trailing newline.
fn render_stats(s: &GraphStats) -> String {
    format!(
        "entities: {}\nopen facts: {}\nclosed facts: {}\npredicates: {}",
        s.entities, s.open_facts, s.closed_facts, s.predicates
    )
}

/// Render one entity summary the way the CLI's `graph entities` command
/// does (`src/commands/graph.rs`'s `cmd_entities`): tab-separated id, name,
/// kind (`-` when absent), fact count.
fn render_entity(e: &EntitySummary) -> String {
    format!(
        "{}\t{}\t{}\t{}",
        e.entity.id,
        e.entity.name,
        e.entity.kind.as_deref().unwrap_or("-"),
        e.fact_count
    )
}

/// Render a list of entity summaries, one per line, or `"No entities."`
/// when empty. No trailing newline.
fn render_entities(entities: &[EntitySummary]) -> String {
    if entities.is_empty() {
        return "No entities.".to_string();
    }
    let mut text = String::new();
    for e in entities {
        writeln!(text, "{}", render_entity(e)).expect("write to String is infallible");
    }
    text.pop(); // drop the trailing '\n' left by the last writeln!
    text
}

/// Build a [`NewObject`]/query object from a raw string per `object_is_value`.
fn build_object(raw: String, is_value: Option<bool>, kind: Option<String>) -> NewObject {
    if is_value.unwrap_or(false) {
        NewObject::Value(raw)
    } else {
        NewObject::Entity { name: raw, kind }
    }
}

/// Parse an optional timestamp argument with [`parse_point`], renaming a
/// validation failure's `field` from `parse_point`'s generic `"timestamp"`
/// to `arg_name` (e.g. `"valid_from"`, `"at"`, `"as_of"`) so the error names
/// the actual tool argument that was rejected.
fn parse_optional_point(raw: Option<&str>, arg_name: &'static str) -> Result<Option<Timestamp>> {
    raw.map(parse_point)
        .transpose()
        .map_err(|e| match e {
            singularmem_core::Error::Validation { reason, .. } => {
                singularmem_core::Error::Validation {
                    field: arg_name,
                    reason,
                }
            }
            other => other,
        })
        .map_err(Error::Core)
}

/// Parse the `direction` argument, defaulting to [`Direction::Both`].
fn parse_direction(raw: Option<&str>) -> Result<Direction> {
    match raw {
        Some("outgoing") => Ok(Direction::Outgoing),
        Some("incoming") => Ok(Direction::Incoming),
        None | Some("both") => Ok(Direction::Both),
        Some(other) => Err(Error::Core(singularmem_core::Error::Validation {
            field: "direction",
            reason: format!("{other:?} must be one of \"outgoing\", \"incoming\", \"both\""),
        })),
    }
}

/// Build the rmcp tool descriptor for `memory_graph_add`.
///
/// # Panics
/// Panics if the hard-coded JSON schema literal is not an object (never happens).
#[must_use]
pub fn tool_descriptor_add() -> Tool {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "subject": { "type": "string", "description": "Subject entity name (created if it doesn't exist)." },
            "predicate": { "type": "string", "description": "Predicate, e.g. \"uses\" or \"works_at\"." },
            "object": { "type": "string", "description": "Object: an entity name, or a literal value when object_is_value is true." },
            "object_is_value": { "type": "boolean", "default": false, "description": "When true, object is a literal string value rather than an entity name." },
            "subject_kind": { "type": "string", "description": "Kind to set on the subject if it is being created for the first time." },
            "object_kind": { "type": "string", "description": "Kind to set on the object entity if it is being created for the first time. Ignored when object_is_value is true." },
            "valid_from": { "type": "string", "description": "Start of the validity window (YYYY-MM-DD or RFC 3339). Omit if unknown." },
            "valid_to": { "type": "string", "description": "End of the validity window (YYYY-MM-DD or RFC 3339). Omit if still valid." },
            "confidence": { "type": "number", "minimum": 0.0, "maximum": 1.0, "default": 1.0, "description": "Confidence in [0.0, 1.0]." },
            "source_item_id": { "type": "string", "description": "ULID of the memory this fact was extracted from, if any." },
            "scope": { "type": "string", "description": "Scope path to record this fact under, if any." }
        },
        "required": ["subject", "predicate", "object"]
    });
    Tool::new(
        "memory_graph_add",
        "Record a new, independent fact in the temporal knowledge graph (subject-predicate-\
         object, with an optional validity window and confidence). Use this for facts that \
         don't replace an existing one — for a fact whose old value should stop being current, \
         use memory_graph_supersede instead. Returns the new fact's rendered text.",
        schema.as_object().expect("schema is object").clone(),
    )
    .annotate(ToolAnnotations::new().read_only(false))
}

/// Build the rmcp tool descriptor for `memory_graph_query`.
///
/// # Panics
/// Panics if the hard-coded JSON schema literal is not an object (never happens).
#[must_use]
pub fn tool_descriptor_query() -> Tool {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "entity": { "type": "string", "description": "Entity name to query facts about. Exactly one of entity/predicate is required." },
            "predicate": { "type": "string", "description": "Predicate to query facts by. Exactly one of entity/predicate is required." },
            "direction": { "type": "string", "enum": ["outgoing", "incoming", "both"], "default": "both", "description": "With entity, which side of the fact to match." },
            "as_of": { "type": "string", "description": "Restrict to facts valid at this instant (YYYY-MM-DD or RFC 3339)." },
            "recorded_at": { "type": "string", "description": "Restrict to facts believed as of this record time (YYYY-MM-DD or RFC 3339)." },
            "scope": { "type": "string", "description": "Restrict to this scope path and its descendants, e.g. \"claude-code/myproj\"." },
            "scope_exact": { "type": "boolean", "default": false, "description": "Match only the exact scope given in scope." }
        },
        "required": []
    });
    Tool::new(
        "memory_graph_query",
        "Query current or historical facts in the temporal knowledge graph by entity or by \
         predicate. Use this before answering questions about who owns what, which tool is \
         used, or any other current-state fact — it is more reliable than free-text retrieval \
         for structured facts. Returns one rendered fact per line, or \"No facts.\".",
        schema.as_object().expect("schema is object").clone(),
    )
    .annotate(ToolAnnotations::new().read_only(true))
}

/// Build the rmcp tool descriptor for `memory_graph_invalidate`.
///
/// # Panics
/// Panics if the hard-coded JSON schema literal is not an object (never happens).
#[must_use]
pub fn tool_descriptor_invalidate() -> Tool {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "subject": { "type": "string", "description": "Subject entity name." },
            "predicate": { "type": "string", "description": "Predicate." },
            "object": { "type": "string", "description": "Object: an entity name, or a literal value when object_is_value is true." },
            "object_is_value": { "type": "boolean", "default": false, "description": "When true, object is a literal string value rather than an entity name." },
            "at": { "type": "string", "description": "Instant the fact ended (YYYY-MM-DD or RFC 3339). Defaults to now." },
            "scope": { "type": "string", "description": "Scope the fact was recorded under, if any." }
        },
        "required": ["subject", "predicate", "object"]
    });
    Tool::new(
        "memory_graph_invalidate",
        "End an open fact — mark subject-predicate-object as no longer true, without \
         recording a replacement value. Use this for facts that simply ended (someone left a \
         role, a tool was retired); when there's a new value to record in its place, use \
         memory_graph_supersede instead. The original row is never modified; this appends a \
         closing revision. Returns the closing revision's rendered text.",
        schema.as_object().expect("schema is object").clone(),
    )
    .annotate(ToolAnnotations::new().read_only(false))
}

/// Build the rmcp tool descriptor for `memory_graph_supersede`.
///
/// # Panics
/// Panics if the hard-coded JSON schema literal is not an object (never happens).
#[must_use]
pub fn tool_descriptor_supersede() -> Tool {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "subject": { "type": "string", "description": "Subject entity name." },
            "predicate": { "type": "string", "description": "Predicate." },
            "old_object": { "type": "string", "description": "The fact's current object. Tolerated if no open fact matches — the response reports \"closed: none\"." },
            "new_object": { "type": "string", "description": "The fact's replacement object, same shape as old_object." },
            "object_is_value": { "type": "boolean", "default": false, "description": "When true, both old_object and new_object are literal string values rather than entity names." },
            "at": { "type": "string", "description": "Instant the change took effect (YYYY-MM-DD or RFC 3339). Defaults to now." },
            "scope": { "type": "string", "description": "Scope the fact was recorded under, if any." }
        },
        "required": ["subject", "predicate", "old_object", "new_object"]
    });
    Tool::new(
        "memory_graph_supersede",
        "Atomically replace one fact's value with another: close the old subject-predicate-\
         object (if any) and open the new one, in a single transaction. Use this for \
         single-valued facts that changed (a person changed teams, a project switched search \
         engines) — it's the right tool whenever there's both an old value ending and a new \
         value starting. Returns two lines: the closed revision (or \"closed: none\") and the \
         opened revision.",
        schema.as_object().expect("schema is object").clone(),
    )
    .annotate(ToolAnnotations::new().read_only(false))
}

/// Build the rmcp tool descriptor for `memory_graph_timeline`.
///
/// # Panics
/// Panics if the hard-coded JSON schema literal is not an object (never happens).
#[must_use]
pub fn tool_descriptor_timeline() -> Tool {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "entity": { "type": "string", "description": "Restrict to facts touching this entity. Omit for the whole graph." },
            "scope": { "type": "string", "description": "Restrict to this scope path and its descendants, e.g. \"claude-code/myproj\"." },
            "scope_exact": { "type": "boolean", "default": false, "description": "Match only the exact scope given in scope." }
        },
        "required": []
    });
    Tool::new(
        "memory_graph_timeline",
        "List every fact revision — open and closed alike — for an entity or the whole graph, \
         ordered by validity start. Use this to see how a fact changed over time, not just its \
         current value. Returns one line per revision, prefixed [current] or [closed], or \
         \"No facts.\".",
        schema.as_object().expect("schema is object").clone(),
    )
    .annotate(ToolAnnotations::new().read_only(true))
}

/// Build the rmcp tool descriptor for `memory_graph_stats`.
///
/// # Panics
/// Panics if the hard-coded JSON schema literal is not an object (never happens).
#[must_use]
pub fn tool_descriptor_stats() -> Tool {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "scope": { "type": "string", "description": "Restrict counts to this scope path and its descendants, if any." }
        },
        "required": []
    });
    Tool::new(
        "memory_graph_stats",
        "Report aggregate counts over the knowledge graph: entities, open facts, closed \
         facts, and distinct predicates. Use this to sanity-check how much structured \
         knowledge exists before querying it in detail.",
        schema.as_object().expect("schema is object").clone(),
    )
    .annotate(ToolAnnotations::new().read_only(true))
}

/// Build the rmcp tool descriptor for `memory_graph_entities`.
///
/// # Panics
/// Panics if the hard-coded JSON schema literal is not an object (never happens).
#[must_use]
pub fn tool_descriptor_entities() -> Tool {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "kind": { "type": "string", "description": "Restrict to entities of this kind." },
            "scope": { "type": "string", "description": "Restrict to entities with at least one fact in this scope path and its descendants, e.g. \"claude-code/myproj\"." },
            "scope_exact": { "type": "boolean", "default": false, "description": "Match only the exact scope given in scope." }
        },
        "required": []
    });
    Tool::new(
        "memory_graph_entities",
        "List entities in the knowledge graph with their fact counts. Filter by kind or by \
         the scope of their facts. Use before memory_graph_query when you are unsure of an \
         entity's exact name. Returns one line per entity, tab-separated id, name, kind (\"-\" \
         when absent), and fact count, or \"No entities.\".",
        schema.as_object().expect("schema is object").clone(),
    )
    .annotate(ToolAnnotations::new().read_only(true))
}

/// Build the rmcp tool descriptor for `memory_graph_history`.
///
/// # Panics
/// Panics if the hard-coded JSON schema literal is not an object (never happens).
#[must_use]
pub fn tool_descriptor_history() -> Tool {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "fact_id": { "type": "string", "description": "ULID of the fact whose revision chain to show." }
        },
        "required": ["fact_id"]
    });
    Tool::new(
        "memory_graph_history",
        "Show every revision of one fact, oldest first: how its validity window was closed or \
         replaced over time. Take fact_id from memory_graph_query or memory_graph_timeline \
         output. Returns one rendered fact per line.",
        schema.as_object().expect("schema is object").clone(),
    )
    .annotate(ToolAnnotations::new().read_only(true))
}

/// Handle a `tools/call` for `memory_graph_add`.
///
/// # Errors
/// - [`Error::ReadOnly`] when `config.read_only` is `true`.
/// - [`Error::InvalidId`] when `args.source_item_id` doesn't parse as a `ULID`.
/// - [`Error::Core`] wrapping [`singularmem_core::Error::Validation`] for bad
///   input; nothing is written.
pub fn handle_memory_graph_add(
    args: MemoryGraphAddArgs,
    config: &Config,
) -> Result<MemoryGraphOutput> {
    if config.read_only {
        return Err(Error::ReadOnly);
    }
    let object = build_object(args.object, args.object_is_value, args.object_kind);
    let valid_from = parse_optional_point(args.valid_from.as_deref(), "valid_from")?;
    let valid_to = parse_optional_point(args.valid_to.as_deref(), "valid_to")?;
    let source_item_id = args
        .source_item_id
        .as_deref()
        .map(ItemId::from_str)
        .transpose()
        .map_err(|e| Error::InvalidId(e.to_string()))?;

    let new_fact = NewFact {
        subject: args.subject,
        subject_kind: args.subject_kind,
        predicate: args.predicate,
        object,
        valid_from,
        valid_to,
        confidence: args.confidence.unwrap_or(1.0),
        source_item_id,
        scope: args.scope,
    };

    let store = open_store_for_writing(config)?;
    let fact = store.add_fact(new_fact)?;
    Ok(MemoryGraphOutput {
        text: render_fact(&fact),
    })
}

/// Handle a `tools/call` for `memory_graph_query`.
///
/// # Errors
/// - [`Error::Core`] wrapping [`singularmem_core::Error::Validation`]
///   (`field: "query"`) unless exactly one of `entity`/`predicate` is given,
///   or (`field: "direction"`) for an unrecognised `direction`, or for a
///   bad `scope`/timestamp/entity/predicate name.
pub fn handle_memory_graph_query(
    args: &MemoryGraphQueryArgs,
    config: &Config,
) -> Result<MemoryGraphOutput> {
    // Validate the entity/predicate XOR before touching the store at all.
    if matches!(
        (args.entity.as_deref(), args.predicate.as_deref()),
        (None, None) | (Some(_), Some(_))
    ) {
        return Err(Error::Core(singularmem_core::Error::Validation {
            field: "query",
            reason: "exactly one of entity or predicate is required".to_string(),
        }));
    }

    let scope = scope_filter(args.scope.as_deref(), args.scope_exact)?;
    let as_of = parse_optional_point(args.as_of.as_deref(), "as_of")?;
    let recorded_at = parse_optional_point(args.recorded_at.as_deref(), "recorded_at")?;
    let direction = parse_direction(args.direction.as_deref())?;
    let q = GraphQuery {
        scope,
        as_of,
        recorded_at,
        direction,
    };

    let store = open_store_for_reading(config)?;
    let facts = match (args.entity.as_deref(), args.predicate.as_deref()) {
        (Some(entity), None) => store.query_entity(entity, &q)?,
        (None, Some(predicate)) => store.query_predicate(predicate, &q)?,
        (None, None) | (Some(_), Some(_)) => {
            unreachable!("validated above: exactly one of entity/predicate is present")
        }
    };

    Ok(MemoryGraphOutput {
        text: render_facts(&facts),
    })
}

/// Handle a `tools/call` for `memory_graph_invalidate`.
///
/// # Errors
/// - [`Error::ReadOnly`] when `config.read_only` is `true`.
/// - [`Error::Core`] wrapping [`singularmem_core::Error::FactNotFound`] if no
///   open head matches; wrapping [`singularmem_core::Error::Validation`] for
///   bad input.
pub fn handle_memory_graph_invalidate(
    args: MemoryGraphInvalidateArgs,
    config: &Config,
) -> Result<MemoryGraphOutput> {
    if config.read_only {
        return Err(Error::ReadOnly);
    }
    let object = build_object(args.object, args.object_is_value, None);
    let at = parse_optional_point(args.at.as_deref(), "at")?;

    let store = open_store_for_writing(config)?;
    let closed = store.invalidate_fact(
        &args.subject,
        &args.predicate,
        &object,
        args.scope.as_deref(),
        at,
    )?;
    Ok(MemoryGraphOutput {
        text: render_fact(&closed),
    })
}

/// Handle a `tools/call` for `memory_graph_supersede`.
///
/// # Errors
/// - [`Error::ReadOnly`] when `config.read_only` is `true`.
/// - [`Error::Core`] wrapping [`singularmem_core::Error::Validation`] for
///   bad input; a missing old fact is tolerated (reported as `closed: none`)
///   rather than erroring.
pub fn handle_memory_graph_supersede(
    args: MemoryGraphSupersedeArgs,
    config: &Config,
) -> Result<MemoryGraphOutput> {
    if config.read_only {
        return Err(Error::ReadOnly);
    }
    let old = build_object(args.old_object, args.object_is_value, None);
    let new = build_object(args.new_object, args.object_is_value, None);
    let at = parse_optional_point(args.at.as_deref(), "at")?;

    let store = open_store_for_writing(config)?;
    let (closed, opened) = store.supersede_fact(
        &args.subject,
        &args.predicate,
        &old,
        new,
        args.scope.as_deref(),
        at,
    )?;

    let closed_text = closed
        .as_ref()
        .map_or_else(|| "none".to_string(), render_fact);
    Ok(MemoryGraphOutput {
        text: format!("closed: {closed_text}\nopened: {}", render_fact(&opened)),
    })
}

/// Handle a `tools/call` for `memory_graph_timeline`.
///
/// # Errors
/// [`Error::Core`] wrapping [`singularmem_core::Error::Validation`] for a
/// bad `scope` or entity name.
pub fn handle_memory_graph_timeline(
    args: &MemoryGraphTimelineArgs,
    config: &Config,
) -> Result<MemoryGraphOutput> {
    let scope = scope_filter(args.scope.as_deref(), args.scope_exact)?;
    let store = open_store_for_reading(config)?;
    let entries = store.timeline(args.entity.as_deref(), scope.as_ref())?;
    Ok(MemoryGraphOutput {
        text: render_timeline(&entries),
    })
}

/// Handle a `tools/call` for `memory_graph_stats`.
///
/// # Errors
/// [`Error::Core`] wrapping [`singularmem_core::Error::Validation`] for a
/// bad `scope`.
pub fn handle_memory_graph_stats(
    args: &MemoryGraphStatsArgs,
    config: &Config,
) -> Result<MemoryGraphOutput> {
    let scope = scope_filter(args.scope.as_deref(), None)?;
    let store = open_store_for_reading(config)?;
    let stats = store.graph_stats(scope.as_ref())?;
    Ok(MemoryGraphOutput {
        text: render_stats(&stats),
    })
}

/// Handle a `tools/call` for `memory_graph_entities`.
///
/// # Errors
/// [`Error::Core`] wrapping [`singularmem_core::Error::Validation`] for a
/// bad `scope`.
pub fn handle_memory_graph_entities(
    args: &MemoryGraphEntitiesArgs,
    config: &Config,
) -> Result<MemoryGraphOutput> {
    let scope = scope_filter(args.scope.as_deref(), args.scope_exact)?;
    let store = open_store_for_reading(config)?;
    let rows = store.entities(scope.as_ref(), args.kind.as_deref())?;
    Ok(MemoryGraphOutput {
        text: render_entities(&rows),
    })
}

/// Handle a `tools/call` for `memory_graph_history`.
///
/// # Errors
/// - [`Error::Core`] wrapping [`singularmem_core::Error::InvalidId`] if
///   `args.fact_id` doesn't parse as a `ULID`.
/// - [`Error::Core`] wrapping [`singularmem_core::Error::FactIdNotFound`] if
///   `args.fact_id` is well-formed but unknown to the store.
/// - [`Error::Core`] wrapping
///   [`singularmem_core::Error::AmbiguousFactRevision`] if the chain forks.
pub fn handle_memory_graph_history(
    args: &MemoryGraphHistoryArgs,
    config: &Config,
) -> Result<MemoryGraphOutput> {
    let id: FactId = args
        .fact_id
        .parse()
        .map_err(singularmem_core::Error::InvalidId)?;
    let store = open_store_for_reading(config)?;
    let facts = store.fact_history(id)?;
    Ok(MemoryGraphOutput {
        text: render_facts(&facts),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use singularmem_core::Store;
    use tempfile::TempDir;

    #[allow(clippy::missing_panics_doc)]
    fn fresh_config(read_only: bool) -> (TempDir, Config) {
        let dir = TempDir::new().unwrap();
        let store_path = dir.path().join("store.db");
        // Ensure the store file exists before a read-only open is attempted.
        drop(Store::open(&store_path).unwrap());
        let config = Config::new(store_path, "plain".to_string(), read_only);
        (dir, config)
    }

    #[allow(clippy::missing_panics_doc)]
    fn seeded() -> (TempDir, Config) {
        let dir = TempDir::new().unwrap();
        let store_path = dir.path().join("store.db");
        let store = Store::open(&store_path).unwrap();
        store
            .add_fact(NewFact::triple("singularmem", "uses", "tantivy"))
            .unwrap();
        drop(store);
        let config = Config::new(store_path, "plain".to_string(), false);
        (dir, config)
    }

    fn add_args(subject: &str, predicate: &str, object: &str) -> MemoryGraphAddArgs {
        MemoryGraphAddArgs {
            subject: subject.to_string(),
            predicate: predicate.to_string(),
            object: object.to_string(),
            object_is_value: None,
            subject_kind: None,
            object_kind: None,
            valid_from: None,
            valid_to: None,
            confidence: None,
            source_item_id: None,
            scope: None,
        }
    }

    #[test]
    fn add_returns_text_with_id_and_arrow() {
        let (_dir, config) = fresh_config(false);
        let out = handle_memory_graph_add(add_args("singularmem", "uses", "tantivy"), &config)
            .expect("ok");
        assert!(out.text.contains("—uses→"), "missing arrow: {}", out.text);
        // The rendered fact starts with its id (a ULID).
        let id = out.text.split_whitespace().next().expect("id token");
        assert_eq!(
            id.len(),
            26,
            "expected a ULID-length id token: {}",
            out.text
        );
    }

    #[test]
    fn add_rejected_in_read_only_mode() {
        let (_dir, config) = fresh_config(true);
        let r = handle_memory_graph_add(add_args("singularmem", "uses", "tantivy"), &config);
        assert!(
            matches!(r, Err(Error::ReadOnly)),
            "expected ReadOnly, got {r:?}"
        );
    }

    fn query_args(entity: Option<&str>, predicate: Option<&str>) -> MemoryGraphQueryArgs {
        MemoryGraphQueryArgs {
            entity: entity.map(str::to_string),
            predicate: predicate.map(str::to_string),
            direction: None,
            as_of: None,
            recorded_at: None,
            scope: None,
            scope_exact: None,
        }
    }

    #[test]
    fn query_by_entity_finds_seeded_fact() {
        let (_dir, config) = seeded();
        let out =
            handle_memory_graph_query(&query_args(Some("singularmem"), None), &config).expect("ok");
        assert!(out.text.contains("tantivy"), "missing fact: {}", out.text);
    }

    #[test]
    fn query_by_predicate_finds_seeded_fact() {
        let (_dir, config) = seeded();
        let out = handle_memory_graph_query(&query_args(None, Some("uses")), &config).expect("ok");
        assert!(out.text.contains("tantivy"), "missing fact: {}", out.text);
    }

    #[test]
    fn query_requires_exactly_one_of_entity_or_predicate() {
        let (_dir, config) = seeded();

        let r = handle_memory_graph_query(&query_args(None, None), &config);
        assert!(
            matches!(
                r,
                Err(Error::Core(singularmem_core::Error::Validation {
                    field: "query",
                    ..
                }))
            ),
            "expected Validation{{field: 'query'}} for neither, got {r:?}"
        );

        let r = handle_memory_graph_query(&query_args(Some("singularmem"), Some("uses")), &config);
        assert!(
            matches!(
                r,
                Err(Error::Core(singularmem_core::Error::Validation {
                    field: "query",
                    ..
                }))
            ),
            "expected Validation{{field: 'query'}} for both, got {r:?}"
        );
    }

    #[test]
    fn query_as_of_sees_the_right_side_of_a_supersede() {
        let (_dir, config) = seeded();

        let supersede_args = MemoryGraphSupersedeArgs {
            subject: "singularmem".to_string(),
            predicate: "uses".to_string(),
            old_object: "tantivy".to_string(),
            new_object: "meilisearch".to_string(),
            object_is_value: None,
            at: Some("2026-06-01".to_string()),
            scope: None,
        };
        handle_memory_graph_supersede(supersede_args, &config).expect("supersede ok");

        let before = handle_memory_graph_query(
            &MemoryGraphQueryArgs {
                as_of: Some("2026-05-01".to_string()),
                ..query_args(Some("singularmem"), None)
            },
            &config,
        )
        .expect("ok");
        assert!(
            before.text.contains("tantivy"),
            "expected tantivy before: {}",
            before.text
        );
        assert!(
            !before.text.contains("meilisearch"),
            "did not expect meilisearch before: {}",
            before.text
        );

        let after = handle_memory_graph_query(
            &MemoryGraphQueryArgs {
                as_of: Some("2026-07-01".to_string()),
                ..query_args(Some("singularmem"), None)
            },
            &config,
        )
        .expect("ok");
        assert!(
            after.text.contains("meilisearch"),
            "expected meilisearch after: {}",
            after.text
        );
        assert!(
            !after.text.contains("tantivy"),
            "did not expect tantivy after: {}",
            after.text
        );
    }

    #[test]
    fn invalidate_then_query_is_empty() {
        let (_dir, config) = seeded();
        let invalidate_args = MemoryGraphInvalidateArgs {
            subject: "singularmem".to_string(),
            predicate: "uses".to_string(),
            object: "tantivy".to_string(),
            object_is_value: None,
            at: None,
            scope: None,
        };
        let out = handle_memory_graph_invalidate(invalidate_args, &config).expect("ok");
        assert!(
            out.text.contains("tantivy"),
            "expected closing revision text: {}",
            out.text
        );

        let queried =
            handle_memory_graph_query(&query_args(Some("singularmem"), None), &config).expect("ok");
        assert_eq!(
            queried.text, "No facts.",
            "expected no open facts: {}",
            queried.text
        );
    }

    #[test]
    fn invalidate_rejected_in_read_only_mode() {
        let (_dir, config) = fresh_config(true);
        let r = handle_memory_graph_invalidate(
            MemoryGraphInvalidateArgs {
                subject: "singularmem".to_string(),
                predicate: "uses".to_string(),
                object: "tantivy".to_string(),
                object_is_value: None,
                at: None,
                scope: None,
            },
            &config,
        );
        assert!(
            matches!(r, Err(Error::ReadOnly)),
            "expected ReadOnly, got {r:?}"
        );
    }

    #[test]
    fn supersede_reports_closed_and_opened() {
        let (_dir, config) = seeded();
        let out = handle_memory_graph_supersede(
            MemoryGraphSupersedeArgs {
                subject: "singularmem".to_string(),
                predicate: "uses".to_string(),
                old_object: "tantivy".to_string(),
                new_object: "meilisearch".to_string(),
                object_is_value: None,
                at: None,
                scope: None,
            },
            &config,
        )
        .expect("ok");
        assert!(
            out.text.starts_with("closed: "),
            "expected closed line: {}",
            out.text
        );
        assert!(
            out.text.contains("tantivy"),
            "expected old value in closed line: {}",
            out.text
        );
        assert!(
            out.text.contains("opened: "),
            "expected opened line: {}",
            out.text
        );
        assert!(
            out.text.contains("meilisearch"),
            "expected new value in opened line: {}",
            out.text
        );
    }

    #[test]
    fn supersede_reports_closed_none_when_nothing_open() {
        let (_dir, config) = fresh_config(false);
        let out = handle_memory_graph_supersede(
            MemoryGraphSupersedeArgs {
                subject: "ghost".to_string(),
                predicate: "uses".to_string(),
                old_object: "nothing".to_string(),
                new_object: "something".to_string(),
                object_is_value: None,
                at: None,
                scope: None,
            },
            &config,
        )
        .expect("ok");
        assert!(
            out.text.contains("closed: none"),
            "expected closed: none, got: {}",
            out.text
        );
        assert!(
            out.text.contains("something"),
            "expected new value: {}",
            out.text
        );
    }

    #[test]
    fn supersede_rejected_in_read_only_mode() {
        let (_dir, config) = fresh_config(true);
        let r = handle_memory_graph_supersede(
            MemoryGraphSupersedeArgs {
                subject: "singularmem".to_string(),
                predicate: "uses".to_string(),
                old_object: "tantivy".to_string(),
                new_object: "meilisearch".to_string(),
                object_is_value: None,
                at: None,
                scope: None,
            },
            &config,
        );
        assert!(
            matches!(r, Err(Error::ReadOnly)),
            "expected ReadOnly, got {r:?}"
        );
    }

    #[test]
    fn timeline_lists_current_and_closed() {
        let (_dir, config) = seeded();
        handle_memory_graph_supersede(
            MemoryGraphSupersedeArgs {
                subject: "singularmem".to_string(),
                predicate: "uses".to_string(),
                old_object: "tantivy".to_string(),
                new_object: "meilisearch".to_string(),
                object_is_value: None,
                at: None,
                scope: None,
            },
            &config,
        )
        .expect("supersede ok");

        let out = handle_memory_graph_timeline(
            &MemoryGraphTimelineArgs {
                entity: Some("singularmem".to_string()),
                scope: None,
                scope_exact: None,
            },
            &config,
        )
        .expect("ok");
        assert!(
            out.text.contains("[closed]"),
            "expected a closed entry: {}",
            out.text
        );
        assert!(
            out.text.contains("[current]"),
            "expected a current entry: {}",
            out.text
        );
    }

    #[test]
    fn timeline_empty_reports_no_facts() {
        let (_dir, config) = fresh_config(false);
        let out = handle_memory_graph_timeline(
            &MemoryGraphTimelineArgs {
                entity: None,
                scope: None,
                scope_exact: None,
            },
            &config,
        )
        .expect("ok");
        assert_eq!(out.text, "No facts.");
    }

    #[test]
    fn stats_reports_four_lines() {
        let (_dir, config) = seeded();
        let out =
            handle_memory_graph_stats(&MemoryGraphStatsArgs { scope: None }, &config).expect("ok");
        assert_eq!(
            out.text.lines().count(),
            4,
            "expected four lines: {}",
            out.text
        );
        // Mirror the CLI's human format exactly (src/commands/graph.rs
        // cmd_stats): "entities: N" / "open facts: N" / "closed facts: N" /
        // "predicates: N".
        assert_eq!(
            out.text, "entities: 2\nopen facts: 1\nclosed facts: 0\npredicates: 1",
            "{}",
            out.text
        );
    }

    #[test]
    fn add_bad_valid_to_names_the_argument() {
        let (_dir, config) = fresh_config(false);
        let r = handle_memory_graph_add(
            MemoryGraphAddArgs {
                valid_to: Some("not-a-timestamp".to_string()),
                ..add_args("singularmem", "uses", "tantivy")
            },
            &config,
        );
        assert!(
            matches!(
                r,
                Err(Error::Core(singularmem_core::Error::Validation {
                    field: "valid_to",
                    ..
                }))
            ),
            "expected Validation{{field: 'valid_to'}}, got {r:?}"
        );
    }

    #[test]
    fn query_bad_as_of_names_the_argument() {
        let (_dir, config) = seeded();
        let r = handle_memory_graph_query(
            &MemoryGraphQueryArgs {
                as_of: Some("not-a-timestamp".to_string()),
                ..query_args(Some("singularmem"), None)
            },
            &config,
        );
        assert!(
            matches!(
                r,
                Err(Error::Core(singularmem_core::Error::Validation {
                    field: "as_of",
                    ..
                }))
            ),
            "expected Validation{{field: 'as_of'}}, got {r:?}"
        );
    }

    #[test]
    fn supersede_bad_at_names_the_argument() {
        let (_dir, config) = seeded();
        let r = handle_memory_graph_supersede(
            MemoryGraphSupersedeArgs {
                subject: "singularmem".to_string(),
                predicate: "uses".to_string(),
                old_object: "tantivy".to_string(),
                new_object: "meilisearch".to_string(),
                object_is_value: None,
                at: Some("not-a-timestamp".to_string()),
                scope: None,
            },
            &config,
        );
        assert!(
            matches!(
                r,
                Err(Error::Core(singularmem_core::Error::Validation {
                    field: "at",
                    ..
                }))
            ),
            "expected Validation{{field: 'at'}}, got {r:?}"
        );
    }

    /// Seed two facts on `singularmem` and one on `other`, for
    /// `memory_graph_entities` tests.
    #[allow(clippy::missing_panics_doc)]
    fn seeded_graph() -> (TempDir, Config) {
        let dir = TempDir::new().unwrap();
        let store_path = dir.path().join("store.db");
        let store = Store::open(&store_path).unwrap();
        store
            .add_fact(NewFact::triple("singularmem", "uses", "tantivy"))
            .unwrap();
        store
            .add_fact(NewFact::triple("singularmem", "uses", "sqlite"))
            .unwrap();
        store
            .add_fact(NewFact::triple("other", "uses", "something"))
            .unwrap();
        drop(store);
        let config = Config::new(store_path, "plain".to_string(), false);
        (dir, config)
    }

    /// [`seeded_graph`] plus a supersede of `singularmem —uses→ tantivy`
    /// by `meilisearch`, returning the id of the closed revision `fact_history`
    /// reports.
    #[allow(clippy::missing_panics_doc)]
    fn seeded_graph_with_supersede() -> (TempDir, Config, String) {
        let (dir, config) = seeded_graph();
        let out = handle_memory_graph_supersede(
            MemoryGraphSupersedeArgs {
                subject: "singularmem".to_string(),
                predicate: "uses".to_string(),
                old_object: "tantivy".to_string(),
                new_object: "meilisearch".to_string(),
                object_is_value: None,
                at: None,
                scope: None,
            },
            &config,
        )
        .expect("supersede ok");
        let closed_line = out
            .text
            .lines()
            .next()
            .expect("closed line")
            .trim_start_matches("closed: ");
        let closed_id = closed_line
            .split_whitespace()
            .next()
            .expect("closed fact id")
            .to_string();
        (dir, config, closed_id)
    }

    #[test]
    fn entities_lists_id_name_kind_count_tab_separated() {
        let (_d, config) = seeded_graph();
        let out =
            handle_memory_graph_entities(&MemoryGraphEntitiesArgs::default(), &config).unwrap();
        let lines: Vec<&str> = out.text.lines().collect();
        assert!(
            lines.iter().any(|l| {
                let cols: Vec<&str> = l.split('\t').collect();
                cols.len() == 4 && cols[1] == "singularmem" && cols[2] == "-"
            }),
            "{}",
            out.text
        );
    }

    #[test]
    fn entities_filters_by_kind_and_reports_empty() {
        let (_d, config) = seeded_graph();
        let out = handle_memory_graph_entities(
            &MemoryGraphEntitiesArgs {
                kind: Some("planet".to_string()),
                ..Default::default()
            },
            &config,
        )
        .unwrap();
        assert_eq!(out.text, "No entities.");
    }

    #[test]
    fn history_walks_the_chain_oldest_first() {
        let (_d, config, closed_id) = seeded_graph_with_supersede();
        let out =
            handle_memory_graph_history(&MemoryGraphHistoryArgs { fact_id: closed_id }, &config)
                .unwrap();
        let lines: Vec<&str> = out.text.lines().collect();
        assert_eq!(lines.len(), 2, "{}", out.text);
        assert!(lines[0].contains("—uses→"));
        // `fact_history` returns the root fact first (its row is never
        // rewritten in place, so it still shows `valid_to: None` — the
        // "open" tail of the render — even though the closing revision
        // supersedes it) and the closing revision second (the same
        // `valid_from`, but a real `valid_to` closing the window).
        assert!(
            lines[0].ends_with(", open)") || lines[0].contains(", open)  conf="),
            "expected the root revision first (open window): {}",
            lines[0]
        );
        assert!(
            !(lines[1].contains(", open)  conf=")),
            "expected the closing revision second (closed window): {}",
            lines[1]
        );
    }

    #[test]
    fn history_unknown_and_malformed_ids_are_not_found() {
        let (_d, config) = seeded_graph();
        let err = handle_memory_graph_history(
            &MemoryGraphHistoryArgs {
                fact_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string(),
            },
            &config,
        )
        .unwrap_err();
        assert!(
            matches!(
                err,
                Error::Core(singularmem_core::Error::FactIdNotFound { .. })
            ),
            "{err:?}"
        );

        let err = handle_memory_graph_history(
            &MemoryGraphHistoryArgs {
                fact_id: "nope".to_string(),
            },
            &config,
        )
        .unwrap_err();
        assert!(
            matches!(err, Error::Core(singularmem_core::Error::InvalidId { .. })),
            "{err:?}"
        );
    }
}
