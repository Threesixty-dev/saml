use serde_json::Value;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

struct Fixture {
    root: PathBuf,
    process: Child,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn as_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn agent_command(args: &[&str]) {
    let output = Command::new(env!("CARGO_BIN_EXE_saml-agent"))
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn running_agent_portal_tracks_repository_changes_without_hub() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("saml-portal-{}-{unique}", std::process::id()));
    fs::create_dir_all(root.join("good")).unwrap();
    fs::create_dir_all(root.join("good-model")).unwrap();
    fs::create_dir_all(root.join("bad")).unwrap();
    fs::write(root.join("good-model/model.spec"), "concept Entity {}").unwrap();
    fs::write(root.join("bad/model.saml"), "entity Broken {").unwrap();
    let config_path = root.join("agent.json");
    agent_command(&["init", "--config", as_str(&config_path)]);
    agent_command(&[
        "add",
        "--config",
        as_str(&config_path),
        "--repo",
        as_str(&root.join("good")),
        "--model-root",
        as_str(&root.join("good-model")),
    ]);
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let base = format!("http://127.0.0.1:{port}");
    let agent = Command::new(env!("CARGO_BIN_EXE_saml-agent"))
        .args(["run", "--config", as_str(&config_path)])
        .env("SAML_AGENT_PORT", port.to_string())
        .env_remove("SAML_AGENT_TOKEN")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let _fixture = Fixture {
        root: root.clone(),
        process: agent,
    };
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let mut ready = false;
    for _ in 0..40 {
        if client
            .get(format!("{base}/api/status"))
            .send()
            .is_ok_and(|response| response.status().is_success())
        {
            ready = true;
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert!(ready, "agent portal did not start");
    let rejected_host = client
        .get(format!("{base}/api/workspaces"))
        .header("Host", "malicious.example")
        .send()
        .unwrap();
    assert_eq!(rejected_host.status(), 403);
    let status: Value = client
        .get(format!("{base}/api/status"))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(status["service"], "saml-agent");
    assert_eq!(status["telemetry_level"], "health_and_versions");
    assert_eq!(status["workspaces"], 1);
    let workspaces: Value = client
        .get(format!("{base}/api/workspaces"))
        .send()
        .unwrap()
        .json()
        .unwrap();
    let good_id = workspaces[0]["id"].as_str().unwrap();
    assert_eq!(
        workspaces[0]["path"],
        as_str(&root.join("good").canonicalize().unwrap())
    );
    let map: Value = client
        .get(format!("{base}/api/map?workspace={good_id}"))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(map["concepts"][0]["word"], "entity");

    agent_command(&[
        "add",
        "--config",
        as_str(&config_path),
        "--repo",
        as_str(&root.join("bad")),
    ]);
    let workspaces: Value = client
        .get(format!("{base}/api/workspaces"))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(workspaces.as_array().unwrap().len(), 2);
    assert_eq!(workspaces[0]["id"], good_id);
    let bad_id = workspaces[1]["id"].as_str().unwrap();
    let diagnostics: Value = client
        .get(format!("{base}/api/diagnostics?workspace={bad_id}"))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert!(diagnostics["errors"].as_u64().unwrap() > 0);
    let guide = client.get(format!("{base}/api/guide")).send().unwrap();
    assert_eq!(guide.status(), 200);

    agent_command(&["remove", "--config", as_str(&config_path), "--id", good_id]);
    let workspaces: Value = client
        .get(format!("{base}/api/workspaces"))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(workspaces.as_array().unwrap().len(), 1);
    assert_eq!(workspaces[0]["id"], bad_id);
    let removed = client
        .get(format!("{base}/api/map?workspace={good_id}"))
        .send()
        .unwrap();
    assert_eq!(removed.status(), 400);
}

#[test]
fn standalone_view_preserves_numeric_selectors() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("saml-view-{}-{unique}", std::process::id()));
    fs::create_dir_all(root.join("first")).unwrap();
    fs::create_dir_all(root.join("second")).unwrap();
    fs::write(root.join("first/model.spec"), "concept Entity {}").unwrap();
    fs::write(root.join("second/model.saml"), "entity Broken {").unwrap();
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let base = format!("http://127.0.0.1:{port}");
    let view = Command::new(env!("CARGO_BIN_EXE_saml-view"))
        .args([as_str(&root.join("first")), as_str(&root.join("second"))])
        .env("SAML_VIEW_PORT", port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let _fixture = Fixture {
        root: root.clone(),
        process: view,
    };
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let mut ready = false;
    for _ in 0..40 {
        if client
            .get(format!("{base}/api/status"))
            .send()
            .is_ok_and(|response| response.status().is_success())
        {
            ready = true;
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert!(ready, "standalone viewer did not start");
    let workspaces: Value = client
        .get(format!("{base}/api/workspaces"))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(workspaces[0]["id"], 0);
    assert_eq!(workspaces[1]["id"], 1);
    let report: Value = client
        .get(format!("{base}/api/diagnostics?workspace=1"))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert!(report["errors"].as_u64().unwrap() > 0);
}
