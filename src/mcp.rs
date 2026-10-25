use crate::diagnostics::diagnostic_report;
use crate::distributed::{AgentConfig, RegisteredRepository};
use crate::repository_info::{self, LOCAL_LIMITS};
use crate::semantic::analyze;
use crate::semantic_query;
use crate::workspace::collect_sources_checked;
use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig},
    schemars::{self, JsonSchema},
    tool, tool_handler, tool_router, ServerHandler, ServiceExt,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;

const INSTRUCTIONS: &str = "Call get_workflow first. Select a workspace with list_workspaces. For repository identity, Git state, model inventory, and diagnostics call get_repository_info; for focused Git details call get_repository_git and page through get_repository_git_history. To investigate one model piece, search_model, follow next_page, inspect the exact ID with get_model_item, read its source, and check get_diagnostics. Model data comes from saved local .spec and .saml files.";

#[derive(Clone)]
pub struct SamlMcp {
    config_path: PathBuf,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EmptyInput {}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WorkspaceInput {
    /// Persistent repository ID returned by list_workspaces.
    workspace_id: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GitHistoryInput {
    /// Persistent repository ID returned by list_workspaces.
    workspace_id: String,
    /// Zero-based commit offset. Use next_offset from the preceding page.
    offset: Option<usize>,
    /// Number of commits per page, from 1 to 100; defaults to 50.
    limit: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchInput {
    /// Persistent repository ID returned by list_workspaces.
    workspace_id: String,
    /// Model name, concept word, or property keywords, not a natural-language question.
    query: String,
    /// Opaque continuation token returned in next_cursor. Omit for the first page.
    cursor: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
enum ItemKind {
    Concept,
    Materialization,
}

impl ItemKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Concept => "concept",
            Self::Materialization => "materialization",
        }
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ItemInput {
    /// Persistent repository ID returned by list_workspaces.
    workspace_id: String,
    /// Either concept or materialization, as returned by search_model.
    kind: ItemKind,
    /// Exact ID returned by search_model; materialization IDs include scope.
    id: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SourceInput {
    /// Persistent repository ID returned by list_workspaces.
    workspace_id: String,
    /// Exact file URI from a model item definition or source entry.
    uri: String,
}

impl SamlMcp {
    pub fn new(config_path: PathBuf) -> io::Result<Self> {
        AgentConfig::load(&config_path).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "Cannot load MCP agent registry {}: {error}",
                    config_path.display()
                ),
            )
        })?;
        Ok(Self { config_path })
    }

    fn repositories(&self) -> Result<Vec<RegisteredRepository>, String> {
        AgentConfig::load(&self.config_path)
            .map(|config| config.repositories)
            .map_err(|error| format!("Cannot load agent registry: {error}"))
    }

    fn repository(&self, id: &str) -> Result<RegisteredRepository, String> {
        self.repositories()?
            .into_iter()
            .find(|repo| repo.id == id)
            .ok_or_else(|| format!("Unknown workspace ID {id:?}; call list_workspaces"))
    }

    fn sources(&self, id: &str) -> Result<HashMap<String, String>, String> {
        let repo = self.repository(id)?;
        let root = repo.model_root.unwrap_or(repo.root);
        let mut sources = HashMap::new();
        collect_sources_checked(&root, &mut sources)
            .map_err(|error| format!("Cannot read model workspace: {error}"))?;
        Ok(sources)
    }
}

