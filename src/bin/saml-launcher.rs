use atomic_write_file::AtomicWriteFile;
use saml_lsp::distributed::{AgentConfig, UpdateEvent, PROTOCOL_VERSION};
use saml_lsp::update::{
    platform, verify_manifest, verify_target, MAX_MANIFEST_BYTES, MAX_TARGET_BYTES, ROOT_PUBLIC_KEY,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Deserialize, Serialize)]
struct ActiveAgent {
    version: String,
    binary: PathBuf,
}

#[derive(Clone, Default, Deserialize, Serialize)]
struct UpdateEventLog {
    pending: Vec<UpdateEvent>,
    last: Option<UpdateEvent>,
}

struct CheckFailure {
    target_version: Option<String>,
    reason: &'static str,
    detail: String,
}

struct UpdateResult<'a> {
    from_version: &'a str,
    target_version: &'a str,
    outcome: &'a str,
    reason: Option<&'a str>,
}

impl CheckFailure {
    fn new(error: impl std::fmt::Display, reason: &'static str, version: Option<&Version>) -> Self {
        Self {
            target_version: version.map(ToString::to_string),
            reason,
            detail: error.to_string(),
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let mut config = None;
    let mut bootstrap = None;
    let mut state_dir = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => config = Some(PathBuf::from(args.next().ok_or("missing --config")?)),
            "--agent" => bootstrap = Some(PathBuf::from(args.next().ok_or("missing --agent")?)),
            "--state-dir" => {
                state_dir = Some(PathBuf::from(args.next().ok_or("missing --state-dir")?))
            }
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }
    let config = config.ok_or("--config is required")?.canonicalize()?;
    let bootstrap = bootstrap.ok_or("--agent is required")?.canonicalize()?;
    let state_dir = state_dir.ok_or("--state-dir is required")?;
    fs::create_dir_all(&state_dir)?;
    let state_dir = state_dir.canonicalize()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&state_dir, fs::Permissions::from_mode(0o700))?;
    }
    let state_path = state_dir.join("active.json");
    let events_path = state_dir.join("update-events.json");
    let agent_id = AgentConfig::load(&config)?.agent_id;
    let mut events: UpdateEventLog = if events_path.exists() {
        serde_json::from_slice(&fs::read(&events_path)?)?
    } else {
        UpdateEventLog::default()
    };
    let mut active = if state_path.exists() {
        let stored: ActiveAgent = serde_json::from_slice(&fs::read(&state_path)?)?;
        let expected = agent_binary_path(&state_dir, &stored.version);
        if stored.binary != expected || !expected.is_file() {
            return Err("invalid active agent state".into());
        }
        Version::parse(&stored.version)?;
        stored
    } else {
        let version = env!("CARGO_PKG_VERSION");
        let binary = agent_binary_path(&state_dir, version);
        fs::copy(&bootstrap, &binary)?;
        make_executable(&binary)?;
        let active = ActiveAgent {
            version: version.into(),
            binary,
        };
        save_state(&state_path, &active)?;
        active
    };
    let interval = env::var("SAML_UPDATE_POLL_SECS")
        .ok()
        .map(|value| value.parse::<u64>())
        .transpose()?
        .unwrap_or(60)
        .max(5);
    let port = env::var("SAML_AGENT_PORT")
        .ok()
        .map(|value| value.parse::<u16>())
        .transpose()?
        .unwrap_or(8787);
    let client = reqwest::blocking::Client::builder()
        .user_agent(format!("saml-launcher/{}", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .build()?;
    let mut child = spawn_agent(&active.binary, &config)?;
    wait_for_health(&client, &mut child, &active.version, port)?;
    println!("SAML launcher supervising agent {}", active.version);
    let mut failed_version = None;
    loop {
        if let Err(error) = flush_events(&client, &config, &events_path, &mut events) {
            eprintln!("Cannot report update outcome: {error}");
        }
        if child.try_wait()?.is_some() {
            eprintln!("Agent exited; restarting {}", active.version);
            child = spawn_agent(&active.binary, &config)?;
        }
        match check_update(&client, &config, &active, &state_dir) {
            Ok(Some((version, binary))) if failed_version.as_ref() != Some(&version) => {
                let previous = ActiveAgent {
                    version: active.version.clone(),
                    binary: active.binary.clone(),
                };
                if let Err(error) = stop_agent(&mut child) {
                    eprintln!("Cannot stop agent for update {version}: {error}");
                    report_event(
                        &client,
                        &config,
                        &events_path,
                        &mut events,
                        &agent_id,
                        UpdateResult {
                            from_version: &previous.version,
                            target_version: &version,
                            outcome: "install_failed",
                            reason: Some("agent_stop_failed"),
                        },
                    );
                    failed_version = Some(version);
                    thread::sleep(Duration::from_secs(interval));
                    continue;
                }
                match spawn_agent(&binary, &config).and_then(|mut candidate| match wait_for_health(
                    &client,
                    &mut candidate,
                    &version,
                    port,
                ) {
                    Ok(()) => Ok(candidate),
                    Err(error) => {
                        let _ = stop_agent(&mut candidate);
                        Err(error)
                    }
                }) {
                    Ok(candidate) => {
                        active = ActiveAgent {
                            version: version.clone(),
                            binary,
                        };
                        if let Err(error) = save_state(&state_path, &active) {
                            eprintln!("Cannot save updated agent state: {error}");
                            child = candidate;
                            if let Err(stop_error) = stop_agent(&mut child) {
                                report_event(
                                    &client,
                                    &config,
                                    &events_path,
                                    &mut events,
                                    &agent_id,
                                    UpdateResult {
                                        from_version: &previous.version,
                                        target_version: &version,
                                        outcome: "rollback_failed",
                                        reason: Some("restore_failed"),
                                    },
                                );
                                return Err(stop_error.into());
                            }
                            active = previous;
                            match restore_agent(&client, &active, &config, port) {
                                Ok(restored) => {
                                    child = restored;
                                    report_event(
                                        &client,
                                        &config,
                                        &events_path,
                                        &mut events,
                                        &agent_id,
                                        UpdateResult {
                                            from_version: &active.version,
                                            target_version: &version,
                                            outcome: "rolled_back",
                                            reason: Some("state_save_failed"),
                                        },
                                    );
                                }
                                Err(restore_error) => {
                                    report_event(
                                        &client,
                                        &config,
                                        &events_path,
                                        &mut events,
                                        &agent_id,
                                        UpdateResult {
                                            from_version: &active.version,
                                            target_version: &version,
                                            outcome: "rollback_failed",
                                            reason: Some("restore_failed"),
                                        },
                                    );
                                    return Err(restore_error.into());
                                }
                            }
                            failed_version = Some(version);
                        } else {
                            child = candidate;
                            failed_version = None;
                            println!("SAML agent updated to {}", active.version);
                            report_event(
                                &client,
                                &config,
                                &events_path,
                                &mut events,
                                &agent_id,
                                UpdateResult {
                                    from_version: &previous.version,
                                    target_version: &active.version,
                                    outcome: "installed",
                                    reason: None,
                                },
                            );
                        }
                    }
                    Err(error) => {
                        eprintln!("Update {version} failed health check; restoring previous agent: {error}");
                        let reason = if error.to_string() == "agent health check timed out" {
                            "health_timeout"
                        } else {
                            "agent_exited"
                        };
                        match restore_agent(&client, &previous, &config, port) {
                            Ok(restored) => {
                                child = restored;
                                report_event(
                                    &client,
                                    &config,
                                    &events_path,
                                    &mut events,
                                    &agent_id,
                                    UpdateResult {
                                        from_version: &previous.version,
                                        target_version: &version,
                                        outcome: "rolled_back",
                                        reason: Some(reason),
                                    },
                                );
                                failed_version = Some(version);
                            }
                            Err(restore_error) => {
                                report_event(
                                    &client,
                                    &config,
                                    &events_path,
                                    &mut events,
                                    &agent_id,
                                    UpdateResult {
                                        from_version: &previous.version,
                                        target_version: &version,
                                        outcome: "rollback_failed",
                                        reason: Some("restore_failed"),
                                    },
                                );
                                return Err(restore_error.into());
                            }
                        }
                    }
                }
            }
            Ok(_) => {}
            Err(error) => {
                eprintln!("Update check failed: {}", error.detail);
                if let Some(version) = error.target_version {
                    report_event(
                        &client,
                        &config,
                        &events_path,
                        &mut events,
                        &agent_id,
                        UpdateResult {
                            from_version: &active.version,
                            target_version: &version,
                            outcome: "download_failed",
                            reason: Some(error.reason),
                        },
                    );
                }
            }
        }
        thread::sleep(Duration::from_secs(interval));
    }
}

fn check_update(
    client: &reqwest::blocking::Client,
    config_path: &Path,
    active: &ActiveAgent,
    state_dir: &Path,
) -> Result<Option<(String, PathBuf)>, CheckFailure> {
    let config = AgentConfig::load(config_path)
        .map_err(|error| CheckFailure::new(error, "config_invalid", None))?;
    let Some(hub) = config.hub_url.as_deref() else {
        return Ok(None);
    };
    let base = hub.trim_end_matches('/');
    let manifest = fetch(
        client,
        &format!("{base}/updates/metadata/release.json"),
        MAX_MANIFEST_BYTES as u64,
    )
    .map_err(|error| CheckFailure::new(error, "manifest_fetch_failed", None))?;
    let signature = fetch(
        client,
        &format!("{base}/updates/metadata/release.sig"),
        1024,
    )
    .map_err(|error| CheckFailure::new(error, "signature_fetch_failed", None))?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| CheckFailure::new(error, "clock_invalid", None))?
        .as_secs();
    let current = Version::parse(&active.version)
        .map_err(|error| CheckFailure::new(error, "active_version_invalid", None))?;
    let Some((version, target)) = verify_manifest(
        &manifest,
        &signature,
        ROOT_PUBLIC_KEY,
        now,
        &current,
        &platform(),
    )
    .map_err(|error| CheckFailure::new(error, "manifest_invalid", None))?
    else {
        return Ok(None);
    };
    let path = agent_binary_path(state_dir, &version.to_string());
    let cached_valid = fs::read(&path)
        .ok()
        .is_some_and(|bytes| verify_target(&bytes, &target).is_ok());
    if !cached_valid {
        if path.exists() {
            fs::remove_file(&path)
                .map_err(|error| CheckFailure::new(error, "target_stage_failed", Some(&version)))?;
        }
        let bytes = fetch(
            client,
            &format!("{base}/updates/targets/{}", target.path),
            MAX_TARGET_BYTES,
        )
        .map_err(|error| CheckFailure::new(error, "target_fetch_failed", Some(&version)))?;
        verify_target(&bytes, &target)
            .map_err(|error| CheckFailure::new(error, "target_invalid", Some(&version)))?;
        let temp = state_dir.join(format!(".saml-agent-{version}-{}.tmp", Uuid::new_v4()));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|error| CheckFailure::new(error, "target_stage_failed", Some(&version)))?;
        file.write_all(&bytes)
            .map_err(|error| CheckFailure::new(error, "target_stage_failed", Some(&version)))?;
        file.sync_all()
            .map_err(|error| CheckFailure::new(error, "target_stage_failed", Some(&version)))?;
        make_executable(&temp)
            .map_err(|error| CheckFailure::new(error, "target_stage_failed", Some(&version)))?;
        fs::rename(temp, &path)
            .map_err(|error| CheckFailure::new(error, "target_stage_failed", Some(&version)))?;
    }
    Ok(Some((version.to_string(), path)))
}

