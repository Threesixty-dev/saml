use saml_lsp::distributed::AgentConfig;
use saml_lsp::workspace::path_to_uri;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;
use uuid::Uuid;

fn unique_root() -> PathBuf {
    std::env::temp_dir().join(format!("saml-mcp-scenario-{}", Uuid::new_v4()))
}

struct Client {
    root: PathBuf,
    config_path: PathBuf,
    child: Child,
    input: Option<ChildStdin>,
    responses: Receiver<Value>,
    next_id: u64,
}

impl Client {
    fn new(register_model: bool) -> Self {
        let root = unique_root();
        fs::create_dir_all(root.join("source-repo")).unwrap();
        fs::create_dir_all(root.join("model-repo")).unwrap();
        fs::write(
            root.join("model-repo/model.spec"),
            "concept Entity { peer { type: Entity } }",
        )
        .unwrap();
        fs::write(
            root.join("model-repo/model.saml"),
            "entity User { peer: Admin }\nentity Admin { peer: User }\n",
        )
        .unwrap();
        let config_path = root.join("agent.json");
        let mut config = AgentConfig::new(None).unwrap();
        if register_model {
            config
                .add_repository_with_model(
                    &root.join("source-repo"),
                    Some(&root.join("model-repo")),
                )
                .unwrap();
        }
        config.save(&config_path).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_saml-agent"))
            .args(["mcp", "--config", config_path.to_str().unwrap()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (sender, responses) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let line = line.unwrap();
                let value: Value = serde_json::from_str(&line)
                    .unwrap_or_else(|error| panic!("non-JSON stdout: {error}: {line}"));
                if sender.send(value).is_err() {
                    break;
                }
            }
        });
        Self {
            root,
            config_path,
            child,
            input: Some(input),
            responses,
            next_id: 0,
        }
    }

    fn ready() -> Self {
        let mut client = Self::new(true);
        client.initialize();
        client
    }

    fn ready_empty() -> Self {
        let mut client = Self::new(false);
        client.initialize();
        client
    }

    fn send(&mut self, value: Value) {
        let input = self.input.as_mut().unwrap();
        writeln!(input, "{value}").unwrap();
        input.flush().unwrap();
    }

    fn send_raw(&mut self, line: &str) {
        let input = self.input.as_mut().unwrap();
        writeln!(input, "{line}").unwrap();
        input.flush().unwrap();
    }

    fn exchange(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let response = self
            .responses
            .recv_timeout(Duration::from_secs(10))
            .unwrap_or_else(|error| panic!("no response to {method}: {error}"));
        assert_eq!(response["id"], id, "unexpected response: {response}");
        response
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let response = self.exchange(method, params);
        assert!(
            response.get("error").is_none(),
            "protocol error: {response}"
        );
        response["result"].clone()
    }

    fn initialize(&mut self) -> Value {
        self.initialize_with("2025-11-25")
    }

    fn initialize_with(&mut self, version: &str) -> Value {
        let result = self.request(
            "initialize",
            json!({
                "protocolVersion": version,
                "capabilities": {},
                "clientInfo": {"name": "saml-scenarios", "version": "1.0"}
            }),
        );
        self.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        result
    }

    fn call(&mut self, name: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({"name": name, "arguments": arguments}))
    }

    fn data(&mut self, name: &str, arguments: Value) -> Value {
        let result = self.call(name, arguments);
        assert_ne!(result["isError"], true, "tool error: {result}");
        result["structuredContent"].clone()
    }

    fn workspace_id(&mut self) -> String {
        self.data("list_workspaces", json!({}))["workspaces"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn model_path(&self, name: &str) -> PathBuf {
        self.root.join("model-repo").join(name)
    }

    fn model_uri(&self, name: &str) -> String {
        path_to_uri(&self.model_path(name).canonicalize().unwrap())
    }

    fn close_and_wait(&mut self) {
        self.input.take();
        for _ in 0..100 {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "MCP process exited with {status}");
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("MCP process did not exit after stdin closed");
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn assert_error(response: &Value) {
    assert!(
        response.get("error").is_some() || response["result"]["isError"] == true,
        "expected an MCP error: {response}"
    );
}

fn assert_json_fallback(result: &Value) {
    let fallback = result["content"][0]["text"].as_str().unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(fallback).unwrap(),
        result["structuredContent"]
    );
}

fn as_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

#[test]
fn mcp_01_invalid_registry_fails_before_protocol_output() {
    let missing = unique_root().join("missing.json");
    let output = Command::new(env!("CARGO_BIN_EXE_saml-agent"))
        .args(["mcp", "--config", as_str(&missing)])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Cannot load MCP agent registry"));
    assert!(stderr.contains("missing.json"));
    let root = unique_root();
    fs::create_dir_all(&root).unwrap();
    let malformed = root.join("invalid.json");
    fs::write(&malformed, "{").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_saml-agent"))
        .args(["mcp", "--config", as_str(&malformed)])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("invalid.json") && stderr.contains("EOF"));
    let unsupported = root.join("unsupported.json");
    let mut config = AgentConfig::new(None).unwrap();
    config.schema_version += 1;
    config.save(&unsupported).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_saml-agent"))
        .args(["mcp", "--config", as_str(&unsupported)])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported config version"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn mcp_02_initialization_lifecycle_and_eof() {
    let mut client = Client::new(true);
    let init = client.initialize();
    assert_eq!(init["protocolVersion"], "2025-11-25");
    assert_eq!(init["serverInfo"]["name"], "saml-agent");
    assert!(init["capabilities"].get("tools").is_some());
    assert!(init["instructions"]
        .as_str()
        .unwrap()
        .contains("get_workflow"));
    assert!(client.request("tools/list", json!({}))["tools"].is_array());
    client.close_and_wait();
    let mut older_client = Client::new(true);
    assert_eq!(
        older_client.initialize_with("2025-06-18")["protocolVersion"],
        "2025-06-18"
    );
    older_client.close_and_wait();
}

#[test]
fn mcp_02_modern_discovery_and_request_metadata() {
    let mut client = Client::new(true);
    let meta = json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": {"name": "saml-scenarios", "version": "1.0"}
    });
    let discovered = client.request("server/discover", json!({"_meta": meta}));
    assert_eq!(discovered["resultType"], "complete");
    assert!(discovered["supportedVersions"]
        .as_array()
        .unwrap()
        .contains(&json!("2026-07-28")));
    assert_eq!(
        discovered["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "saml-agent"
    );

    let listed = client.request("tools/list", json!({"_meta": meta}));
    assert_eq!(listed["resultType"], "complete");
    assert_eq!(listed["tools"].as_array().unwrap().len(), 11);

    let missing_meta = client.exchange("tools/list", json!({}));
    assert_eq!(missing_meta["error"]["code"], -32602);

    let workflow = client.request(
        "tools/call",
        json!({"_meta": meta, "name": "get_workflow", "arguments": {}}),
    );
    assert_eq!(workflow["resultType"], "complete");
    assert_eq!(
        workflow["structuredContent"]["steps"][0]["tool"],
        "list_workspaces"
    );
    client.close_and_wait();
}

