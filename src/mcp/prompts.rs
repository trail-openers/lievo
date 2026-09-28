use rmcp::model::{
    ErrorData as McpError, GetPromptRequestParams, GetPromptResult, ListPromptsResult, Prompt,
    PromptArgument, PromptMessage, Role,
};
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub(crate) struct DocWriterPromptParams {
    pub(crate) scope: Option<String>,
    pub(crate) style: Option<String>,
}

fn doc_writer_scope(scope: Option<&str>) -> String {
    match scope.unwrap_or("full") {
        "full" => "Scope: full. Generate a complete documentation set for the codebase.".to_string(),
        "update" => {
            "Scope: update. Update existing documentation and keep it aligned with the current codebase.".to_string()
        }
        topic => format!("Scope: {topic}. Focus the documentation on this specific topic."),
    }
}

fn doc_writer_style(style: Option<&str>) -> String {
    match style.unwrap_or("match-existing") {
        "concise" => "Style: concise. Keep sentences short and direct.".to_string(),
        "detailed" => {
            "Style: detailed. Include the extra context needed to explain the architecture clearly."
                .to_string()
        }
        _ => {
            "Style: match-existing. Match the existing project documentation style and terminology."
                .to_string()
        }
    }
}

/// Builds the full `doc-writer` prompt text. This inline string is the single
/// source of truth for the prompt (a duplicate copy once lived in
/// docs/doc-writer-skill.md and has since been deleted) — keep wording changes here.
fn build_doc_writer_prompt(scope: Option<&str>, style: Option<&str>) -> String {
    format!(
        "You are a documentation writer for a software project. You have access to lievo's MCP tools which provide deep codebase analysis. Use them to write accurate, grounded documentation.\n\n## Core Principle: Ground Every Claim\nBefore writing any factual statement about the codebase, verify it using the tools. Never rely on assumptions or world knowledge about how code \"probably\" works.\n\n## Tool Usage Order\n\n1. **Discover the project structure first**\n   - Call `list_subsystems` to understand major components\n   - Call `search_entities` with broad terms to discover key modules and files\n   - Call `get_conventions` to understand coding patterns\n\n2. **For each subsystem you will document**\n   - Call `get_module_details` for architectural context\n   - Call `list_relationships` to map dependencies (both incoming and outgoing)\n   - Call `search_entities` to find specific types, functions, and files\n   - Call `get_entity` for detailed information on key entities\n\n3. **Verify dependencies before naming them**\n   - Call `read_file` on `Cargo.toml`, `package.json`, `go.mod`, or equivalent\n   - Only name libraries you have confirmed in actual dependency files\n\n4. **Check existing documentation**\n   - Call `list_project_docs` to discover what docs already exist\n   - Call `read_project_doc` to read existing docs before updating them\n   - Match the existing style and terminology\n\n5. **For execution flows and data paths**\n   - Call `get_execution_flows` to trace how data moves through the system\n   - Call `get_insights` to understand complexity hotspots and coupling\n\n## Grounding Rules\n- NEVER state a fact about the codebase without having called a tool to verify it\n- If a tool returns empty results, say \"not found in analysis\" rather than guessing\n- Prefer tool results over user descriptions when they conflict\n- Quote actual file paths and entity names from tool results\n\n## Information Precedence\n1. Tool results (highest authority)\n2. User-provided context\n3. General software engineering knowledge (lowest authority)\n\n## Structure Guidance\nOrganize documentation as:\n- Architecture overview: what the system does, major subsystems, how they connect\n- Per-subsystem docs: purpose, key files, public API, dependencies, conventions\n- Getting started: minimal steps to understand and contribute\n- Reference: CLI commands, configuration, operational concerns\n\n## Style Guidance\n- Match existing project documentation style (check with `list_project_docs` + `read_project_doc`)\n- Use concrete examples from actual code paths\n- Prefer short sentences over long ones\n- Include file paths when referencing code locations\n\n## Scope and Style\n- Scope: {}\n- Style: {}\n",
        doc_writer_scope(scope),
        doc_writer_style(style),
    )
}

fn prompt_arguments() -> Vec<PromptArgument> {
    vec![
        PromptArgument::new("scope")
            .with_description("Optional. Use `full`, `update`, or a topic string. Defaults to `full`.")
            .with_required(false),
        PromptArgument::new("style")
            .with_description("Optional. Use `concise`, `detailed`, or `match-existing`. Defaults to `match-existing`.")
            .with_required(false),
    ]
}