fn report_event(
    client: &reqwest::blocking::Client,
    config_path: &Path,
    events_path: &Path,
    events: &mut UpdateEventLog,
    agent_id: &str,
    result: UpdateResult<'_>,
) {
    if let Err(error) = queue_event(
        events_path,
        events,
        agent_id,
        result.from_version,
        result.target_version,
        result.outcome,
        result.reason,
    ) {
        eprintln!("Cannot save update outcome: {error}");
        return;
    }
    if let Err(error) = flush_events(client, config_path, events_path, events) {
        eprintln!("Cannot report update outcome: {error}");
    }
}

fn queue_event(
    path: &Path,
    events: &mut UpdateEventLog,
    agent_id: &str,
    from_version: &str,
    target_version: &str,
    outcome: &str,
    reason: Option<&str>,
) -> io::Result<()> {
    if outcome != "installed"
        && events.last.as_ref().is_some_and(|last| {
            last.from_version == from_version
                && last.target_version == target_version
                && last.outcome == outcome
                && last.reason.as_deref() == reason
        })
    {
        return Ok(());
    }
    let mut next = events.clone();
    let event = UpdateEvent {
        schema_version: PROTOCOL_VERSION,
        agent_id: agent_id.into(),
        event_id: Uuid::new_v4().to_string(),
        at_unix_secs: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_secs(),
        from_version: from_version.into(),
        target_version: target_version.into(),
        outcome: outcome.into(),
        reason: reason.map(str::to_owned),
    };
    next.last = Some(event.clone());
    next.pending.push(event);
    if next.pending.len() > 100 {
        next.pending.drain(..next.pending.len() - 100);
    }
    save_json(path, &next)?;
    *events = next;
    Ok(())
}