#[test]
fn mcp_03_tool_discovery_and_schema_rejection() {
    let mut client = Client::ready();
    let listed = client.request("tools/list", json!({}));
    let tools = listed["tools"].as_array().unwrap();
    let names: BTreeSet<_> = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        BTreeSet::from([
            "get_diagnostics",
            "get_guide",
            "get_model_item",
            "get_repository_git",
            "get_repository_git_history",
            "get_repository_info",
            "get_semantic_map",
            "get_workflow",
            "list_workspaces",
            "read_model_source",
            "search_model",
        ])
    );
    for (name, required) in [
        ("search_model", vec!["workspace_id", "query"]),
        ("get_model_item", vec!["workspace_id", "kind", "id"]),
        ("read_model_source", vec!["workspace_id", "uri"]),
        ("get_diagnostics", vec!["workspace_id"]),
        ("get_repository_info", vec!["workspace_id"]),
        ("get_repository_git", vec!["workspace_id"]),
        ("get_repository_git_history", vec!["workspace_id"]),
    ] {
        let schema = &tools.iter().find(|tool| tool["name"] == name).unwrap()["inputSchema"];
        for field in required {
            assert!(schema["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item == field));
        }
        assert_eq!(schema["additionalProperties"], false);
    }
    let workspace_id = client.workspace_id();
    let rejected = client.exchange(
        "tools/call",
        json!({"name": "search_model", "arguments": {"workspace_id": workspace_id, "query": "User", "unexpected": true}}),
    );
    assert_error(&rejected);
    for name in ["list_workspaces", "get_workflow", "get_guide"] {
        let schema = &tools.iter().find(|tool| tool["name"] == name).unwrap()["inputSchema"];
        assert_eq!(schema["additionalProperties"], false);
        assert_error(&client.exchange(
            "tools/call",
            json!({"name": name, "arguments": {"unexpected": true}}),
        ));
    }
}

#[test]
fn mcp_04_workflow_is_ordered_and_followable() {
    let mut client = Client::ready();
    fs::write(client.model_path("broken.saml"), "entity Broken {}").unwrap();
    let workflow = client.data("get_workflow", json!({}));
    let names: Vec<_> = workflow["steps"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(index, step)| {
            assert_eq!(step["step"], index + 1);
            assert!(!step["instruction"].as_str().unwrap().is_empty());
            step["tool"].as_str().unwrap().to_owned()
        })
        .collect();
    assert_eq!(
        names,
        [
            "list_workspaces",
            "search_model",
            "get_model_item",
            "read_model_source",
            "get_diagnostics"
        ]
    );
    assert_eq!(workflow["complete_graph_tool"], "get_semantic_map");
    assert_eq!(workflow["guide_tool"], "get_guide");
    let workspace_id = client.workspace_id();
    let search = client.data(
        "search_model",
        json!({"workspace_id": workspace_id, "query": "User"}),
    );
    assert_eq!(search["total"], 1, "unexpected search response: {search}");
    let next = &search["matches"][0]["next_tool"];
    let detail = client.data(next["name"].as_str().unwrap(), next["arguments"].clone());
    let source = &detail["sources"][0]["next_tool"];
    let read = client.data(
        source["name"].as_str().unwrap(),
        source["arguments"].clone(),
    );
    assert!(read["content"].as_str().unwrap().contains("entity User"));
    assert_eq!(
        client.data("get_diagnostics", json!({"workspace_id": workspace_id}))["errors"],
        1
    );
}

#[test]
fn mcp_05_structured_content_text_fallback_and_tool_error() {
    let mut client = Client::ready();
    assert_json_fallback(&client.call("list_workspaces", json!({})));
    let workspace_id = client.workspace_id();
    for (name, arguments) in [
        ("get_workflow", json!({})),
        (
            "search_model",
            json!({"workspace_id": workspace_id, "query": "User"}),
        ),
        (
            "get_model_item",
            json!({"workspace_id": workspace_id, "kind": "materialization", "id": "/entity:User"}),
        ),
        (
            "read_model_source",
            json!({"workspace_id": workspace_id, "uri": client.model_uri("model.saml")}),
        ),
        ("get_diagnostics", json!({"workspace_id": workspace_id})),
        ("get_semantic_map", json!({"workspace_id": workspace_id})),
    ] {
        assert_json_fallback(&client.call(name, arguments));
    }
    let bad = client.call(
        "get_model_item",
        json!({"workspace_id": workspace_id, "kind": "concept", "id": "Missing"}),
    );
    assert_eq!(bad["isError"], true);
    assert!(bad["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("call search_model"));
}

#[test]
fn mcp_06_empty_registry_still_serves_workflow_and_guide() {
    let mut client = Client::ready_empty();
    assert_eq!(
        client.data("list_workspaces", json!({}))["workspaces"],
        json!([])
    );
    assert_eq!(
        client.data("get_workflow", json!({}))["steps"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    assert!(client.call("get_guide", json!({}))["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("# SAML guide"));
    assert_eq!(
        client.call(
            "search_model",
            json!({"workspace_id": "missing", "query": "User"})
        )["isError"],
        true
    );
}

#[test]
fn mcp_07_external_model_root_is_only_search_source() {
    let mut client = Client::ready();
    fs::write(
        client.root.join("source-repo/decoy.spec"),
        "concept Decoy {}",
    )
    .unwrap();
    let listed = client.data("list_workspaces", json!({}));
    let workspace = &listed["workspaces"][0];
    let id = workspace["id"].as_str().unwrap().to_owned();
    assert_eq!(
        workspace["path"],
        as_str(&client.root.join("source-repo").canonicalize().unwrap())
    );
    assert_eq!(
        workspace["model_root"],
        as_str(&client.root.join("model-repo").canonicalize().unwrap())
    );
    assert_eq!(client.workspace_id(), id);
    assert_eq!(
        client.data(
            "search_model",
            json!({"workspace_id": id, "query": "Decoy"})
        )["total"],
        0
    );
    assert_eq!(
        client.data("search_model", json!({"workspace_id": id, "query": "User"}))["total"],
        1
    );
    let decoy_uri = path_to_uri(
        &client
            .root
            .join("source-repo/decoy.spec")
            .canonicalize()
            .unwrap(),
    );
    assert_eq!(
        client.call(
            "read_model_source",
            json!({"workspace_id": id, "uri": decoy_uri})
        )["isError"],
        true
    );
}

#[test]
fn mcp_08_case_insensitive_search_ranks_exact_name_first() {
    let mut client = Client::ready();
    fs::write(
        client.model_path("model.saml"),
        "entity User {}\nentity UserService {}\n",
    )
    .unwrap();
    let id = client.workspace_id();
    for query in ["user", "USER"] {
        let search = client.data("search_model", json!({"workspace_id": id, "query": query}));
        assert_eq!(search["total"], 2);
        assert_eq!(search["matches"][0]["name"], "User");
        assert_eq!(
            search["matches"][0]["next_tool"]["arguments"]["id"],
            "/entity:User"
        );
        for match_item in search["matches"].as_array().unwrap() {
            let follow = &match_item["next_tool"];
            let detail = client.data(
                follow["name"].as_str().unwrap(),
                follow["arguments"].clone(),
            );
            assert_eq!(detail["item"]["id"], match_item["id"]);
        }
    }
}

#[test]
fn mcp_09_multi_term_metadata_search_is_deterministic() {
    let mut client = Client::ready();
    fs::write(
        client.model_path("model.spec"),
        "boundary Scope {}\nconcept Entity { peer { type: Entity } }",
    )
    .unwrap();
    fs::write(
        client.model_path("model.saml"),
        "scope Alpha entity User { peer: Admin }\nentity Admin {}\n",
    )
    .unwrap();
    let id = client.workspace_id();
    let scoped = client.data(
        "search_model",
        json!({"workspace_id": id, "query": "alpha peer"}),
    );
    assert_eq!(scoped["total"], 1);
    assert_eq!(scoped["matches"][0]["name"], "User");
    let metadata = client.data(
        "search_model",
        json!({"workspace_id": id, "query": "entity peer"}),
    );
    assert!(metadata["matches"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["kind"] == "concept"));
    for (query, expected_kind, expected_name) in [
        ("boundary", "concept", "scope"),
        ("peer", "concept", "entity"),
        ("Alpha", "materialization", "User"),
        ("entity", "materialization", "User"),
        ("peer", "materialization", "User"),
    ] {
        let result = client.data("search_model", json!({"workspace_id": id, "query": query}));
        assert!(result["matches"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| { item["kind"] == expected_kind && item["name"] == expected_name }));
    }
    assert_eq!(
        client.data(
            "search_model",
            json!({"workspace_id": id, "query": "Alpha Admin"})
        )["total"],
        0
    );
    assert_eq!(
        metadata,
        client.data(
            "search_model",
            json!({"workspace_id": id, "query": "entity peer"})
        )
    );
}

#[test]
fn mcp_10_no_matches_and_invalid_query_errors() {
    let mut client = Client::ready();
    let id = client.workspace_id();
    let absent = client.data(
        "search_model",
        json!({"workspace_id": id, "query": "NeverHere"}),
    );
    assert_eq!(absent["total"], 0);
    assert_eq!(absent["matches"], json!([]));
    for query in ["".to_owned(), "   ".to_owned(), "x".repeat(201)] {
        assert_eq!(
            client.call("search_model", json!({"workspace_id": id, "query": query}))["isError"],
            true
        );
    }
    for query in ["x".repeat(200), "\u{00e9}".repeat(100)] {
        assert_eq!(
            client.data("search_model", json!({"workspace_id": id, "query": query}))["total"],
            0
        );
    }
    assert_eq!(
        client.call(
            "search_model",
            json!({"workspace_id": id, "query": "\u{00e9}".repeat(101)})
        )["isError"],
        true
    );
}

#[test]
fn mcp_11_equal_names_in_scopes_have_distinct_ids() {
    let mut client = Client::ready();
    fs::write(
        client.model_path("model.spec"),
        "boundary Scope {}\nconcept Entity {}",
    )
    .unwrap();
    fs::write(
        client.model_path("model.saml"),
        "scope First entity User {}\nscope Second entity User {}\n",
    )
    .unwrap();
    let id = client.workspace_id();
    let results = client.data("search_model", json!({"workspace_id": id, "query": "User"}));
    assert_eq!(results["total"], 2);
    let hits = results["matches"].as_array().unwrap();
    assert_ne!(hits[0]["id"], hits[1]["id"]);
    let first = &hits[0]["next_tool"];
    let second = &hits[1]["next_tool"];
    let left = client.data(first["name"].as_str().unwrap(), first["arguments"].clone());
    let right = client.data(
        second["name"].as_str().unwrap(),
        second["arguments"].clone(),
    );
    assert_ne!(left["item"]["scope"], right["item"]["scope"]);
    assert_ne!(
        left["item"]["definitions"][0]["start"],
        right["item"]["definitions"][0]["start"]
    );
    for (detail, scope) in [(&left, "First"), (&right, "Second")] {
        let follow = &detail["sources"][0]["next_tool"];
        let source = client.data(
            follow["name"].as_str().unwrap(),
            follow["arguments"].clone(),
        );
        let content = source["content"].as_str().unwrap();
        let start = detail["item"]["definitions"][0]["start"].as_u64().unwrap() as usize;
        let line_start = content[..start].rfind('\n').map_or(0, |index| index + 1);
        let line_end = content[start..]
            .find('\n')
            .map_or(content.len(), |index| start + index);
        assert!(content[line_start..line_end].contains(&format!("scope {scope} entity User")));
    }
}

#[test]
fn mcp_scoped_links_resolve_within_each_scope() {
    let mut client = Client::ready();
    fs::write(
        client.model_path("model.spec"),
        "boundary Scope {}\nconcept Entity { peer { type: Entity } }",
    )
    .unwrap();
    fs::write(
        client.model_path("model.saml"),
        "scope First { entity User { peer: Friend } entity Friend { peer: User } }\nscope Second { entity User { peer: Friend } entity Friend { peer: User } }\n",
    )
    .unwrap();
    let id = client.workspace_id();
    for scope in ["First", "Second"] {
        let search = client.data(
            "search_model",
            json!({"workspace_id": id, "query": format!("{scope} User")}),
        );
        assert_eq!(search["total"], 1);
        let follow = &search["matches"][0]["next_tool"];
        let detail = client.data(
            follow["name"].as_str().unwrap(),
            follow["arguments"].clone(),
        );
        let target = detail["outgoing_links"]
            .as_array()
            .unwrap()
            .iter()
            .find(|link| link["relation"] == "property:peer")
            .unwrap()["to"]
            .as_str()
            .unwrap();
        assert!(target.contains(scope) && target.ends_with("entity:Friend"));
    }
}

#[test]
fn mcp_12_large_search_is_bounded_and_ordered() {
    let mut client = Client::ready();
    fs::write(client.model_path("model.spec"), "concept Entity {}").unwrap();
    let model = (0..51)
        .map(|index| format!("entity Item{index:03} {{}}\n"))
        .collect::<String>();
    fs::write(client.model_path("model.saml"), model).unwrap();
    let id = client.workspace_id();
    let results = client.data("search_model", json!({"workspace_id": id, "query": "Item"}));
    assert_eq!(results["total"], 51);
    let hits = results["matches"].as_array().unwrap();
    assert_eq!(hits.len(), 50);
    for (index, hit) in hits.iter().enumerate() {
        assert_eq!(hit["id"], format!("/entity:Item{index:03}"));
    }
    assert_eq!(
        results,
        client.data("search_model", json!({"workspace_id": id, "query": "Item"}))
    );
    let follow = &hits[49]["next_tool"];
    let detail = client.data(
        follow["name"].as_str().unwrap(),
        follow["arguments"].clone(),
    );
    assert_eq!(detail["item"]["id"], "/entity:Item049");
    let next_page = &results["next_page"];
    assert_eq!(next_page["name"], "search_model");
    assert_eq!(next_page["arguments"]["cursor"], results["next_cursor"]);
    let last = client.data(
        next_page["name"].as_str().unwrap(),
        next_page["arguments"].clone(),
    );
    assert_eq!(last["total"], 51);
    assert_eq!(last["matches"].as_array().unwrap().len(), 1);
    assert_eq!(last["matches"][0]["id"], "/entity:Item050");
    assert!(last.get("next_cursor").is_none());
    let follow = &last["matches"][0]["next_tool"];
    let detail = client.data(
        follow["name"].as_str().unwrap(),
        follow["arguments"].clone(),
    );
    assert_eq!(detail["item"]["id"], "/entity:Item050");
}

#[test]
fn mcp_search_cursor_rejects_wrong_query_and_changed_results() {
    let mut client = Client::ready();
    fs::write(client.model_path("model.spec"), "concept Entity {}").unwrap();
    let model = (0..51)
        .map(|index| format!("entity Item{index:03} {{}}\n"))
        .collect::<String>();
    fs::write(client.model_path("model.saml"), &model).unwrap();
    let workspace_id = client.workspace_id();
    let first = client.data(
        "search_model",
        json!({"workspace_id": workspace_id, "query": "Item"}),
    );
    let cursor = first["next_cursor"].as_str().unwrap();
    assert_eq!(
        client.call(
            "search_model",
            json!({"workspace_id": workspace_id, "query": "Entity", "cursor": cursor})
        )["isError"],
        true
    );
    assert_eq!(
        client.call(
            "search_model",
            json!({"workspace_id": workspace_id, "query": "Item", "cursor": "invalid"})
        )["isError"],
        true
    );
    assert_eq!(
        client.call(
            "search_model",
            json!({"workspace_id": workspace_id, "query": "Item", "cursor": "x".repeat(100)})
        )["isError"],
        true
    );
    let second = client.root.join("second-repo");
    fs::create_dir_all(&second).unwrap();
    fs::write(second.join("model.spec"), "concept Entity {}").unwrap();
    fs::write(second.join("model.saml"), &model).unwrap();
    let mut config = AgentConfig::load(&client.config_path).unwrap();
    let second_id = config.add_repository(&second).unwrap();
    config.save(&client.config_path).unwrap();
    assert_eq!(
        client.call(
            "search_model",
            json!({"workspace_id": second_id, "query": "Item", "cursor": cursor})
        )["isError"],
        true
    );
    fs::write(
        client.model_path("model.saml"),
        format!("{model}entity Item999 {{}}\n"),
    )
    .unwrap();
    let stale = client.call(
        "search_model",
        json!({"workspace_id": workspace_id, "query": "Item", "cursor": cursor}),
    );
    assert_eq!(stale["isError"], true);
    assert!(stale["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("restart the search"));
}

#[test]
fn mcp_search_continues_across_three_pages() {
    let mut client = Client::ready();
    fs::write(client.model_path("model.spec"), "concept Entity {}").unwrap();
    let model = (0..101)
        .map(|index| format!("entity Item{index:03} {{}}\n"))
        .collect::<String>();
    fs::write(client.model_path("model.saml"), model).unwrap();
    let workspace_id = client.workspace_id();
    let mut page = client.data(
        "search_model",
        json!({"workspace_id": workspace_id, "query": "Item"}),
    );
    let mut ids = Vec::new();
    let mut pages = 0;
    loop {
        pages += 1;
        assert!(pages <= 3, "search repeated a page");
        ids.extend(
            page["matches"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["id"].as_str().unwrap().to_owned()),
        );
        let Some(next_page) = page.get("next_page") else {
            break;
        };
        page = client.data(
            next_page["name"].as_str().unwrap(),
            next_page["arguments"].clone(),
        );
    }
    assert_eq!(pages, 3);
    assert_eq!(ids.len(), 101);
    for (index, id) in ids.iter().enumerate() {
        assert_eq!(id, &format!("/entity:Item{index:03}"));
    }
}

#[test]
fn mcp_13_concept_detail_has_properties_materializations_and_sources() {
    let mut client = Client::ready();
    fs::write(
        client.model_path("extra.spec"),
        "concept Entity { friend { type: Entity } }",
    )
    .unwrap();
    let id = client.workspace_id();
    let detail = client.data(
        "get_model_item",
        json!({"workspace_id": id, "kind": "concept", "id": "entity"}),
    );
    assert_eq!(detail["item"]["word"], "entity");
    assert_eq!(detail["item"]["roles"], json!(["concept"]));
    assert!(detail["item"]["properties"]
        .as_array()
        .unwrap()
        .iter()
        .any(|property| property["name"] == "peer"));
    assert!(detail["item"]["properties"]
        .as_array()
        .unwrap()
        .iter()
        .any(|property| property["name"] == "friend"));
    assert_eq!(detail["materializations"].as_array().unwrap().len(), 2);
    assert_eq!(detail["item"]["definitions"].as_array().unwrap().len(), 2);
    for entry in detail["materializations"].as_array().unwrap() {
        let follow = &entry["next_tool"];
        let item = client.data(
            follow["name"].as_str().unwrap(),
            follow["arguments"].clone(),
        );
        assert_eq!(item["item"]["id"], entry["id"]);
    }
    for entry in detail["sources"].as_array().unwrap() {
        let follow = &entry["next_tool"];
        assert!(client.data(
            follow["name"].as_str().unwrap(),
            follow["arguments"].clone()
        )["content"]
            .is_string());
    }
}

#[test]
fn mcp_14_materialization_links_bindings_and_related_items() {
    let mut client = Client::ready();
    fs::write(client.model_path("model.spec"), "concept Data {}\nconcept ApplicationCapability {}\nconcept Application { capability { type: ApplicationCapability } }").unwrap();
    fs::write(client.model_path("model.saml"), "data Article {}\ndata Author {}\napplicationCapability RestApi { expose: { type: [Data] } }\napplication Portal { capability: RestApi { expose: [Article, Author] } }").unwrap();
    let id = client.workspace_id();
    let portal = client.data(
        "get_model_item",
        json!({"workspace_id": id, "kind": "materialization", "id": "/application:Portal"}),
    );
    assert!(portal["outgoing_links"]
        .as_array()
        .unwrap()
        .iter()
        .any(|link| link["to"] == "/applicationCapability:RestApi"));
    assert_eq!(portal["bindings"].as_array().unwrap().len(), 1);
    let binding = &portal["bindings"][0];
    assert_eq!(binding["from"], "/application:Portal");
    assert_eq!(binding["target"], "/applicationCapability:RestApi");
    assert_eq!(binding["arguments"][0]["name"], "expose");
    let targets: BTreeSet<_> = binding["arguments"][0]["values"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value["target"].as_str().unwrap())
        .collect();
    assert_eq!(targets, BTreeSet::from(["/data:Article", "/data:Author"]));
    let source = client.data(
        "read_model_source",
        json!({"workspace_id": id, "uri": binding["source"]["uri"]}),
    );
    let content = source["content"].as_str().unwrap();
    let start = binding["source"]["start"].as_u64().unwrap() as usize;
    let end = binding["source"]["end"].as_u64().unwrap() as usize;
    assert_eq!(&content[start..end], "RestApi");
    let map = client.data("get_semantic_map", json!({"workspace_id": id}));
    assert!(map["bindings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item == binding));
    let rest = client.data("get_model_item", json!({"workspace_id": id, "kind": "materialization", "id": "/applicationCapability:RestApi"}));
    assert!(rest["incoming_links"]
        .as_array()
        .unwrap()
        .iter()
        .any(|link| link["from"] == "/application:Portal"));
    assert!(!portal["related"].as_array().unwrap().is_empty());
    for related in portal["related"].as_array().unwrap() {
        let follow = &related["next_tool"];
        let item = client.data(
            follow["name"].as_str().unwrap(),
            follow["arguments"].clone(),
        );
        assert_eq!(item["item"]["id"], related["id"]);
    }
}

#[test]
fn mcp_15_source_span_offsets_select_utf8_name() {
    let mut client = Client::ready();
    fs::write(
        client.model_path("model.saml"),
        "entity Caf\u{00e9} { peer: Admin }\nentity Admin {}\n",
    )
    .unwrap();
    let id = client.workspace_id();
    let results = client.data(
        "search_model",
        json!({"workspace_id": id, "query": "Caf\u{00e9}"}),
    );
    assert_eq!(results["total"], 1);
    let follow = &results["matches"][0]["next_tool"];
    let detail = client.data(
        follow["name"].as_str().unwrap(),
        follow["arguments"].clone(),
    );
    let span = &detail["item"]["definitions"][0];
    let source_follow = &detail["sources"][0]["next_tool"];
    let source = client.data(
        source_follow["name"].as_str().unwrap(),
        source_follow["arguments"].clone(),
    );
    assert_eq!(span["uri"], source["uri"]);
    let content = source["content"].as_str().unwrap();
    let start = span["start"].as_u64().unwrap() as usize;
    let end = span["end"].as_u64().unwrap() as usize;
    assert_eq!(&content[start..end], "Caf\u{00e9}");
    let admin = client.data(
        "get_model_item",
        json!({"workspace_id": id, "kind": "materialization", "id": "/entity:Admin"}),
    );
    let admin_span = &admin["item"]["definitions"][0];
    let start = admin_span["start"].as_u64().unwrap() as usize;
    let end = admin_span["end"].as_u64().unwrap() as usize;
    assert_eq!(&content[start..end], "Admin");
}

#[test]
fn mcp_16_diagnostics_report_broken_model_without_blocking_reads() {
    let mut client = Client::ready();
    fs::write(client.model_path("model.saml"), "entity Broken {}").unwrap();
    let id = client.workspace_id();
    let report = client.data("get_diagnostics", json!({"workspace_id": id}));
    assert_eq!(report["errors"], 1);
    assert!(report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| {
            item["uri"].as_str().unwrap().ends_with("model.saml")
                && item["severity"] == "error"
                && item["code"] == "missing-required-property"
                && item["range"]["start"]["line"] == 0
        }));
    assert!(
        client.data(
            "search_model",
            json!({"workspace_id": id, "query": "entity"})
        )["total"]
            .as_u64()
            .unwrap()
            > 0
    );
    let source = client.data(
        "read_model_source",
        json!({"workspace_id": id, "uri": client.model_uri("model.saml")}),
    );
    assert_eq!(source["content"], "entity Broken {}");
    fs::write(
        client.model_path("model.saml"),
        "entity Broken { peer: Broken }",
    )
    .unwrap();
    let fixed = client.data("get_diagnostics", json!({"workspace_id": id}));
    assert_eq!(fixed["errors"], 0);
    assert_eq!(fixed["diagnostics"], json!([]));
}

#[test]
fn mcp_17_full_map_agrees_with_focused_tools_and_guide() {
    let mut client = Client::ready();
    let id = client.workspace_id();
    let search = client.data("search_model", json!({"workspace_id": id, "query": "User"}));
    let target = search["matches"][0]["id"].clone();
    let map = client.data("get_semantic_map", json!({"workspace_id": id}));
    assert!(map["concepts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["word"] == "entity"));
    assert!(map["materializations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["id"] == target));
    assert!(map["links"]
        .as_array()
        .unwrap()
        .iter()
        .any(|link| link["from"] == target && link["to"] == "/entity:Admin"));
    for concept in map["concepts"].as_array().unwrap() {
        let detail = client.data(
            "get_model_item",
            json!({"workspace_id": id, "kind": "concept", "id": concept["word"]}),
        );
        assert_eq!(detail["item"], *concept);
    }
    for materialization in map["materializations"].as_array().unwrap() {
        let item_id = &materialization["id"];
        let detail = client.data(
            "get_model_item",
            json!({"workspace_id": id, "kind": "materialization", "id": item_id}),
        );
        assert_eq!(detail["item"], *materialization);
        let outgoing: Vec<_> = map["links"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|link| link["from"] == *item_id)
            .cloned()
            .collect();
        let incoming: Vec<_> = map["links"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|link| link["to"] == *item_id)
            .cloned()
            .collect();
        assert_eq!(detail["outgoing_links"], json!(outgoing));
        assert_eq!(detail["incoming_links"], json!(incoming));
    }
    let guide = client.call("get_guide", json!({}));
    let guide_text = guide["content"][0]["text"].as_str().unwrap();
    assert!(guide_text.starts_with("# SAML guide"));
    assert!(guide_text.contains("search_model") && guide_text.contains("get_diagnostics"));
}

#[test]
fn mcp_18_invalid_arguments_and_ids_do_not_stop_server() {
    let mut client = Client::ready();
    let id = client.workspace_id();
    assert_error(&client.exchange("unknown/method", json!({})));
    for arguments in [
        json!({"workspace_id": id}),
        json!({"workspace_id": id, "query": "User", "extra": 1}),
        json!({"workspace_id": id, "query": 42}),
    ] {
        assert_error(&client.exchange(
            "tools/call",
            json!({"name": "search_model", "arguments": arguments}),
        ));
    }
    assert_error(&client.exchange("tools/call", json!({"name": "get_model_item", "arguments": {"workspace_id": id, "kind": "wrong", "id": "entity"}})));
    assert_eq!(
        client.call(
            "search_model",
            json!({"workspace_id": "unknown", "query": "User"})
        )["isError"],
        true
    );
    assert_eq!(
        client.call(
            "get_model_item",
            json!({"workspace_id": id, "kind": "concept", "id": "Missing"})
        )["isError"],
        true
    );
    client.send_raw("{not json");
    if let Ok(malformed) = client.responses.recv_timeout(Duration::from_millis(500)) {
        assert!(malformed.get("error").is_some(), "{malformed}");
    }
    assert_eq!(
        client.data("search_model", json!({"workspace_id": id, "query": "User"}))["total"],
        1
    );
}

#[test]
fn mcp_19_source_reads_are_isolated_to_selected_workspace() {
    let mut client = Client::ready();
    let other = client.root.join("other-repo");
    fs::create_dir_all(&other).unwrap();
    fs::write(other.join("model.spec"), "concept Foreign {}").unwrap();
    fs::write(client.model_path("notes.txt"), "not a model source").unwrap();
    let mut config = AgentConfig::load(&client.config_path).unwrap();
    config.add_repository(&other).unwrap();
    config.save(&client.config_path).unwrap();
    let id = client.workspace_id();
    for uri in [
        path_to_uri(&other.join("model.spec").canonicalize().unwrap()),
        "file:///etc/passwd".to_owned(),
        client.model_uri("notes.txt"),
    ] {
        assert_eq!(
            client.call("read_model_source", json!({"workspace_id": id, "uri": uri}))["isError"],
            true
        );
    }
    let valid = client.data(
        "read_model_source",
        json!({"workspace_id": id, "uri": client.model_uri("model.spec")}),
    );
    assert!(valid["content"]
        .as_str()
        .unwrap()
        .contains("concept Entity"));
    let valid_uri = client.model_uri("model.spec");
    for alias in [
        valid_uri.replacen("file://", "file://localhost", 1),
        valid_uri.replacen("model.spec", "%6Dodel.spec", 1),
    ] {
        assert_eq!(
            client.call(
                "read_model_source",
                json!({"workspace_id": id, "uri": alias})
            )["isError"],
            true
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let outside = client.root.join("outside.spec");
        fs::write(&outside, "concept Secret {}").unwrap();
        fs::remove_file(client.model_path("model.spec")).unwrap();
        symlink(&outside, client.model_path("model.spec")).unwrap();
        assert_eq!(
            client.call(
                "read_model_source",
                json!({"workspace_id": id, "uri": valid_uri})
            )["isError"],
            true
        );
    }
}

#[test]
fn mcp_20_registry_and_saved_model_refresh_without_restart() {
    let mut client = Client::ready();
    let id = client.workspace_id();
    fs::write(
        client.model_path("model.saml"),
        "entity User {}\nentity Fresh {}\n",
    )
    .unwrap();
    assert_eq!(
        client.data(
            "search_model",
            json!({"workspace_id": id, "query": "Fresh"})
        )["total"],
        1
    );
    let second = client.root.join("second-repo");
    fs::create_dir_all(&second).unwrap();
    fs::write(second.join("model.spec"), "concept Event {}").unwrap();
    fs::write(second.join("model.saml"), "event New {}").unwrap();
    let mut config = AgentConfig::load(&client.config_path).unwrap();
    let second_id = config.add_repository(&second).unwrap();
    config.save(&client.config_path).unwrap();
    assert_eq!(
        client.data("list_workspaces", json!({}))["workspaces"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        client.data(
            "search_model",
            json!({"workspace_id": second_id, "query": "New"})
        )["total"],
        1
    );
    let removed_uri = client.model_uri("model.saml");
    fs::remove_file(client.model_path("model.saml")).unwrap();
    assert_eq!(
        client.data(
            "search_model",
            json!({"workspace_id": id, "query": "Fresh"})
        )["total"],
        0
    );
    assert_eq!(
        client.call(
            "read_model_source",
            json!({"workspace_id": id, "uri": removed_uri})
        )["isError"],
        true
    );
    let mut config = AgentConfig::load(&client.config_path).unwrap();
    config.repositories.retain(|repo| repo.id != second_id);
    config.save(&client.config_path).unwrap();
    assert_eq!(
        client.data("list_workspaces", json!({}))["workspaces"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        client.call(
            "search_model",
            json!({"workspace_id": second_id, "query": "New"})
        )["isError"],
        true
    );
    client.close_and_wait();
}
