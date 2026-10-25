use crate::diagnostics::diagnostic_report;
use crate::distributed::AgentConfig;
use crate::semantic::analyze;
use crate::workspace::collect_sources;
use serde_json::json;
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

const HTML: &str = include_str!("../viewer/index.html");
const CSS: &str = include_str!("../viewer/style.css");
const SCRIPT: &str = include_str!("../viewer/app.js");
const GUIDE: &str = include_str!("../LLM_GUIDE.md");

pub enum PortalSource {
    Static(Vec<PathBuf>),
    AgentConfig(PathBuf),
}

#[derive(Clone, Debug)]
pub struct PortalRepository {
    pub id: String,
    pub root: PathBuf,
}

impl PortalSource {
    fn repositories(&self) -> io::Result<Vec<PortalRepository>> {
        match self {
            Self::Static(roots) => Ok(roots
                .iter()
                .enumerate()
                .map(|(index, root)| PortalRepository {
                    id: index.to_string(),
                    root: root.clone(),
                })
                .collect()),
            Self::AgentConfig(path) => Ok(AgentConfig::load(path)?
                .repositories
                .into_iter()
                .map(|repo| PortalRepository {
                    id: repo.id,
                    root: repo.root,
                })
                .collect()),
        }
    }

    fn service(&self) -> &'static str {
        match self {
            Self::Static(_) => "saml-view",
            Self::AgentConfig(_) => "saml-agent",
        }
    }
}

pub fn serve(listener: TcpListener, source: PortalSource) -> io::Result<()> {
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(error) = handle(stream, &source) {
                    eprintln!("portal request failed: {error}");
                }
            }
            Err(error) => eprintln!("portal connection failed: {error}"),
        }
    }
    Ok(())
}

fn handle(mut stream: TcpStream, source: &PortalSource) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    let mut reader = BufReader::new(&stream);
    let mut request = String::new();
    reader.read_line(&mut request)?;
    if request.len() > 8192 {
        return respond(
            &mut stream,
            "400 Bad Request",
            "text/plain",
            b"Invalid request",
        );
    }
    let mut host = None;
    let mut complete = false;
    for _ in 0..64 {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        if line == "\r\n" || line == "\n" {
            complete = true;
            break;
        }
        if line.len() > 8192 {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            break;
        };
        if name.eq_ignore_ascii_case("host") && host.replace(value.trim().to_owned()).is_some() {
            break;
        }
    }
    drop(reader);
    if !complete {
        return respond(
            &mut stream,
            "400 Bad Request",
            "text/plain",
            b"Invalid headers",
        );
    }
    if !host.as_deref().is_some_and(local_host) {
        return respond(&mut stream, "403 Forbidden", "text/plain", b"Invalid host");
    }
    let mut parts = request.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");
    let (route, query) = target.split_once('?').unwrap_or((target, ""));
    if method != "GET" {
        return respond(
            &mut stream,
            "405 Method Not Allowed",
            "text/plain",
            b"GET only",
        );
    }
    let roots = match source.repositories() {
        Ok(value) => value,
        Err(error) => {
            eprintln!("cannot load portal repositories: {error}");
            return respond(
                &mut stream,
                "503 Service Unavailable",
                "text/plain",
                b"Repository registry unavailable",
            );
        }
    };
    match route {
        "/api/status" => {
            let body = serde_json::to_vec(&json!({
                "schema_version": 1,
                "service": source.service(),
                "version": env!("CARGO_PKG_VERSION"),
                "health": "serving",
                "os": std::env::consts::OS,
                "arch": std::env::consts::ARCH,
                "workspaces": roots.len(),
            }))?;
            respond(
                &mut stream,
                "200 OK",
                "application/json; charset=utf-8",
                &body,
            )
        }
        "/" => respond(
            &mut stream,
            "200 OK",
            "text/html; charset=utf-8",
            HTML.as_bytes(),
        ),
        "/style.css" => respond(
            &mut stream,
            "200 OK",
            "text/css; charset=utf-8",
            CSS.as_bytes(),
        ),
        "/app.js" => respond(
            &mut stream,
            "200 OK",
            "text/javascript; charset=utf-8",
            SCRIPT.as_bytes(),
        ),
        "/api/workspaces" => {
            let workspaces: Vec<_> = roots
                .iter()
                .map(|repo| {
                    let id = match source {
                        PortalSource::Static(_) => json!(repo.id.parse::<usize>().unwrap_or(0)),
                        PortalSource::AgentConfig(_) => json!(repo.id),
                    };
                    json!({
                        "id": id,
                        "name": repo.root.file_name().unwrap_or(repo.root.as_os_str()).to_string_lossy(),
                        "path": repo.root.to_string_lossy(),
                    })
                })
                .collect();
            let body = serde_json::to_vec(&workspaces)?;
            respond(
                &mut stream,
                "200 OK",
                "application/json; charset=utf-8",
                &body,
            )
        }
        "/api/map" => {
            let Some(root) = selected_root(query, &roots) else {
                return respond(
                    &mut stream,
                    "400 Bad Request",
                    "text/plain",
                    b"Invalid workspace",
                );
            };
            let mut sources = HashMap::new();
            collect_sources(root, &mut sources);
            let body = serde_json::to_vec(&analyze(&sources))?;
            respond(
                &mut stream,
                "200 OK",
                "application/json; charset=utf-8",
                &body,
            )
        }
        "/api/diagnostics" => {
            let Some(root) = selected_root(query, &roots) else {
                return respond(
                    &mut stream,
                    "400 Bad Request",
                    "text/plain",
                    b"Invalid workspace",
                );
            };
            let mut sources = HashMap::new();
            collect_sources(root, &mut sources);
            let body = serde_json::to_vec(&diagnostic_report(&sources, &analyze(&sources)))?;
            respond(
                &mut stream,
                "200 OK",
                "application/json; charset=utf-8",
                &body,
            )
        }
        "/api/guide" => respond(
            &mut stream,
            "200 OK",
            "text/markdown; charset=utf-8",
            GUIDE.as_bytes(),
        ),
        "/favicon.ico" => respond(&mut stream, "204 No Content", "text/plain", b""),
        _ => respond(&mut stream, "404 Not Found", "text/plain", b"Not found"),
    }
}