fn flush_events(
    client: &reqwest::blocking::Client,
    config_path: &Path,
    events_path: &Path,
    events: &mut UpdateEventLog,
) -> io::Result<()> {
    if events.pending.is_empty() {
        return Ok(());
    }
    let Ok(token) = env::var("SAML_AGENT_TOKEN") else {
        return Ok(());
    };
    if token.len() < 32 {
        return Err(io::Error::other("invalid SAML_AGENT_TOKEN"));
    }
    let config = AgentConfig::load(config_path)?;
    let Some(hub) = config.hub_url.as_deref() else {
        return Ok(());
    };
    let url = format!("{}/api/v1/update-events", hub.trim_end_matches('/'));
    while let Some(event) = events.pending.first() {
        let mut request = client.post(&url).bearer_auth(&token).json(event);
        if let (Ok(id), Ok(secret)) = (
            env::var("SAML_CF_ACCESS_CLIENT_ID"),
            env::var("SAML_CF_ACCESS_CLIENT_SECRET"),
        ) {
            request = request
                .header("CF-Access-Client-Id", id)
                .header("CF-Access-Client-Secret", secret);
        }
        let response = request.send().map_err(io::Error::other)?;
        if !response.status().is_success() {
            return Err(io::Error::other(format!(
                "hub rejected update event: HTTP {}",
                response.status()
            )));
        }
        let mut next = events.clone();
        next.pending.remove(0);
        save_json(events_path, &next)?;
        *events = next;
    }
    Ok(())
}

