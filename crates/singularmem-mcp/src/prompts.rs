//! MCP prompts: one `wake-up` prompt wrapping `memory_wakeup`.

use rmcp::model::{GetPromptResult, Prompt, PromptArgument, PromptMessage, PromptMessageRole};

use crate::tools::wakeup::{handle_memory_wakeup, MemoryWakeupArgs};
use crate::{Config, Result};

pub const WAKE_UP: &str = "wake-up";

/// The prompt list advertised by `prompts/list`.
#[must_use]
pub fn list() -> Vec<Prompt> {
    vec![Prompt::new(
        WAKE_UP,
        Some("Recent memory for the current project, ready to paste into context"),
        Some(vec![PromptArgument::new("project")
            .with_description("Project directory; defaults to the server's --project, then its cwd")
            .with_required(false)]),
    )]
}

/// Build `prompts/get` for `wake-up`. `project` is the optional argument.
///
/// # Errors
/// Same as [`handle_memory_wakeup`].
pub fn get(config: &Config, project: Option<&str>) -> Result<GetPromptResult> {
    let args = MemoryWakeupArgs {
        project: project.map(str::to_string),
        ..MemoryWakeupArgs::default()
    };
    let out = handle_memory_wakeup(&args, config)?;
    let mut result = GetPromptResult::new(vec![PromptMessage::new_text(
        PromptMessageRole::User,
        out.text,
    )]);
    result.description = Some("Singularmem wake-up".into());
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use singularmem_core::{NewItem, Store};
    use tempfile::TempDir;

    /// Store with items under claude-code/proj-a; returns the temp root, a
    /// real directory named proj-a inside it, and a Config. Mirrors
    /// `tools::wakeup::tests::seeded`.
    fn seeded() -> (TempDir, std::path::PathBuf, Config) {
        let dir = TempDir::new().unwrap();
        let store_path = dir.path().join("store.db");
        let store = Store::open(&store_path).unwrap();
        let mut item = NewItem::text("alpha decision".to_string());
        item.scope = Some("claude-code/proj-a".to_string());
        store.ingest(item).unwrap();
        drop(store);
        let project = dir.path().join("proj-a");
        std::fs::create_dir(&project).unwrap();
        let config = Config::new(store_path, "plain".to_string(), false);
        (dir, project, config)
    }

    #[test]
    fn list_has_one_wake_up_prompt_with_an_optional_project_argument() {
        let prompts = list();
        assert_eq!(prompts.len(), 1);
        let p = &prompts[0];
        assert_eq!(p.name, WAKE_UP);
        let args = p.arguments.as_ref().expect("arguments");
        assert_eq!(args.len(), 1);
        assert_eq!(args[0].name, "project");
        assert_eq!(args[0].required, Some(false));
    }

    #[test]
    fn get_returns_one_user_message_starting_with_the_wakeup_header() {
        let (_d, project, config) = seeded();
        let result = get(&config, Some(&project.display().to_string())).unwrap();
        assert_eq!(result.messages.len(), 1);
        assert_eq!(result.messages[0].role, PromptMessageRole::User);
        let rmcp::model::PromptMessageContent::Text { text } = &result.messages[0].content else {
            panic!("expected text content: {:?}", result.messages[0].content);
        };
        assert!(text.starts_with("# Singularmem wake-up"), "{text}");
    }
}
