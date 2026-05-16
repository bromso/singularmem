//! `Store::export` — emit the entire store as JSONL on a writer.
//!
//! Format spec: `docs/formats/store-v1.md` § "Export format — `export-v1`".

use std::io::Write;

use serde::Serialize;

use crate::error::{Error, Result};
use crate::format::{EXPORT_FORMAT, FORMAT_VERSION};
use crate::item::Item;
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

impl Store {
    /// Stream every item in the store as JSONL into `w`. Format defined in
    /// `docs/formats/store-v1.md` ("export-v1"). Deterministic order: meta
    /// line first, then items in `created_at` ascending.
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
        w.flush()?;
        Ok(())
    }
}
