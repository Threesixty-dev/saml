use atomic_write_file::AtomicWriteFile;
use saml_lsp::distributed::{validate_heartbeat, Heartbeat, HubState};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use uuid::Uuid;

const MAX_BODY: u64 = 64 * 1024;

#[derive(Default, Deserialize, Serialize)]
struct Credentials {
    agents: HashMap<String, String>,
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut args = env::args().skip(1);
    let command = args.next();
    if command.as_deref() == Some("provision") {
        let agent_id = args
            .next()
            .ok_or("usage: saml-hub provision AGENT_ID CREDENTIALS_PATH")?;
        let path = PathBuf::from(
            args.next()
                .ok_or("usage: saml-hub provision AGENT_ID CREDENTIALS_PATH")?,
        );
        if args.next().is_some() || Uuid::parse_str(&agent_id).is_err() {
            return Err("invalid provision arguments or agent ID".into());
        }
        let mut credentials = if path.exists() {
            load_credentials(&path)?
        } else {
            Credentials::default()
        };
        if credentials.agents.contains_key(&agent_id) {
            return Err("agent already provisioned".into());
        }
        let token = format!("{}{}", Uuid::new_v4(), Uuid::new_v4());
        credentials.agents.insert(agent_id, token.clone());
        save_credentials(&path, &credentials)?;
        println!("{token}");
        return Ok(());
    }
    if command.is_some() {
        return Err("usage: saml-hub [provision AGENT_ID CREDENTIALS_PATH]".into());
    }
    let bind: SocketAddr = env::var("SAML_HUB_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8765".into())
        .parse()?;
    if !bind.ip().is_loopback() {
        return Err(
            "SAML hub binds only to loopback; expose it through an HTTPS reverse proxy".into(),
        );
    }
    let state_path = PathBuf::from(env::var("SAML_HUB_STATE")?);
    let credentials_path = PathBuf::from(env::var("SAML_HUB_CREDENTIALS")?);
    let admin_token = env::var("SAML_HUB_ADMIN_TOKEN")?;
    if admin_token.len() < 32 {
        return Err("SAML_HUB_ADMIN_TOKEN must be at least 32 characters".into());
    }
    load_credentials(&credentials_path)?;
    let mut state = HubState::load(&state_path)?;
    let server = Server::http(bind)?;
    println!("SAML hub listening on {bind}");
    for request in server.incoming_requests() {
        if let Err(error) = handle(
            request,
            &mut state,
            &state_path,
            &credentials_path,
            &admin_token,
        ) {
            eprintln!("Hub request failed: {error}");
        }
    }
    Ok(())
}

fn handle(
    mut request: Request,
    state: &mut HubState,
    state_path: &Path,
    credentials_path: &Path,
    admin_token: &str,
) -> io::Result<()> {
    let route = request.url().split('?').next().unwrap_or("");
    match (request.method(), route) {
        (Method::Get, "/healthz") => respond_json(request, 200, &json!({"health":"serving"})),
        (Method::Get, "/api/v1/agents") => {
            if !authorized(&request, admin_token) {
                return respond_json(request, 401, &json!({"error":"unauthorized"}));
            }
            respond_json(request, 200, state)
        }
        (Method::Post, "/api/v1/heartbeat") => {
            let mut body = Vec::new();
            request
                .as_reader()
                .take(MAX_BODY + 1)
                .read_to_end(&mut body)?;
            if body.len() as u64 > MAX_BODY {
                return respond_json(request, 413, &json!({"error":"body too large"}));
            }
            let heartbeat: Heartbeat = match serde_json::from_slice(&body) {
                Ok(value) if validate_heartbeat(&value).is_ok() => value,
                _ => return respond_json(request, 400, &json!({"error":"invalid heartbeat"})),
            };
            let credentials = match load_credentials(credentials_path) {
                Ok(value) => value,
                Err(error) => {
                    eprintln!("Cannot load agent credentials: {error}");
                    return respond_json(request, 503, &json!({"error":"credentials unavailable"}));
                }
            };
            let Some(token) = credentials.agents.get(&heartbeat.agent_id) else {
                return respond_json(request, 401, &json!({"error":"unauthorized"}));
            };
            if !authorized(&request, token) {
                return respond_json(request, 401, &json!({"error":"unauthorized"}));
            }
            let mut next = state.clone();
            if let Err(error) = next.record(heartbeat).and_then(|()| next.save(state_path)) {
                eprintln!("Cannot persist heartbeat: {error}");
                return respond_json(request, 503, &json!({"error":"registry unavailable"}));
            }
            *state = next;
            respond_json(request, 202, &json!({"accepted":true}))
        }
        _ => respond_json(request, 404, &json!({"error":"not found"})),
    }
}

fn load_credentials(path: &Path) -> io::Result<Credentials> {
    let credentials: Credentials = serde_json::from_slice(&fs::read(path)?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if credentials.agents.values().any(|token| token.len() < 32) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "agent credentials must be at least 32 characters",
        ));
    }
    Ok(credentials)
}

fn save_credentials(path: &Path, credentials: &Credentials) -> io::Result<()> {
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
    serde_json::to_writer_pretty(&mut file, credentials)?;
    file.write_all(b"\n")?;
    file.commit()
}

fn authorized(request: &Request, token: &str) -> bool {
    request.headers().iter().any(|header| {
        header.field.equiv("Authorization") && header.value.as_str() == format!("Bearer {token}")
    })
}

fn respond_json<T: serde::Serialize>(request: Request, code: u16, value: &T) -> io::Result<()> {
    let body = serde_json::to_vec(value)?;
    let content_type = Header::from_bytes("Content-Type", "application/json; charset=utf-8")
        .map_err(|_| io::Error::other("invalid response header"))?;
    request.respond(
        Response::from_data(body)
            .with_status_code(StatusCode(code))
            .with_header(content_type),
    )
}
