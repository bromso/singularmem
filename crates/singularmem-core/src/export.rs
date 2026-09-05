//! `Store::export` — emit the entire store as JSONL on a writer.
//!
//! Format spec: `docs/formats/store-v4.md` § "Export — `export-v2`".

use std::io::Write;

use jiff::Timestamp;
use serde::Serialize;

use crate::error::{Error, Result};
use crate::format::{EXPORT_FORMAT, FORMAT_VERSION};
use crate::graph::{Entity, EntityRef, FactObject};
use crate::id::{EntityId, FactId};
use crate::item::{Item, ItemId};
use crate::store::Store;

#[derive(Serialize)]
struct ExportMeta<'a> {
    #[serde(rename = "_singularmem_format")]
    format: &'a str,
    #[serde(rename = "_kind")]
    kind: &'a str,
    store_format_version: &'a str,
    exported_at: String,
}

#[derive(Serialize)]
struct ExportItem<'a> {
    #[serde(rename = "_kind")]
    kind: &'a str,
    #[serde(flatten)]
    item: &'a Item,
}

#[derive(Serialize)]
struct ExportEntity<'a> {
    #[serde(rename = "_kind")]
    line_kind: &'a str,
    id: EntityId,
    name: &'a str,
    normalised_name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<&'a str>,
    created_at: Timestamp,
}

#[derive(Serialize)]
struct ExportFact<'a> {
    #[serde(rename = "_kind")]
    line_kind: &'a str,
    id: FactId,
    subject: &'a EntityRef,
    predicate: &'a str,
    object: &'a FactObject,
    #[serde(skip_serializing_if = "Option::is_none")]
    valid_from: Option<Timestamp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    valid_to: Option<Timestamp>,
    confidence: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_item_id: Option<ItemId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scope: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    supersedes: Option<FactId>,
    recorded_at: Timestamp,
}

impl Store {
    /// Stream every item, entity, and fact revision in the store as JSONL
    /// into `w`. Format defined in `docs/formats/store-v4.md`
    /// (`export-v2`). Deterministic order: meta line first, then items
    /// (`created_at` ascending), then entities (`created_at` ascending,
    /// then id), then fact revisions (`recorded_at` ascending, then id).
    ///
    /// Loaders MUST ignore unknown `_kind`s so this format can grow new
    /// line kinds without breaking existing readers.
    ///
    /// # Errors
    ///
    /// Returns `Error::Sqlite` if the underlying enumeration fails;
    /// `Error::Io` if the writer fails; `Error::Json` if serialisation
    /// fails (should not happen given the validated input).
    pub fn export(&self, w: &mut dyn Write) -> Result<()> {
        let now = self.clock.now().to_string();
        let meta = ExportMeta {
            format: EXPORT_FORMAT,
            kind: "meta",
            store_format_version: FORMAT_VERSION,
            exported_at: now,
        };
        serde_json::to_writer(&mut *w, &meta).map_err(|e| Error::Json {
            context: "writing export meta line",
            source: e,
        })?;
        writeln!(w)?;

        for item_result in self.list()? {
            let item = item_result?;
            let line = ExportItem {
                kind: "item",
                item: &item,
            };
            serde_json::to_writer(&mut *w, &line).map_err(|e| Error::Json {
                context: "writing export item line",
                source: e,
            })?;
            writeln!(w)?;
        }

        let mut entities: Vec<Entity> = self
            .entities(None, None)?
            .into_iter()
            .map(|summary| summary.entity)
            .collect();
        entities.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        for entity in &entities {
            let line = ExportEntity {
                line_kind: "entity",
                id: entity.id,
                name: &entity.name,
                normalised_name: &entity.normalised_name,
                kind: entity.kind.as_deref(),
                created_at: entity.created_at,
            };
            serde_json::to_writer(&mut *w, &line).map_err(|e| Error::Json {
                context: "writing export entity line",
                source: e,
            })?;
            writeln!(w)?;
        }

        for fact in self.all_facts_chronological()? {
            let line = ExportFact {
                line_kind: "fact",
                id: fact.id,
                subject: &fact.subject,
                predicate: &fact.predicate,
                object: &fact.object,
                valid_from: fact.valid_from,
                valid_to: fact.valid_to,
                confidence: fact.confidence,
                source_item_id: fact.source_item_id,
                scope: fact.scope.as_deref(),
                supersedes: fact.supersedes,
                recorded_at: fact.recorded_at,
            };
            serde_json::to_writer(&mut *w, &line).map_err(|e| Error::Json {
                context: "writing export fact line",
                source: e,
            })?;
            writeln!(w)?;
        }

        w.flush()?;
        Ok(())
    }
}
