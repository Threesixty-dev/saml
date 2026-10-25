use saml_lsp::distributed::{AgentConfig, UpdateEvent, PROTOCOL_VERSION};
use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

struct Fixture {
    root: PathBuf,
    hub: Child,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.hub.kill();
        let _ = self.hub.wait();
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn command(binary: &str, args: &[&str]) -> std::process::Output {
    let output = Command::new(binary).args(args).output().unwrap();
    assert!(
        output.status.success(),
        "{binary} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn as_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

#[test]
fn agent_sends_cloudflare_service_token_with_heartbeat() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let root = std::env::temp_dir().join(format!("saml-access-{}-{port}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let config_path = root.join("agent.json");
    AgentConfig::new(Some(format!("http://127.0.0.1:{port}")))
        .unwrap()
        .save(&config_path)
        .unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut chunk = [0; 4096];
        while !request.windows(4).any(|part| part == b"\r\n\r\n") {
            let size = stream.read(&mut chunk).unwrap();
            assert!(size > 0);
            request.extend_from_slice(&chunk[..size]);
        }
        stream
            .write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n")
            .unwrap();
        String::from_utf8(request).unwrap().to_ascii_lowercase()
    });
    let output = Command::new(env!("CARGO_BIN_EXE_saml-agent"))
        .args(["run", "--config", as_str(&config_path), "--once"])
        .env(
            "SAML_AGENT_TOKEN",
            "agent-token-with-more-than-32-characters",
        )
        .env("SAML_CF_ACCESS_CLIENT_ID", "saml-client-id")
        .env("SAML_CF_ACCESS_CLIENT_SECRET", "saml-client-secret")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let request = server.join().unwrap();
    assert!(request.contains("authorization: bearer agent-token-with-more-than-32-characters"));
    assert!(request.contains("cf-access-client-id: saml-client-id"));
    assert!(request.contains("cf-access-client-secret: saml-client-secret"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn hub_initializes_empty_credentials_on_fresh_volume() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("saml-hub-fresh-{}-{unique}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let credentials_path = root.join("data/credentials.json");
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let hub = Command::new(env!("CARGO_BIN_EXE_saml-hub"))
        .env("SAML_HUB_BIND", format!("127.0.0.1:{port}"))
        .env("SAML_HUB_STATE", root.join("data/state.json"))
        .env("SAML_HUB_CREDENTIALS", &credentials_path)
        .env("SAML_HUB_AUTH_MODE", "external")
        .env_remove("SAML_HUB_ADMIN_TOKEN")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let _fixture = Fixture {
        root: root.clone(),
        hub,
    };
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let mut ready = false;
    for _ in 0..40 {
        if client
            .get(format!("{base}/healthz"))
            .send()
            .is_ok_and(|response| response.status().is_success())
        {
            ready = true;
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert!(ready, "hub did not start with an empty data directory");
    let health: Value = client
        .get(format!("{base}/healthz"))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(health["version"], env!("CARGO_PKG_VERSION"));
    let config: Value = client
        .get(format!("{base}/api/v1/config"))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(config["auth_mode"], "external");
    let agents: Value = client
        .get(format!("{base}/api/v1/agents"))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(agents["agents"], serde_json::json!({}));
    let credentials: Value = serde_json::from_slice(&fs::read(&credentials_path).unwrap()).unwrap();
    assert_eq!(credentials["agents"], serde_json::json!({}));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&credentials_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
fn hub_receives_repository_details_and_keeps_history_compact() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("saml-runtime-{}-{unique}", std::process::id()));
    fs::create_dir_all(root.join("good")).unwrap();
    fs::create_dir_all(root.join("bad")).unwrap();
    fs::write(root.join("good/model.spec"), "concept Entity {}").unwrap();
    fs::write(root.join("bad/model.saml"), "entity Broken {").unwrap();
    let config_path = root.join("agent.json");
    let credentials_path = root.join("credentials.json");
    let state_path = root.join("hub-state.json");
    let update_dir = root.join("updates");
    fs::create_dir_all(update_dir.join("metadata")).unwrap();
    fs::create_dir_all(update_dir.join("targets/saml-agent/linux")).unwrap();
    fs::write(
        update_dir.join("metadata/1.root.json"),
        b"{\"signed\":true}",
    )
    .unwrap();
    fs::write(
        update_dir.join("targets/saml-agent/linux/release.bin"),
        b"sample release",
    )
    .unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        &credentials_path,
        update_dir.join("targets/credentials.json"),
    )
    .unwrap();
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let base = format!("http://127.0.0.1:{port}");
    let admin_token = "admin-token-with-more-than-32-characters";

    command(
        env!("CARGO_BIN_EXE_saml-agent"),
        &["init", "--config", as_str(&config_path), "--hub", &base],
    );
    command(
        env!("CARGO_BIN_EXE_saml-agent"),
        &[
            "add",
            "--config",
            as_str(&config_path),
            "--repo",
            as_str(&root.join("good")),
        ],
    );
    command(
        env!("CARGO_BIN_EXE_saml-agent"),
        &[
            "add",
            "--config",
            as_str(&config_path),
            "--repo",
            as_str(&root.join("bad")),
        ],
    );
    let agent_config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    let agent_id = agent_config["agent_id"].as_str().unwrap();
    let provision = command(
        env!("CARGO_BIN_EXE_saml-hub"),
        &["provision", agent_id, as_str(&credentials_path)],
    );
    let token = String::from_utf8(provision.stdout).unwrap();
    let token = token.trim();

    let hub = Command::new(env!("CARGO_BIN_EXE_saml-hub"))
        .env("SAML_HUB_BIND", format!("127.0.0.1:{port}"))
        .env("SAML_HUB_STATE", &state_path)
        .env("SAML_HUB_CREDENTIALS", &credentials_path)
        .env("SAML_HUB_ADMIN_TOKEN", admin_token)
        .env_remove("SAML_HUB_AUTH_MODE")
        .env("SAML_HUB_UPDATE_DIR", &update_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let _fixture = Fixture {
        root: root.clone(),
        hub,
    };
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let mut ready = false;
    for _ in 0..40 {
        if client
            .get(format!("{base}/healthz"))
            .send()
            .is_ok_and(|response| response.status().is_success())
        {
            ready = true;
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert!(ready, "hub did not start");
    let dashboard = client.get(&base).send().unwrap();
    assert_eq!(dashboard.status(), 200);
    assert!(dashboard.headers().get("content-security-policy").is_some());
    assert!(dashboard.text().unwrap().contains("SAML / OBSERVATORY"));
    let config: Value = client
        .get(format!("{base}/api/v1/config"))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(config["auth_mode"], "token");
    assert_eq!(
        client
            .get(format!("{base}/style.css"))
            .send()
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        client
            .get(format!("{base}/app.js"))
            .send()
            .unwrap()
            .status(),
        200
    );
    let metadata = client
        .get(format!("{base}/updates/metadata/1.root.json"))
        .send()
        .unwrap();
    assert_eq!(metadata.status(), 200);
    assert_eq!(metadata.bytes().unwrap(), b"{\"signed\":true}"[..]);
    let target = client
        .get(format!(
            "{base}/updates/targets/saml-agent/linux/release.bin"
        ))
        .send()
        .unwrap();
    assert_eq!(target.status(), 200);
    assert_eq!(target.bytes().unwrap(), b"sample release"[..]);
    let encoded_traversal = client
        .get(format!("{base}/updates/metadata/%2e%2e/credentials.json"))
        .send()
        .unwrap();
    assert_eq!(encoded_traversal.status(), 404);
    #[cfg(unix)]
    assert_eq!(
        client
            .get(format!("{base}/updates/targets/credentials.json"))
            .send()
            .unwrap()
            .status(),
        404
    );

    let heartbeat = AgentConfig::load(&config_path).unwrap().heartbeat();
    let rejected = client
        .post(format!("{base}/api/v1/heartbeat"))
        .bearer_auth("incorrect-agent-credential")
        .json(&heartbeat)
        .send()
        .unwrap();
    assert_eq!(rejected.status(), 401);
    assert!(!state_path.exists());

    let mut legacy = serde_json::to_value(&heartbeat).unwrap();
    for repository in legacy["repositories"].as_array_mut().unwrap() {
        repository.as_object_mut().unwrap().remove("details");
    }
    let accepted = client
        .post(format!("{base}/api/v1/heartbeat"))
        .bearer_auth(token)
        .json(&legacy)
        .send()
        .unwrap();
    assert_eq!(accepted.status(), 202);

    let output = Command::new(env!("CARGO_BIN_EXE_saml-agent"))
        .args(["run", "--config", as_str(&config_path), "--once"])
        .env("SAML_AGENT_TOKEN", token)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let unauthorized = client.get(format!("{base}/api/v1/agents")).send().unwrap();
    assert_eq!(unauthorized.status(), 401);
    let response = client
        .get(format!("{base}/api/v1/agents"))
        .bearer_auth(admin_token)
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);
    let registry: Value = response.json().unwrap();
    let heartbeat = &registry["agents"][agent_id]["heartbeat"];
    assert_eq!(heartbeat["schema_version"], 1);
    assert_eq!(heartbeat["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(heartbeat["repositories"][0]["health"], "healthy");
    assert_eq!(heartbeat["repositories"][1]["health"], "degraded");
    assert_eq!(heartbeat["repositories"][0]["details"]["name"], "good");
    assert_eq!(
        heartbeat["repositories"][0]["details"]["model"]["concept_count"],
        1
    );
    assert!(
        heartbeat["repositories"][1]["details"]["model"]["errors"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(
        heartbeat["repositories"][0]["details"]["git"]["available"],
        false
    );
    let history = &registry["agents"][agent_id]["history"];
    assert_eq!(history.as_array().unwrap().len(), 2);
    assert!(history[0]["repositories"][0].get("details").is_none());
    assert!(history[1]["repositories"][0].get("details").is_none());
    let failed = UpdateEvent {
        schema_version: PROTOCOL_VERSION,
        agent_id: agent_id.into(),
        event_id: uuid::Uuid::new_v4().to_string(),
        at_unix_secs: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        from_version: "0.3.3".into(),
        target_version: "0.3.5".into(),
        outcome: "rolled_back".into(),
        reason: Some("health_timeout".into()),
    };
    let rejected = client
        .post(format!("{base}/api/v1/update-events"))
        .bearer_auth("incorrect-agent-credential")
        .json(&failed)
        .send()
        .unwrap();
    assert_eq!(rejected.status(), 401);
    for _ in 0..2 {
        let accepted = client
            .post(format!("{base}/api/v1/update-events"))
            .bearer_auth(token)
            .json(&failed)
            .send()
            .unwrap();
        assert_eq!(accepted.status(), 202);
    }
    let mut succeeded = failed.clone();
    succeeded.event_id = uuid::Uuid::new_v4().to_string();
    succeeded.outcome = "installed".into();
    succeeded.reason = None;
    let accepted = client
        .post(format!("{base}/api/v1/update-events"))
        .bearer_auth(token)
        .json(&succeeded)
        .send()
        .unwrap();
    assert_eq!(accepted.status(), 202);
    let registry: Value = client
        .get(format!("{base}/api/v1/agents"))
        .bearer_auth(admin_token)
        .send()
        .unwrap()
        .json()
        .unwrap();
    let updates = &registry["update_events"][agent_id];
    assert_eq!(updates.as_array().unwrap().len(), 2);
    assert_eq!(updates[0]["outcome"], "rolled_back");
    assert_eq!(updates[1]["outcome"], "installed");
    let saved = fs::read_to_string(&state_path).unwrap();
    let saved_state: Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(
        saved_state["agents"][agent_id]["heartbeat"]["repositories"][0]["details"]["path"],
        agent_config["repositories"][0]["root"]
    );
    assert!(!saved.contains("concept Entity {}"));
    assert!(!saved.contains(token));
    assert!(!saved.contains(admin_token));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in [&config_path, &credentials_path, &state_path] {
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
