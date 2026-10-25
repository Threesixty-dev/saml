use saml_lsp::distributed::AgentConfig;
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

struct Fixture {
    root: PathBuf,
    config_path: PathBuf,
    child: Child,
    input: ChildStdin,
    responses: Receiver<Value>,
}

impl Fixture {
    fn new() -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("saml-mcp-{}-{unique}", std::process::id()));
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
        config
            .add_repository_with_model(&root.join("source-repo"), Some(&root.join("model-repo")))
            .unwrap();
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
                let response = serde_json::from_str(&line)
                    .unwrap_or_else(|error| panic!("non-JSON stdout: {error}: {line}"));
                if sender.send(response).is_err() {
                    break;
                }
            }
        });
        Self {
            root,
            config_path,
            child,
            input,
            responses,
        }
    }

    fn send(&mut self, value: Value) {
        writeln!(self.input, "{value}").unwrap();
        self.input.flush().unwrap();
    }

    fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let response = self
            .responses
            .recv_timeout(Duration::from_secs(10))
            .unwrap_or_else(|error| panic!("no MCP response to {method}: {error}"));
        assert_eq!(response["id"], id, "unexpected MCP response: {response}");
        assert!(response.get("error").is_none(), "MCP error: {response}");
        response["result"].clone()
    }

    fn call(&mut self, id: u64, name: &str, arguments: Value) -> Value {
        self.request(
            id,
            "tools/call",
            json!({"name": name, "arguments": arguments}),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn as_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

#[test]
fn mcp_stdio_exposes_workflow_and_model_evidence() {
    let mut fixture = Fixture::new();
    let init = fixture.request(
        1,
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "saml-test", "version": "1.0"}
        }),
    );
    assert_eq!(init["protocolVersion"], "2025-06-18");
    assert_eq!(init["serverInfo"]["name"], "saml-agent");
    assert!(init["instructions"]
        .as_str()
        .unwrap()
        .contains("get_workflow"));
    fixture.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));

    let listed = fixture.request(2, "tools/list", json!({}));
    let tools = listed["tools"].as_array().unwrap();
    for name in [
        "list_workspaces",
        "get_workflow",
        "search_model",
        "get_model_item",
        "read_model_source",
        "get_diagnostics",
        "get_semantic_map",
        "get_guide",
    ] {
        assert!(
            tools.iter().any(|tool| tool["name"] == name),
            "missing tool {name}"
        );
    }
    let search_schema = tools
        .iter()
        .find(|tool| tool["name"] == "search_model")
        .unwrap();
    assert!(search_schema["inputSchema"]["required"]
        .as_array()
        .unwrap()
        .iter()
        .any(|field| field == "workspace_id"));

    let workflow = fixture.call(3, "get_workflow", json!({}));
    assert_eq!(
        workflow["structuredContent"]["steps"][2]["tool"],
        "get_model_item"
    );
    let workspaces = fixture.call(4, "list_workspaces", json!({}));
    let workspace_id = workspaces["structuredContent"]["workspaces"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        workspaces["structuredContent"]["workspaces"][0]["model_root"],
        as_str(&fixture.root.join("model-repo").canonicalize().unwrap())
    );

    let search = fixture.call(
        5,
        "search_model",
        json!({"workspace_id": workspace_id, "query": "User"}),
    );
    assert_eq!(search["structuredContent"]["total"], 1);
    let hit = &search["structuredContent"]["matches"][0];
    assert_eq!(hit["id"], "/entity:User");
    assert_eq!(hit["next_tool"]["name"], "get_model_item");
    assert!(hit.get("detail_url").is_none());

    let detail = fixture.call(6, "get_model_item", hit["next_tool"]["arguments"].clone());
    let detail = &detail["structuredContent"];
    assert!(detail["outgoing_links"]
        .as_array()
        .unwrap()
        .iter()
        .any(|link| link["to"] == "/entity:Admin"));
    assert!(detail["incoming_links"]
        .as_array()
        .unwrap()
        .iter()
        .any(|link| link["from"] == "/entity:Admin"));
    assert_eq!(
        detail["sources"][0]["next_tool"]["name"],
        "read_model_source"
    );
    assert!(detail["sources"][0].get("url").is_none());
    let source = fixture.call(
        7,
        "read_model_source",
        detail["sources"][0]["next_tool"]["arguments"].clone(),
    );
    assert!(source["structuredContent"]["content"]
        .as_str()
        .unwrap()
        .contains("entity User"));

    let denied = fixture.call(
        8,
        "read_model_source",
        json!({
            "workspace_id": workspace_id, "uri": "file:///etc/passwd"
        }),
    );
    assert_eq!(denied["isError"], true);
    let unknown = fixture.call(
        9,
        "search_model",
        json!({"workspace_id": "unknown", "query": "User"}),
    );
    assert_eq!(unknown["isError"], true);
    let diagnostics = fixture.call(10, "get_diagnostics", json!({"workspace_id": workspace_id}));
    assert!(diagnostics["structuredContent"]
        .get("diagnostics")
        .is_some());
    let full_map = fixture.call(
        11,
        "get_semantic_map",
        json!({"workspace_id": workspace_id}),
    );
    assert_eq!(
        full_map["structuredContent"]["concepts"][0]["word"],
        "entity"
    );
    let guide = fixture.call(12, "get_guide", json!({}));
    assert!(guide["content"][0]["text"]
        .as_str()
        .unwrap()
        .starts_with("# SAML guide"));
    let concept = fixture.call(
        13,
        "get_model_item",
        json!({
            "workspace_id": workspace_id, "kind": "concept", "id": "entity"
        }),
    );
    assert_eq!(
        concept["structuredContent"]["materializations"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        concept["structuredContent"]["materializations"][0]["next_tool"]["name"],
        "get_model_item"
    );

    fs::create_dir_all(fixture.root.join("second")).unwrap();
    fs::write(fixture.root.join("second/model.spec"), "concept Event {}").unwrap();
    let mut config = AgentConfig::load(&fixture.config_path).unwrap();
    config.add_repository(&fixture.root.join("second")).unwrap();
    config.save(&fixture.config_path).unwrap();
    let refreshed = fixture.call(14, "list_workspaces", json!({}));
    assert_eq!(
        refreshed["structuredContent"]["workspaces"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}