#[tool_router]
impl SamlMcp {
    #[tool(
        description = "List registered model workspaces and their persistent IDs. Start here before querying."
    )]
    fn list_workspaces(&self, Parameters(_input): Parameters<EmptyInput>) -> CallToolResult {
        match self.repositories() {
            Ok(repositories) => CallToolResult::structured(json!({
                "schema_version": 1,
                "workspaces": repositories.into_iter().map(|repo| {
                    let model_root = repo.model_root.as_ref().unwrap_or(&repo.root);
                    json!({
                        "id": repo.id,
                        "name": repo.root.file_name().unwrap_or(repo.root.as_os_str()).to_string_lossy(),
                        "path": repo.root,
                        "model_root": model_root,
                    })
                }).collect::<Vec<_>>(),
            })),
            Err(error) => tool_error(error),
        }
    }

    #[tool(
        description = "Inspect one repository: identity and paths, model inventory and diagnostics summary, Git changes, branches, and recent commits. Lists are bounded; use get_semantic_map and get_diagnostics for complete model data."
    )]
    fn get_repository_info(&self, Parameters(input): Parameters<WorkspaceInput>) -> CallToolResult {
        match self.repository(&input.workspace_id) {
            Ok(repo) => {
                CallToolResult::structured(json!(repository_info::inspect(&repo, LOCAL_LIMITS)))
            }
            Err(error) => tool_error(error),
        }
    }

    #[tool(
        description = "Inspect Git state for one registered repository: worktree, branch, HEAD, changes, local and remote branches, and recent commits. Returns available=false for non-Git directories."
    )]
    fn get_repository_git(&self, Parameters(input): Parameters<WorkspaceInput>) -> CallToolResult {
        match self.repository(&input.workspace_id) {
            Ok(repo) => CallToolResult::structured(json!(repository_info::inspect_git(
                &repo.root,
                LOCAL_LIMITS
            ))),
            Err(error) => tool_error(error),
        }
    }

    #[tool(
        description = "Page through commit history for a registered repository. Start at offset 0 and follow next_offset until it is null; limit is 1 to 100."
    )]
    fn get_repository_git_history(
        &self,
        Parameters(input): Parameters<GitHistoryInput>,
    ) -> CallToolResult {
        let limit = input.limit.unwrap_or(50);
        if !(1..=100).contains(&limit) {
            return tool_error("limit must be between 1 and 100");
        }
        match self.repository(&input.workspace_id) {
            Ok(repo) => CallToolResult::structured(json!(repository_info::inspect_git_history(
                &repo.root,
                input.offset.unwrap_or(0),
                limit
            ))),
            Err(error) => tool_error(error),
        }
    }

    #[tool(
        description = "Get the step-by-step workflow for finding, inspecting, and verifying one piece of a SAML semantic model."
    )]
    fn get_workflow(&self, Parameters(_input): Parameters<EmptyInput>) -> CallToolResult {
        CallToolResult::structured(json!({
            "schema_version": 1,
            "steps": [
                {"step": 1, "tool": "list_workspaces", "instruction": "Choose one workspace ID. Reuse it for all following calls."},
                {"step": 2, "tool": "search_model", "instruction": "Extract a name, concept, or property keyword from the question. Search uses case-insensitive keyword matching, not natural-language inference. Follow next_page until the intended item is found or no page remains."},
                {"step": 3, "tool": "get_model_item", "instruction": "Pass the exact kind and ID from a search match. Inspect properties, incoming and outgoing links, bindings, and related IDs."},
                {"step": 4, "tool": "read_model_source", "instruction": "Pass a source URI from the item. Verify the saved declaration text; source spans use UTF-8 byte offsets."},
                {"step": 5, "tool": "get_diagnostics", "instruction": "Check syntax and semantic findings before relying on the answer."}
            ],
            "repository_steps": [
                {"step": 1, "tool": "list_workspaces", "instruction": "Choose one registered repository ID."},
                {"step": 2, "tool": "get_repository_info", "instruction": "Read identity, model inventory and diagnostics summary, Git state, branches, and recent commits. Lists are bounded and carry counts and truncated flags."},
                {"step": 3, "tool": "get_repository_git", "instruction": "Use for a focused Git view."},
                {"step": 4, "tool": "get_repository_git_history", "instruction": "For older commits, start at offset 0 and follow next_offset until null. Use get_semantic_map and get_diagnostics for complete model data."}
            ],
            "complete_graph_tool": "get_semantic_map",
            "guide_tool": "get_guide",
        }))
    }

    #[tool(
        description = "Search concepts and materializations by model name, kind, scope, field, or property keywords. Returns exact IDs and a next_page tool call when more matches exist."
    )]
    fn search_model(&self, Parameters(input): Parameters<SearchInput>) -> CallToolResult {
        let query = input.query.trim();
        if query.is_empty() || input.query.len() > 200 {
            return tool_error("query must contain keywords and be at most 200 bytes");
        }
        match self.sources(&input.workspace_id) {
            Ok(sources) => {
                let mut result = match semantic_query::search_with_cursor(
                    &analyze(&sources),
                    &input.workspace_id,
                    query,
                    input.cursor.as_deref(),
                ) {
                    Ok(result) => result,
                    Err(error) => return tool_error(error),
                };
                if let Some(matches) = result["matches"].as_array_mut() {
                    for entry in matches {
                        let kind = entry["kind"].clone();
                        let id = entry["id"].clone();
                        if let Some(object) = entry.as_object_mut() {
                            object.remove("detail_url");
                            object.insert("next_tool".into(), json!({
                                "name": "get_model_item",
                                "arguments": {"workspace_id": input.workspace_id, "kind": kind, "id": id},
                            }));
                        }
                    }
                }
                if let Some(cursor) = result["next_cursor"].as_str() {
                    result["next_page"] = json!({
                        "name": "search_model",
                        "arguments": {"workspace_id": input.workspace_id, "query": query, "cursor": cursor},
                    });
                }
                CallToolResult::structured(result)
            }
            Err(error) => tool_error(error),
        }
    }

    #[tool(
        description = "Inspect one exact concept or materialization ID, including relationships and source locations."
    )]
    fn get_model_item(&self, Parameters(input): Parameters<ItemInput>) -> CallToolResult {
        match self.sources(&input.workspace_id) {
            Ok(sources) => {
                let map = analyze(&sources);
                match semantic_query::item(
                    &map,
                    &input.workspace_id,
                    input.kind.as_str(),
                    &input.id,
                ) {
                    Some(mut result) => {
                        add_mcp_followups(&mut result, &input.workspace_id);
                        CallToolResult::structured(result)
                    }
                    None => tool_error(format!(
                        "Model item {:?} was not found; call search_model",
                        input.id
                    )),
                }
            }
            Err(error) => tool_error(error),
        }
    }

    #[tool(
        description = "Read a saved .spec or .saml source by an exact URI returned by get_model_item. Other files are inaccessible."
    )]
    fn read_model_source(&self, Parameters(input): Parameters<SourceInput>) -> CallToolResult {
        match self.sources(&input.workspace_id) {
            Ok(sources) => match sources.get(&input.uri) {
                Some(content) => CallToolResult::structured(json!({
                    "uri": input.uri,
                    "content": content,
                })),
                None => tool_error("Source URI is not part of the selected model workspace"),
            },
            Err(error) => tool_error(error),
        }
    }

    #[tool(description = "Get syntax and semantic diagnostics for one registered model workspace.")]
    fn get_diagnostics(&self, Parameters(input): Parameters<WorkspaceInput>) -> CallToolResult {
        match self.sources(&input.workspace_id) {
            Ok(sources) => {
                let map = analyze(&sources);
                CallToolResult::structured(json!(diagnostic_report(&sources, &map)))
            }
            Err(error) => tool_error(error),
        }
    }

    #[tool(
        description = "Get the complete semantic map when focused search and item inspection are insufficient."
    )]
    fn get_semantic_map(&self, Parameters(input): Parameters<WorkspaceInput>) -> CallToolResult {
        match self.sources(&input.workspace_id) {
            Ok(sources) => CallToolResult::structured(json!(analyze(&sources))),
            Err(error) => tool_error(error),
        }
    }

    #[tool(
        description = "Read the SAML guide for agents, including language rules and the semantic-model investigation workflow."
    )]
    fn get_guide(&self, Parameters(_input): Parameters<EmptyInput>) -> CallToolResult {
        CallToolResult::success(vec![ContentBlock::text(include_str!("../LLM_GUIDE.md"))])
    }
}

