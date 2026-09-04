//! Editor hook envelopes and config-file merging for Singularmem.
//!
//! Pure logic over `serde_json::Value`: editor/event enums, parsing an
//! editor's hook stdin JSON into a normalised [`HookInput`], building the
//! session-start output envelope, producing our hook config entries, and
//! merging/removing/inspecting them in the editor's config file. No I/O
//! except the config file read/write in [`config`]; the `singularmem`
//! binary's `hook` and `hooks` verbs are the only consumer.
//!
//! Spec: `docs/superpowers/specs/2026-09-04-hooks-wakeup-13-design.md`.

#![forbid(unsafe_code)]

pub mod config;
pub mod editor;
pub mod envelope;
pub mod error;
pub mod input;

pub use config::{
    entries, is_ours, merge, parse_ours, read_config, remove, status, write_config, HookStatus,
    MARKER,
};
pub use editor::{config_path, Editor, Event, ParseError};
pub use envelope::session_start_envelope;
pub use error::{Error, Result};
pub use input::{parse_input, HookInput};