pub(crate) fn doc_writer_prompt_definition() -> Prompt {
    Prompt::new(
        "doc-writer",
        Some("Guides agents to write grounded documentation using lievo's MCP tools"),
        Some(prompt_arguments()),
    )
}

pub(crate) fn doc_writer_prompt_result(args: &DocWriterPromptParams) -> GetPromptResult {
    let messages = vec![PromptMessage::new_text(
        Role::User,
        build_doc_writer_prompt(args.scope.as_deref(), args.style.as_deref()),
    )];
    GetPromptResult::new(messages).with_description(
        doc_writer_prompt_definition()
            .description
            .clone()
            .unwrap_or_default(),
    )
}

pub(crate) fn doc_writer_list_prompts() -> ListPromptsResult {
    ListPromptsResult::with_all_items(vec![doc_writer_prompt_definition()])
}

pub(crate) fn get_prompt(request: GetPromptRequestParams) -> Result<GetPromptResult, McpError> {
    if request.name != "doc-writer" {
        return Err(McpError::invalid_params("unknown prompt", None));
    }

    let args = DocWriterPromptParams {
        scope: request.arguments.as_ref().and_then(|arguments| {
            arguments
                .get("scope")
                .and_then(|value| value.as_str())
                .map(ToString::to_string)
        }),
        style: request.arguments.as_ref().and_then(|arguments| {
            arguments
                .get("style")
                .and_then(|value| value.as_str())
                .map(ToString::to_string)
        }),
    };

    Ok(doc_writer_prompt_result(&args))
}

pub(crate) fn list_prompts() -> ListPromptsResult {
    doc_writer_list_prompts()
}

#[cfg(test)]
mod tests {
    use super::{
        DocWriterPromptParams, build_doc_writer_prompt, doc_writer_list_prompts,
        doc_writer_prompt_result, get_prompt,
    };

    use rmcp::model::{ContentBlock, GetPromptRequestParams, PromptMessage, Role};

    fn text(messages: &[PromptMessage]) -> Result<&str, String> {
        messages
            .first()
            .ok_or_else(|| "expected at least one message".to_string())
            .and_then(|message| match &message.content {
                ContentBlock::Text(text) => Ok(text.text.as_str()),
                _ => Err("expected text content in prompt message".to_string()),
            })
    }

    #[test]
    fn list_prompts_returns_doc_writer_prompt_with_description() {
        let prompts = doc_writer_list_prompts().prompts;

        let prompt = prompts
            .iter()
            .find(|prompt| prompt.name == "doc-writer")
            .expect("doc-writer prompt");

        assert!(
            prompt
                .description
                .as_deref()
                .expect("description")
                .contains("grounded documentation")
        );
    }

    #[tokio::test]
    async fn get_prompt_with_no_parameters_returns_non_empty_message() {
        let messages = doc_writer_prompt_result(&DocWriterPromptParams::default()).messages;

        let text_content = text(&messages).expect("text extraction should succeed");
        assert!(!text_content.is_empty());
        assert!(matches!(messages[0].role, Role::User));
    }

    #[tokio::test]
    async fn get_prompt_with_update_scope_mentions_update() {
        let messages = doc_writer_prompt_result(&DocWriterPromptParams {
            scope: Some("update".to_string()),
            style: None,
        })
        .messages;

        let text_content = text(&messages).expect("text extraction should succeed");
        assert!(text_content.contains("update"));
    }

    #[tokio::test]
    async fn get_prompt_with_detailed_style_mentions_detailed() {
        let messages = doc_writer_prompt_result(&DocWriterPromptParams {
            scope: None,
            style: Some("detailed".to_string()),
        })
        .messages;

        let text_content = text(&messages).expect("text extraction should succeed");
        assert!(text_content.contains("detailed"));
    }

    #[test]
    fn prompt_text_includes_requested_focus() {
        let prompt = build_doc_writer_prompt(Some("api design"), Some("concise"));

        assert!(prompt.contains("api design"));
        assert!(prompt.contains("concise"));
        assert!(prompt.contains("## Structure Guidance"));
        assert!(prompt.contains("## Style Guidance"));
    }

    #[test]
    fn get_prompt_rejects_unknown_prompt_name() {
        let err = get_prompt(GetPromptRequestParams::new("unknown")).unwrap_err();

        assert!(err.to_string().contains("unknown prompt"));
    }

    #[test]
    fn text_extraction_returns_error_for_non_text_content() {
        // Create a message with non-text content by using the Image variant
        let image = ContentBlock::image("image_data", "image/png");
        let message = PromptMessage::new(Role::User, image);
        let messages = vec![message];

        let result = text(&messages);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("text content"));
    }
}