fn fetch(client: &reqwest::blocking::Client, url: &str, limit: u64) -> io::Result<Vec<u8>> {
    let mut request = client.get(url);
    if let (Ok(id), Ok(secret)) = (
        env::var("SAML_CF_ACCESS_CLIENT_ID"),
        env::var("SAML_CF_ACCESS_CLIENT_SECRET"),
    ) {
        request = request
            .header("CF-Access-Client-Id", id)
            .header("CF-Access-Client-Secret", secret);
    }
    let response = request.send().map_err(io::Error::other)?;
    if !response.status().is_success() {
        return Err(io::Error::other(format!(
            "update HTTP {}",
            response.status()
        )));
    }
    let mut bytes = Vec::new();
    response.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(io::Error::other("update response too large"));
    }
    Ok(bytes)
}

fn spawn_agent(binary: &Path, config: &Path) -> io::Result<Child> {
    Command::new(binary)
        .args(["run", "--config"])
        .arg(config)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
}

fn restore_agent(
    client: &reqwest::blocking::Client,
    previous: &ActiveAgent,
    config: &Path,
    port: u16,
) -> io::Result<Child> {
    let mut child = spawn_agent(&previous.binary, config)?;
    if let Err(error) = wait_for_health(client, &mut child, &previous.version, port) {
        let _ = stop_agent(&mut child);
        return Err(error);
    }
    Ok(child)
}

