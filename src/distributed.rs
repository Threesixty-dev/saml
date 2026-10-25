use crate::diagnostics::diagnostic_report;
use crate::semantic::analyze;
use crate::workspace::collect_sources_checked;
use atomic_write_file::AtomicWriteFile;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_HISTORY: usize = 100;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    pub schema_version: u32,
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hub_url: Option<String>,
    pub repositories: Vec<RegisteredRepository>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RegisteredRepository {
    pub id: String,
    pub root: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Heartbeat {
    pub schema_version: u32,
    pub agent_id: String,
    pub version: String,
    pub os: String,
    pub arch: String,
    pub health: String,
    pub repositories: Vec<RepositoryHealth>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryHealth {
    pub id: String,
    pub health: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct HubState {
    pub agents: BTreeMap<String, AgentRecord>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentRecord {
    pub last_seen_unix_secs: u64,
    pub heartbeat: Heartbeat,
    pub history: Vec<HealthSample>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HealthSample {
    pub at_unix_secs: u64,
    pub health: String,
    pub repositories: Vec<RepositoryHealth>,
}

impl AgentConfig {
    pub fn new(hub_url: Option<String>) -> io::Result<Self> {
        if let Some(url) = &hub_url {
            validate_hub_url(url)?;
        }
        Ok(Self {
            schema_version: PROTOCOL_VERSION,
            agent_id: Uuid::new_v4().to_string(),
            hub_url,
            repositories: Vec::new(),
        })
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        let config: Self = read_json(path)?;
        if config.schema_version != PROTOCOL_VERSION || Uuid::parse_str(&config.agent_id).is_err() {
            return Err(invalid("unsupported config version or agent ID"));
        }
        if let Some(url) = &config.hub_url {
            validate_hub_url(url)?;
        }
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        write_json(path, self)
    }

    pub fn add_repository(&mut self, root: &Path) -> io::Result<String> {
        let root = root.canonicalize()?;
        if !root.is_dir() {
            return Err(invalid("repository root must be a directory"));
        }
        if let Some(existing) = self.repositories.iter().find(|repo| repo.root == root) {
            return Ok(existing.id.clone());
        }
        let id = Uuid::new_v4().to_string();
        self.repositories.push(RegisteredRepository {
            id: id.clone(),
            root,
        });
        Ok(id)
    }

    pub fn heartbeat(&self) -> Heartbeat {
        let repositories: Vec<_> = self
            .repositories
            .iter()
            .map(|repo| {
                let health = if !repo.root.is_dir() {
                    "missing"
                } else {
                    let mut sources = HashMap::new();
                    match collect_sources_checked(&repo.root, &mut sources) {
                        Err(_) => "unavailable",
                        Ok(()) => {
                            let map = analyze(&sources);
                            if diagnostic_report(&sources, &map).errors > 0 {
                                "degraded"
                            } else {
                                "healthy"
                            }
                        }
                    }
                };
                RepositoryHealth {
                    id: repo.id.clone(),
                    health: health.into(),
                }
            })
            .collect();
        let health = if repositories.iter().any(|repo| repo.health != "healthy") {
            "degraded"
        } else {
            "healthy"
        };
        Heartbeat {
            schema_version: PROTOCOL_VERSION,
            agent_id: self.agent_id.clone(),
            version: env!("CARGO_PKG_VERSION").into(),
            os: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH.into(),
            health: health.into(),
            repositories,
        }
    }
}

impl HubState {
    pub fn load(path: &Path) -> io::Result<Self> {
        if path.exists() {
            read_json(path)
        } else {
            Ok(Self::default())
        }
    }

    pub fn record(&mut self, heartbeat: Heartbeat) -> io::Result<()> {
        validate_heartbeat(&heartbeat)?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_secs();
        let sample = HealthSample {
            at_unix_secs: now,
            health: heartbeat.health.clone(),
            repositories: heartbeat.repositories.clone(),
        };
        let record = self
            .agents
            .entry(heartbeat.agent_id.clone())
            .or_insert_with(|| AgentRecord {
                last_seen_unix_secs: now,
                heartbeat: heartbeat.clone(),
                history: Vec::new(),
            });
        record.last_seen_unix_secs = now;
        record.heartbeat = heartbeat;
        record.history.push(sample);
        if record.history.len() > MAX_HISTORY {
            record.history.drain(..record.history.len() - MAX_HISTORY);
        }
        Ok(())
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        write_json(path, self)
    }
}

pub fn validate_heartbeat(heartbeat: &Heartbeat) -> io::Result<()> {
    if heartbeat.schema_version != PROTOCOL_VERSION
        || Uuid::parse_str(&heartbeat.agent_id).is_err()
        || heartbeat.version.len() > 64
        || heartbeat.os.len() > 32
        || heartbeat.arch.len() > 32
        || !matches!(heartbeat.health.as_str(), "healthy" | "degraded")
        || heartbeat.repositories.len() > 1000
    {
        return Err(invalid("invalid heartbeat"));
    }
    let mut ids = std::collections::HashSet::new();
    for repo in &heartbeat.repositories {
        if Uuid::parse_str(&repo.id).is_err()
            || !ids.insert(&repo.id)
            || !matches!(
                repo.health.as_str(),
                "healthy" | "degraded" | "missing" | "unavailable"
            )
        {
            return Err(invalid("invalid repository health"));
        }
    }
    Ok(())
}

pub fn validate_hub_url(value: &str) -> io::Result<()> {
    let url = reqwest::Url::parse(value).map_err(|_| invalid("invalid hub URL"))?;
    let local_http = url.scheme() == "http"
        && matches!(
            url.host_str(),
            Some("127.0.0.1" | "localhost" | "[::1]" | "::1")
        );
    if (url.scheme() != "https" && !local_http)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid(
            "hub URL must use HTTPS (or loopback HTTP) without credentials",
        ));
    }
    Ok(())
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> io::Result<T> {
    serde_json::from_slice(&fs::read(path)?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut options = AtomicWriteFile::options();
    #[cfg(unix)]
    {
        use atomic_write_file::unix::OpenOptionsExt;
        use std::os::unix::fs::OpenOptionsExt as _;
        options.preserve_mode(false).mode(0o600);
    }
    let mut file = options.open(path)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    file.commit()
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_http_is_rejected() {
        assert!(validate_hub_url("http://example.com").is_err());
        assert!(validate_hub_url("http://127.0.0.1:8765").is_ok());
        assert!(validate_hub_url("https://example.com").is_ok());
    }

    #[test]
    fn repository_ids_persist_and_health_is_isolated() {
        let root = std::env::temp_dir().join(format!("saml-agent-test-{}", Uuid::new_v4()));
        fs::create_dir_all(root.join("good")).unwrap();
        fs::create_dir_all(root.join("bad")).unwrap();
        fs::write(root.join("good/model.spec"), "concept Entity {}").unwrap();
        fs::write(root.join("bad/model.saml"), "entity Broken {").unwrap();
        let config_path = root.join("config.json");
        let mut config = AgentConfig::new(Some("http://127.0.0.1:8765".into())).unwrap();
        let good_id = config.add_repository(&root.join("good")).unwrap();
        let bad_id = config.add_repository(&root.join("bad")).unwrap();
        assert_eq!(config.add_repository(&root.join("good")).unwrap(), good_id);
        config.save(&config_path).unwrap();
        let loaded = AgentConfig::load(&config_path).unwrap();
        let heartbeat = loaded.heartbeat();
        assert_eq!(heartbeat.repositories[0].id, good_id);
        assert_eq!(heartbeat.repositories[0].health, "healthy");
        assert_eq!(heartbeat.repositories[1].id, bad_id);
        assert_eq!(heartbeat.repositories[1].health, "degraded");
        let serialized = serde_json::to_string(&heartbeat).unwrap();
        assert!(!serialized.contains(root.to_str().unwrap()));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn hub_history_is_bounded() {
        let config = AgentConfig::new(Some("http://127.0.0.1:8765".into())).unwrap();
        let mut state = HubState::default();
        for _ in 0..MAX_HISTORY + 2 {
            state.record(config.heartbeat()).unwrap();
        }
        assert_eq!(state.agents[&config.agent_id].history.len(), MAX_HISTORY);
    }
}