fn local_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    ["localhost", "127.0.0.1", "[::1]"].iter().any(|name| {
        host.eq_ignore_ascii_case(name)
            || host
                .strip_prefix(&format!("{name}:"))
                .is_some_and(|port| port.parse::<u16>().is_ok())
    })
}

fn selected_root<'a>(query: &str, roots: &'a [PortalRepository]) -> Option<&'a Path> {
    let mut selected = None;
    for part in query.split('&') {
        if let Some(value) = part.strip_prefix("workspace=") {
            if selected.is_some() {
                return None;
            }
            selected = Some(value);
        }
    }
    match selected {
        Some(id) => roots
            .iter()
            .find(|repo| repo.id == id)
            .map(|repo| repo.root.as_path()),
        None => roots.first().map(|repo| repo.root.as_path()),
    }
}

fn respond(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_selection_is_bounded_and_unambiguous() {
        let roots = [
            PortalRepository {
                id: "first".into(),
                root: PathBuf::from("/one"),
            },
            PortalRepository {
                id: "second".into(),
                root: PathBuf::from("/two"),
            },
        ];
        assert_eq!(selected_root("", &roots), Some(Path::new("/one")));
        assert_eq!(
            selected_root("workspace=second", &roots),
            Some(Path::new("/two"))
        );
        assert_eq!(selected_root("workspace=third", &roots), None);
        assert_eq!(selected_root("workspace=abc", &roots), None);
        assert_eq!(
            selected_root("workspace=first&workspace=second", &roots),
            None
        );
    }

    #[test]
    fn local_host_check_rejects_rebinding_names() {
        assert!(local_host("localhost:8787"));
        assert!(local_host("127.0.0.1:8787"));
        assert!(local_host("[::1]:8787"));
        assert!(!local_host("localhost.example.com:8787"));
        assert!(!local_host("127.0.0.1.evil.example:8787"));
    }
}