fn agent_binary_path(state_dir: &Path, version: &str) -> PathBuf {
    state_dir.join(format!(
        "saml-agent-{version}{}",
        std::env::consts::EXE_SUFFIX
    ))
}

fn stop_agent(child: &mut Child) -> io::Result<()> {
    if child.try_wait()?.is_none() {
        child.kill()?;
    }
    child.wait()?;
    Ok(())
}

fn wait_for_health(
    client: &reqwest::blocking::Client,
    child: &mut Child,
    expected: &str,
    port: u16,
) -> io::Result<()> {
    let url = format!("http://127.0.0.1:{port}/api/status");
    for _ in 0..60 {
        if child.try_wait()?.is_some() {
            return Err(io::Error::other("agent exited during startup"));
        }
        if client
            .get(&url)
            .send()
            .ok()
            .and_then(|response| response.json::<Value>().ok())
            .is_some_and(|value| value["version"] == expected && value["service"] == "saml-agent")
        {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(250));
    }
    Err(io::Error::other("agent health check timed out"))
}

fn save_state(path: &Path, active: &ActiveAgent) -> io::Result<()> {
    save_json(path, active)
}

fn save_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let mut options = AtomicWriteFile::options();
    #[cfg(unix)]
    {
        use atomic_write_file::unix::OpenOptionsExt;
        use std::os::unix::fs::OpenOptionsExt as _;
        options.preserve_mode(false).mode(0o600);
    }
    let mut file = options.open(path)?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    file.commit()
}

fn make_executable(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_then_successful_update_survives_retry() {
        let root = std::env::temp_dir().join(format!("saml-launcher-events-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let events_path = root.join("events.json");
        let config_path = root.join("agent.json");
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr().to_ip().unwrap();
        let config = AgentConfig::new(Some(format!("http://{address}"))).unwrap();
        config.save(&config_path).unwrap();
        let mut events = UpdateEventLog::default();
        queue_event(
            &events_path,
            &mut events,
            &config.agent_id,
            "0.3.3",
            "0.3.5",
            "download_failed",
            Some("target_invalid"),
        )
        .unwrap();
        queue_event(
            &events_path,
            &mut events,
            &config.agent_id,
            "0.3.3",
            "0.3.5",
            "download_failed",
            Some("target_invalid"),
        )
        .unwrap();
        assert_eq!(events.pending.len(), 1);
        queue_event(
            &events_path,
            &mut events,
            &config.agent_id,
            "0.3.3",
            "0.3.5",
            "installed",
            None,
        )
        .unwrap();
        assert_eq!(events.pending.len(), 2);
        let listener = thread::spawn(move || {
            for outcome in ["download_failed", "installed"] {
                let mut request = server.recv().unwrap();
                assert_eq!(request.url(), "/api/v1/update-events");
                assert_eq!(
                    request
                        .headers()
                        .iter()
                        .find(|header| header.field.equiv("Authorization"))
                        .unwrap()
                        .value
                        .as_str(),
                    "Bearer agent-token-with-more-than-32-characters"
                );
                let mut body = String::new();
                request.as_reader().read_to_string(&mut body).unwrap();
                assert_eq!(
                    serde_json::from_str::<Value>(&body).unwrap()["outcome"],
                    outcome
                );
                request.respond(tiny_http::Response::empty(202)).unwrap();
            }
        });
        env::set_var(
            "SAML_AGENT_TOKEN",
            "agent-token-with-more-than-32-characters",
        );
        let client = reqwest::blocking::Client::new();
        flush_events(&client, &config_path, &events_path, &mut events).unwrap();
        env::remove_var("SAML_AGENT_TOKEN");
        listener.join().unwrap();
        assert!(events.pending.is_empty());
        let persisted: UpdateEventLog =
            serde_json::from_slice(&fs::read(&events_path).unwrap()).unwrap();
        assert!(persisted.pending.is_empty());
        assert_eq!(persisted.last.unwrap().outcome, "installed");
        fs::remove_dir_all(root).unwrap();
    }
}
