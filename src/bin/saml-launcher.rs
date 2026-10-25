use atomic_write_file::AtomicWriteFile;
use saml_lsp::distributed::AgentConfig;
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
    let mut active = if state_path.exists() {
        let stored: ActiveAgent = serde_json::from_slice(&fs::read(&state_path)?)?;
        let expected = state_dir.join(format!("saml-agent-{}", stored.version));
        if stored.binary != expected || !expected.is_file() {
            return Err("invalid active agent state".into());
        }
        Version::parse(&stored.version)?;
        stored
    } else {
        let version = env!("CARGO_PKG_VERSION");
        let binary = state_dir.join(format!("saml-agent-{version}"));
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
                stop_agent(&mut child)?;
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
                        active = ActiveAgent { version, binary };
                        if let Err(error) = save_state(&state_path, &active) {
                            eprintln!("Cannot save updated agent state: {error}");
                            child = candidate;
                            stop_agent(&mut child)?;
                            active = previous;
                            child = spawn_agent(&active.binary, &config)?;
                        } else {
                            child = candidate;
                            failed_version = None;
                            println!("SAML agent updated to {}", active.version);
                        }
                    }
                    Err(error) => {
                        eprintln!("Update {version} failed health check; restoring previous agent: {error}");
                        failed_version = Some(version);
                        child = spawn_agent(&previous.binary, &config)?;
                    }
                }
            }
            Ok(_) => {}
            Err(error) => eprintln!("Update check failed: {error}"),
        }
        thread::sleep(Duration::from_secs(interval));
    }
}

fn check_update(
    client: &reqwest::blocking::Client,
    config_path: &Path,
    active: &ActiveAgent,
    state_dir: &Path,
) -> Result<Option<(String, PathBuf)>, Box<dyn std::error::Error>> {
    let config = AgentConfig::load(config_path)?;
    let Some(hub) = config.hub_url.as_deref() else {
        return Ok(None);
    };
    let base = hub.trim_end_matches('/');
    let manifest = fetch(
        client,
        &format!("{base}/updates/metadata/release.json"),
        MAX_MANIFEST_BYTES as u64,
    )?;
    let signature = fetch(
        client,
        &format!("{base}/updates/metadata/release.sig"),
        1024,
    )?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let current = Version::parse(&active.version)?;
    let Some((version, target)) = verify_manifest(
        &manifest,
        &signature,
        ROOT_PUBLIC_KEY,
        now,
        &current,
        &platform(),
    )?
    else {
        return Ok(None);
    };
    let path = state_dir.join(format!("saml-agent-{version}"));
    if path.exists() {
        verify_target(&fs::read(&path)?, &target)?;
    } else {
        let bytes = fetch(
            client,
            &format!("{base}/updates/targets/{}", target.path),
            MAX_TARGET_BYTES,
        )?;
        verify_target(&bytes, &target)?;
        let temp = state_dir.join(format!(".saml-agent-{version}-{}.tmp", Uuid::new_v4()));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        make_executable(&temp)?;
        fs::rename(temp, &path)?;
    }
    Ok(Some((version.to_string(), path)))
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
    let mut options = AtomicWriteFile::options();
    #[cfg(unix)]
    {
        use atomic_write_file::unix::OpenOptionsExt;
        use std::os::unix::fs::OpenOptionsExt as _;
        options.preserve_mode(false).mode(0o600);
    }
    let mut file = options.open(path)?;
    serde_json::to_writer(&mut file, active)?;
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
