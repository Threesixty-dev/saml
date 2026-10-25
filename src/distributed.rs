use crate::repository_info::{self, RepositoryInfo, HUB_LIMITS};
use atomic_write_file::AtomicWriteFile;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_HISTORY: usize = 100;
pub const MAX_UPDATE_HISTORY: usize = 100;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_root: Option<PathBuf>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<RepositoryInfo>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct HubState {
    pub agents: BTreeMap<String, AgentRecord>,
    #[serde(default)]
    pub update_events: BTreeMap<String, Vec<UpdateEvent>>,
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateEvent {
    pub schema_version: u32,
    pub agent_id: String,
    pub event_id: String,
    pub at_unix_secs: u64,
    pub from_version: String,
    pub target_version: String,
    pub outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
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
        self.add_repository_with_model(root, None)
    }

    pub fn add_repository_with_model(
        &mut self,
        root: &Path,
        model_root: Option<&Path>,
    ) -> io::Result<String> {
        let root = root.canonicalize()?;
        if !root.is_dir() {
            return Err(invalid("repository root must be a directory"));
        }
        let model_root = model_root.map(|path| path.canonicalize()).transpose()?;
        if model_root.as_ref().is_some_and(|path| !path.is_dir()) {
            return Err(invalid("model root must be a directory"));
        }
        if let Some(existing) = self.repositories.iter_mut().find(|repo| repo.root == root) {
            if model_root.is_some() {
                existing.model_root = model_root;
            }
            return Ok(existing.id.clone());
        }
        let id = Uuid::new_v4().to_string();
        self.repositories.push(RegisteredRepository {
            id: id.clone(),
            root,
            model_root,
        });
        Ok(id)
    }

    pub fn heartbeat(&self) -> Heartbeat {
        let repositories: Vec<_> = self
            .repositories
            .iter()
            .map(|repo| {
                let details = repository_info::inspect(repo, HUB_LIMITS);
                let health = if !repo.root.is_dir() {
                    "missing"
                } else if !details.model.available {
                    "unavailable"
                } else if details.model.source_file_count == 0 || details.model.errors > 0 {
                    "degraded"
                } else {
                    "healthy"
                };
                RepositoryHealth {
                    id: repo.id.clone(),
                    health: health.into(),
                    details: Some(details),
                }
            })
            .collect();
        let health = if repositories.iter().any(|repo| repo.health != "healthy") {
            "degraded"
        } else {
            "healthy"
        };
        let mut heartbeat = Heartbeat {
            schema_version: PROTOCOL_VERSION,
            agent_id: self.agent_id.clone(),
            version: env!("CARGO_PKG_VERSION").into(),
            os: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH.into(),
            health: health.into(),
            repositories,
        };
        for index in (0..heartbeat.repositories.len()).rev() {
            // Keep the health of every repo even when a fleet exceeds the hub body limit.
            if serde_json::to_vec(&heartbeat).is_ok_and(|body| body.len() <= 60 * 1024) {
                break;
            }
            heartbeat.repositories[index].details = None;
        }
        heartbeat
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
            repositories: heartbeat
                .repositories
                .iter()
                .map(|repo| RepositoryHealth {
                    id: repo.id.clone(),
                    health: repo.health.clone(),
                    details: None,
                })
                .collect(),
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

    pub fn record_update(&mut self, event: UpdateEvent) -> io::Result<()> {
        validate_update_event(&event)?;
        let history = self
            .update_events
            .entry(event.agent_id.clone())
            .or_default();
        if history
            .iter()
            .any(|existing| existing.event_id == event.event_id)
        {
            return Ok(());
        }
        history.push(event);
        if history.len() > MAX_UPDATE_HISTORY {
            history.drain(..history.len() - MAX_UPDATE_HISTORY);
        }
        Ok(())
    }
}

pub fn validate_update_event(event: &UpdateEvent) -> io::Result<()> {
    let versions = Version::parse(&event.from_version)
        .and_then(|from| Version::parse(&event.target_version).map(|target| (from, target)));
    let valid_reason = match event.outcome.as_str() {
        "installed" => event.reason.is_none(),
        "rolled_back" => matches!(
            event.reason.as_deref(),
            Some("agent_exited" | "health_timeout" | "state_save_failed")
        ),
        "download_failed" => matches!(
            event.reason.as_deref(),
            Some("target_fetch_failed" | "target_invalid" | "target_stage_failed")
        ),
        "install_failed" => event.reason.as_deref() == Some("agent_stop_failed"),
        "rollback_failed" => event.reason.as_deref() == Some("restore_failed"),
        _ => false,
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_secs();
    if event.schema_version != PROTOCOL_VERSION
        || Uuid::parse_str(&event.agent_id).is_err()
        || Uuid::parse_str(&event.event_id).is_err()
        || !versions.is_ok_and(|(from, target)| target > from)
        || event.from_version.len() > 64
        || event.target_version.len() > 64
        || event.at_unix_secs < 1_577_836_800
        || event.at_unix_secs > now.saturating_add(86_400)
        || !valid_reason
    {
        return Err(invalid("invalid update event"));
    }
    Ok(())
}

pub fn validate_heartbeat(heartbeat: &Heartbeat) -> io::Result<()> {
    if heartbeat.schema_version != PROTOCOL_VERSION
        || Uuid::parse_str(&heartbeat.agent_id).is_err()
        || heartbeat.version.len() > 64
        || heartbeat.os.len() > 32
        || heartbeat.arch.len() > 32
        || !matches!(heartbeat.health.as_str(), "healthy" | "degraded")
        || heartbeat.repositories.len() > 1000
        || serde_json::to_vec(heartbeat).map_or(true, |body| body.len() > 60 * 1024)
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
        assert_eq!(
            heartbeat.repositories[0]
                .details
                .as_ref()
                .unwrap()
                .model
                .concept_count,
            1
        );
        assert!(
            heartbeat.repositories[1]
                .details
                .as_ref()
                .unwrap()
                .model
                .errors
                > 0
        );
        let serialized = serde_json::to_value(&heartbeat).unwrap();
        assert_eq!(
            serialized["repositories"][0]["details"]["path"],
            loaded.repositories[0].root.to_string_lossy().as_ref()
        );
        let mut state = HubState::default();
        state.record(heartbeat.clone()).unwrap();
        assert!(state.agents[&config.agent_id].heartbeat.repositories[0]
            .details
            .is_some());
        assert!(state.agents[&config.agent_id].history[0].repositories[0]
            .details
            .is_none());
        let mut oversized = heartbeat;
        oversized.repositories[0].details.as_mut().unwrap().name = "x".repeat(61 * 1024);
        assert!(validate_heartbeat(&oversized).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn external_model_root_keeps_repository_identity() {
        let root = std::env::temp_dir().join(format!("saml-model-test-{}", Uuid::new_v4()));
        fs::create_dir_all(root.join("source")).unwrap();
        fs::create_dir_all(root.join("model")).unwrap();
        fs::write(root.join("model/spec.saml"), "concept Entity {}").unwrap();
        let mut config = AgentConfig::new(None).unwrap();
        let id = config
            .add_repository_with_model(&root.join("source"), Some(&root.join("model")))
            .unwrap();
        assert_eq!(
            config.repositories[0].root,
            root.join("source").canonicalize().unwrap()
        );
        assert_eq!(
            config.repositories[0].model_root,
            Some(root.join("model").canonicalize().unwrap())
        );
        assert_eq!(config.heartbeat().repositories[0].health, "healthy");
        assert_eq!(config.add_repository(&root.join("source")).unwrap(), id);
        fs::remove_file(root.join("model/spec.saml")).unwrap();
        assert_eq!(config.heartbeat().repositories[0].health, "degraded");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn large_fleet_keeps_every_health_when_details_exceed_hub_limit() {
        let mut config = AgentConfig::new(None).unwrap();
        for index in 0..200 {
            config.repositories.push(RegisteredRepository {
                id: Uuid::new_v4().to_string(),
                root: std::env::temp_dir().join(format!("{}-{index}", "missing".repeat(24))),
                model_root: None,
            });
        }
        let heartbeat = config.heartbeat();
        assert_eq!(heartbeat.repositories.len(), 200);
        assert!(heartbeat
            .repositories
            .iter()
            .any(|repository| repository.details.is_none()));
        assert!(heartbeat
            .repositories
            .iter()
            .all(|repository| repository.health == "missing"));
        validate_heartbeat(&heartbeat).unwrap();
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

    #[test]
    fn update_events_are_validated_and_deduplicated() {
        let agent_id = Uuid::new_v4().to_string();
        let event = UpdateEvent {
            schema_version: PROTOCOL_VERSION,
            agent_id: agent_id.clone(),
            event_id: Uuid::new_v4().to_string(),
            at_unix_secs: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            from_version: "0.3.4".into(),
            target_version: "0.3.5".into(),
            outcome: "rolled_back".into(),
            reason: Some("health_timeout".into()),
        };
        let mut state = HubState::default();
        state.record_update(event.clone()).unwrap();
        state.record_update(event.clone()).unwrap();
        assert_eq!(state.update_events[&agent_id].len(), 1);
        let mut unsafe_event = event;
        unsafe_event.event_id = Uuid::new_v4().to_string();
        unsafe_event.reason = Some("/private/path?token=secret".into());
        assert!(state.record_update(unsafe_event).is_err());
        assert_eq!(state.update_events[&agent_id].len(), 1);
    }
}
