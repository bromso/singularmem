//! Server configuration assembled from clap args + env vars + built-in
//! defaults.

use std::path::PathBuf;

use singularmem_retrieve::Adapter;

/// Runtime configuration for the MCP server.
pub struct Config {
    /// Path to the `SQLite` store backing the server.
    pub store_path: PathBuf,
    /// Default adapter name when the client doesn't specify one.
    /// Must be the `name()` of one of `known_adapters`.
    pub default_adapter: String,
    /// Registered adapters available to clients. Mirrors the root
    /// binary's `known_adapters()` registry.
    pub known_adapters: Vec<Box<dyn Adapter>>,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let adapter_names: Vec<&str> = self.known_adapters.iter().map(|a| a.name()).collect();
        f.debug_struct("Config")
            .field("store_path", &self.store_path)
            .field("default_adapter", &self.default_adapter)
            .field("known_adapters", &adapter_names)
            .finish()
    }
}

impl Config {
    /// Build a config from CLI args. Adapter registry is hard-coded
    /// to the four constitutional Principle II providers.
    #[must_use]
    pub fn new(store_path: PathBuf, default_adapter: String) -> Self {
        Self {
            store_path,
            default_adapter,
            known_adapters: vec![
                Box::new(singularmem_retrieve::PlainAdapter),
                Box::new(singularmem_adapter_claude::ClaudeAdapter),
                Box::new(singularmem_adapter_openai::OpenAiAdapter),
                Box::new(singularmem_adapter_gemini::GeminiAdapter),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_new_registers_four_adapters() {
        let cfg = Config::new(PathBuf::from("/tmp/store.db"), "plain".to_string());
        let names: Vec<&str> = cfg.known_adapters.iter().map(|a| a.name()).collect();
        assert_eq!(names, vec!["plain", "claude", "openai", "gemini"]);
    }

    #[test]
    fn config_new_preserves_store_path() {
        let cfg = Config::new(PathBuf::from("/tmp/custom.db"), "claude".to_string());
        assert_eq!(cfg.store_path, PathBuf::from("/tmp/custom.db"));
    }

    #[test]
    fn config_new_preserves_default_adapter() {
        let cfg = Config::new(PathBuf::from("/tmp/store.db"), "openai".to_string());
        assert_eq!(cfg.default_adapter, "openai");
    }
}
