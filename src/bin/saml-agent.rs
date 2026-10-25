use saml_lsp::distributed::{validate_hub_url, AgentConfig};
use saml_lsp::portal::{serve, PortalSource};
use std::env;
use std::io;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let command = args
        .next()
        .ok_or("usage: saml-agent <init|set-hub|add|remove|run> --config PATH ...")?;
    let mut config_path = None;
    let mut hub_url = None;
    let mut repo_path = None;
    let mut repo_id = None;
    let mut once = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => {
                config_path = Some(PathBuf::from(args.next().ok_or("missing --config value")?))
            }
            "--hub" => hub_url = Some(args.next().ok_or("missing --hub value")?),
            "--repo" => repo_path = Some(PathBuf::from(args.next().ok_or("missing --repo value")?)),
            "--id" => repo_id = Some(args.next().ok_or("missing --id value")?),
            "--once" => once = true,
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }
    let config_path = config_path.ok_or("--config PATH is required")?;
    match command.as_str() {
        "init" => {
            if config_path.exists() {
                return Err("config already exists".into());
            }
            let config = AgentConfig::new(hub_url)?;
            config.save(&config_path)?;
            println!("Agent ID: {}", config.agent_id);
            println!("Config: {}", config_path.display());
        }
        "set-hub" => {
            let url = hub_url.ok_or("--hub URL is required")?;
            validate_hub_url(&url)?;
            let mut config = AgentConfig::load(&config_path)?;
            config.hub_url = Some(url);
            config.save(&config_path)?;
            println!("Hub configured");
        }
        "add" => {
            let mut config = AgentConfig::load(&config_path)?;
            let id = config.add_repository(&repo_path.ok_or("--repo PATH is required")?)?;
            config.save(&config_path)?;
            println!("Repository ID: {id}");
        }
        "remove" => {
            let mut config = AgentConfig::load(&config_path)?;
            let id = repo_id.ok_or("--id ID is required")?;
            let count = config.repositories.len();
            config.repositories.retain(|repo| repo.id != id);
            if config.repositories.len() == count {
                return Err("repository ID not found".into());
            }
            config.save(&config_path)?;
            println!("Removed repository {id}");
        }
        "run" => {
            let config = AgentConfig::load(&config_path)?;
            let token = env::var("SAML_AGENT_TOKEN").ok();
            if token.as_deref().is_some_and(|value| value.len() < 32) {
                return Err("SAML_AGENT_TOKEN must be at least 32 characters".into());
            }
            if once && token.is_none() {
                return Err("SAML_AGENT_TOKEN is required for --once".into());
            }
            if token.is_some() && config.hub_url.is_none() {
                return Err("configure --hub before sending heartbeats".into());
            }
            if !once {
                let port = env::var("SAML_AGENT_PORT")
                    .ok()
                    .map(|value| value.parse::<u16>())
                    .transpose()?
                    .unwrap_or(8787);
                let listener = TcpListener::bind(("127.0.0.1", port))?;
                println!("SAML agent portal: http://{}", listener.local_addr()?);
                let portal_config = config_path.clone();
                thread::spawn(move || {
                    if let Err(error) = serve(listener, PortalSource::AgentConfig(portal_config)) {
                        eprintln!("Agent portal stopped: {error}");
                    }
                });
                if token.is_none() {
                    println!("Hub heartbeat disabled; local portal remains available");
                }
            }
            let client = reqwest::blocking::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(15))
                .build()?;
            let interval = env::var("SAML_HEARTBEAT_SECS")
                .ok()
                .map(|value| value.parse::<u64>())
                .transpose()?
                .unwrap_or(30)
                .max(5);
            loop {
                if let Some(token) = &token {
                    match send_once(&client, &config_path, token) {
                        Ok(()) => println!("Heartbeat sent"),
                        Err(error) if once => return Err(error),
                        Err(error) => eprintln!("Heartbeat failed: {error}"),
                    }
                }
                if once {
                    break;
                }
                thread::sleep(Duration::from_secs(interval));
            }
        }
        _ => return Err(format!("unknown command: {command}").into()),
    }
    Ok(())
}

fn send_once(
    client: &reqwest::blocking::Client,
    config_path: &Path,
    token: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = AgentConfig::load(config_path)?;
    let hub_url = config
        .hub_url
        .as_deref()
        .ok_or("hub URL is not configured")?;
    let url = format!("{}/api/v1/heartbeat", hub_url.trim_end_matches('/'));
    let response = client
        .post(url)
        .bearer_auth(token)
        .json(&config.heartbeat())
        .send()?;
    if !response.status().is_success() {
        return Err(io::Error::other(format!(
            "hub rejected heartbeat: HTTP {}",
            response.status()
        ))
        .into());
    }
    Ok(())
}
