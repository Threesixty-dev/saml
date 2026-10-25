use saml_lsp::distributed::AgentConfig;
use serde_json::Value;
use std::fs;
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
        .env(
            "SAML_HUB_ADMIN_TOKEN",
            "admin-token-with-more-than-32-characters",
        )
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
fn hub_receives_only_version_and_isolated_repository_health() {
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
    let config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    let agent_id = config["agent_id"].as_str().unwrap();
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
    let saved = fs::read_to_string(&state_path).unwrap();
    assert!(!saved.contains(as_str(&root)));
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
