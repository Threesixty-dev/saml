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
const DASHBOARD_HTML: &str = include_str!("../../hub/index.html");
const DASHBOARD_CSS: &str = include_str!("../../hub/style.css");
const DASHBOARD_JS: &str = include_str!("../../hub/app.js");

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
    if !bind.ip().is_loopback() && env::var("SAML_HUB_ALLOW_NON_LOOPBACK").as_deref() != Ok("1") {
        return Err(
            "SAML hub binds only to loopback unless SAML_HUB_ALLOW_NON_LOOPBACK=1; expose it through an HTTPS reverse proxy".into(),
        );
    }
    let state_path = PathBuf::from(env::var("SAML_HUB_STATE")?);
    let credentials_path = PathBuf::from(env::var("SAML_HUB_CREDENTIALS")?);
    let update_dir = env::var("SAML_HUB_UPDATE_DIR")
        .ok()
        .map(|value| PathBuf::from(value).canonicalize())
        .transpose()?;
    let admin_token = match env::var("SAML_HUB_AUTH_MODE").as_deref() {
        Ok("external") => None,
        Ok("token") | Err(env::VarError::NotPresent) => {
            let token = env::var("SAML_HUB_ADMIN_TOKEN")?;
            if token.len() < 32 {
                return Err("SAML_HUB_ADMIN_TOKEN must be at least 32 characters".into());
            }
            Some(token)
        }
        _ => return Err("SAML_HUB_AUTH_MODE must be token or external".into()),
    };
    if !credentials_path.exists() {
        save_credentials(&credentials_path, &Credentials::default())?;
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
            admin_token.as_deref(),
            update_dir.as_deref(),
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
    admin_token: Option<&str>,
    update_dir: Option<&Path>,
) -> io::Result<()> {
    let route = request.url().split('?').next().unwrap_or("").to_owned();
    if request.method() == &Method::Get && route.starts_with("/updates/") {
        return serve_update_file(request, &route, update_dir);
    }
    match (request.method(), route.as_str()) {
        (Method::Get, "/") => respond_bytes(
            request,
            200,
            "text/html; charset=utf-8",
            DASHBOARD_HTML.as_bytes(),
        ),
        (Method::Get, "/style.css") => respond_bytes(
            request,
            200,
            "text/css; charset=utf-8",
            DASHBOARD_CSS.as_bytes(),
        ),
        (Method::Get, "/app.js") => respond_bytes(
            request,
            200,
            "text/javascript; charset=utf-8",
            DASHBOARD_JS.as_bytes(),
        ),
        (Method::Get, "/healthz") => respond_json(
            request,
            200,
            &json!({"health":"serving", "version":env!("CARGO_PKG_VERSION")}),
        ),
        (Method::Get, "/api/v1/config") => respond_json(
            request,
            200,
            &json!({"auth_mode": if admin_token.is_some() { "token" } else { "external" }}),
        ),
        (Method::Get, "/api/v1/agents") => {
            if admin_token.is_some_and(|token| !authorized(&request, token)) {
                return respond_json(request, 401, &json!({"error":"unauthorized"}));
            }
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(io::Error::other)?
                .as_secs();
            respond_json(
                request,
                200,
                &json!({
                    "observed_at_unix_secs": now,
                    "agents": &state.agents,
                }),
            )
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

fn serve_update_file(request: Request, route: &str, update_dir: Option<&Path>) -> io::Result<()> {
    let Some(base) = update_dir else {
        return respond_json(request, 404, &json!({"error":"not found"}));
    };
    let (subdir, name, max_size) = if let Some(name) = route.strip_prefix("/updates/metadata/") {
        ("metadata", name, 10 * 1024 * 1024)
    } else if let Some(name) = route.strip_prefix("/updates/targets/") {
        ("targets", name, 512 * 1024 * 1024)
    } else {
        return respond_json(request, 404, &json!({"error":"not found"}));
    };
    if !safe_update_name(name) {
        return respond_json(request, 404, &json!({"error":"not found"}));
    }
    let directory = match base.join(subdir).canonicalize() {
        Ok(path) if path.starts_with(base) && path.is_dir() => path,
        _ => return respond_json(request, 404, &json!({"error":"not found"})),
    };
    let path = match directory.join(name).canonicalize() {
        Ok(path) if path.starts_with(&directory) && path.is_file() => path,
        _ => return respond_json(request, 404, &json!({"error":"not found"})),
    };
    let file = fs::File::open(&path)?;
    if file.metadata()?.len() > max_size {
        return respond_json(request, 413, &json!({"error":"update file too large"}));
    }
    let mime = if subdir == "metadata" {
        "application/json"
    } else {
        "application/octet-stream"
    };
    let content_type = Header::from_bytes("Content-Type", mime)
        .map_err(|_| io::Error::other("invalid response header"))?;
    let nosniff = Header::from_bytes("X-Content-Type-Options", "nosniff")
        .map_err(|_| io::Error::other("invalid response header"))?;
    let cache = Header::from_bytes("Cache-Control", "no-store")
        .map_err(|_| io::Error::other("invalid response header"))?;
    request.respond(
        Response::from_file(file)
            .with_header(content_type)
            .with_header(nosniff)
            .with_header(cache),
    )
}

fn safe_update_name(name: &str) -> bool {
    !name.is_empty()
        && name.split('/').all(|segment| {
            !segment.is_empty()
                && segment != "."
                && segment != ".."
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        })
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
    respond_bytes(request, code, "application/json; charset=utf-8", &body)
}

fn respond_bytes(request: Request, code: u16, mime: &str, body: &[u8]) -> io::Result<()> {
    let content_type = Header::from_bytes("Content-Type", mime)
        .map_err(|_| io::Error::other("invalid response header"))?;
    let cache = Header::from_bytes("Cache-Control", "no-store")
        .map_err(|_| io::Error::other("invalid response header"))?;
    let nosniff = Header::from_bytes("X-Content-Type-Options", "nosniff")
        .map_err(|_| io::Error::other("invalid response header"))?;
    let policy = Header::from_bytes(
        "Content-Security-Policy",
        "default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'",
    )
    .map_err(|_| io::Error::other("invalid response header"))?;
    request.respond(
        Response::from_data(body.to_vec())
            .with_status_code(StatusCode(code))
            .with_header(content_type)
            .with_header(cache)
            .with_header(nosniff)
            .with_header(policy),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_paths_reject_traversal_and_encoded_names() {
        assert!(safe_update_name("1.root.json"));
        assert!(safe_update_name("saml-agent/linux/x86_64/0.4.0"));
        assert!(!safe_update_name("../credentials.json"));
        assert!(!safe_update_name("dir//file"));
        assert!(!safe_update_name("%2e%2e/credentials.json"));
        assert!(!safe_update_name("file?download=1"));
    }
}