fn add_mcp_followups(result: &mut Value, workspace_id: &str) {
    for key in ["materializations", "related"] {
        if let Some(items) = result[key].as_array_mut() {
            for item in items {
                let id = item["id"].clone();
                if let Some(object) = item.as_object_mut() {
                    object.remove("detail_url");
                    object.insert("next_tool".into(), json!({
                        "name": "get_model_item",
                        "arguments": {"workspace_id": workspace_id, "kind": "materialization", "id": id},
                    }));
                }
            }
        }
    }
    if let Some(sources) = result["sources"].as_array_mut() {
        for source in sources {
            let uri = source["uri"].clone();
            if let Some(object) = source.as_object_mut() {
                object.remove("url");
                object.insert(
                    "next_tool".into(),
                    json!({
                        "name": "read_model_source",
                        "arguments": {"workspace_id": workspace_id, "uri": uri},
                    }),
                );
            }
        }
    }
}

fn tool_error(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.into())])
}

#[tool_handler]
impl ServerHandler for SamlMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("saml-agent", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }
}

pub fn serve(config_path: PathBuf) -> io::Result<()> {
    let server = SamlMcp::new(config_path)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let service = server
            .serve(rmcp::transport::stdio())
            .await
            .map_err(io::Error::other)?;
        service.waiting().await.map_err(io::Error::other)?;
        Ok(())
    })
}
