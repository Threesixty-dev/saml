use crate::diagnostics::diagnostic_report;
use crate::distributed::{AgentConfig, RegisteredRepository};
use crate::repository_info::{self, LOCAL_LIMITS};
use crate::semantic::analyze;
use crate::semantic_query;
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
    pub model_root: PathBuf,
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
                    model_root: root.clone(),
                })
                .collect()),
            Self::AgentConfig(path) => Ok(AgentConfig::load(path)?
                .repositories
                .into_iter()
                .map(|repo| PortalRepository {
                    id: repo.id,
                    model_root: repo.model_root.unwrap_or_else(|| repo.root.clone()),
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
                "telemetry_level": if source.service() == "saml-agent" { "repository_details" } else { "local_only" },
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
        "/api/repository" | "/api/git" => {
            let Some(repo) = selected_repo(query, &roots) else {
                return respond(
                    &mut stream,
                    "400 Bad Request",
                    "text/plain",
                    b"Invalid workspace",
                );
            };
            let body = if route == "/api/git" {
                serde_json::to_vec(&repository_info::inspect_git(&repo.root, LOCAL_LIMITS))?
            } else {
                let registered = RegisteredRepository {
                    id: repo.id.clone(),
                    root: repo.root.clone(),
                    model_root: Some(repo.model_root.clone()),
                };
                serde_json::to_vec(&repository_info::inspect(&registered, LOCAL_LIMITS))?
            };
            respond(
                &mut stream,
                "200 OK",
                "application/json; charset=utf-8",
                &body,
            )
        }
        "/api/git/history" => {
            let Some(repo) = selected_repo(query, &roots) else {
                return respond(
                    &mut stream,
                    "400 Bad Request",
                    "text/plain",
                    b"Invalid workspace",
                );
            };
            let (Ok(offset), Ok(limit)) = (
                numeric_parameter(query, "offset", 0),
                numeric_parameter(query, "limit", 50),
            ) else {
                return respond(
                    &mut stream,
                    "400 Bad Request",
                    "text/plain",
                    b"Invalid page",
                );
            };
            if !(1..=100).contains(&limit) {
                return respond(
                    &mut stream,
                    "400 Bad Request",
                    "text/plain",
                    b"Invalid limit",
                );
            }
            let body = serde_json::to_vec(&repository_info::inspect_git_history(
                &repo.root, offset, limit,
            ))?;
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
        "/api/workflow" => {
            let body = serde_json::to_vec(&semantic_query::workflow())?;
            respond(
                &mut stream,
                "200 OK",
                "application/json; charset=utf-8",
                &body,
            )
        }
        "/api/query" | "/api/item" | "/api/source" => {
            let Some(repo) = selected_repo(query, &roots) else {
                return respond(
                    &mut stream,
                    "400 Bad Request",
                    "text/plain",
                    b"Invalid workspace",
                );
            };
            let mut sources = HashMap::new();
            collect_sources(&repo.model_root, &mut sources);
            let (status, result) = match route {
                "/api/query" => match (parameter(query, "q"), parameter(query, "cursor")) {
                    (Ok(Some(q)), Ok(cursor)) if !q.trim().is_empty() && q.len() <= 200 => {
                        let q = q.trim();
                        match semantic_query::search_with_cursor(
                            &analyze(&sources),
                            &repo.id,
                            q,
                            cursor.as_deref(),
                        ) {
                            Ok(mut result) => {
                                if let Some(next_cursor) = result["next_cursor"].as_str() {
                                    result["next_url"] = json!(format!(
                                        "/api/query?workspace={}&q={}&cursor={}",
                                        semantic_query::encode(&repo.id),
                                        semantic_query::encode(q),
                                        semantic_query::encode(next_cursor),
                                    ));
                                }
                                ("200 OK", result)
                            }
                            Err(error) => ("400 Bad Request", json!({"error": error})),
                        }
                    }
                    _ => (
                        "400 Bad Request",
                        json!({"error": "Provide one nonempty q of at most 200 bytes and at most one cursor"}),
                    ),
                },
                "/api/item" => match (parameter(query, "kind"), parameter(query, "id")) {
                    (Ok(Some(kind)), Ok(Some(id))) if !id.is_empty() => {
                        match semantic_query::item(&analyze(&sources), &repo.id, &kind, &id) {
                            Some(item) => ("200 OK", item),
                            None => ("404 Not Found", json!({"error": "Item not found"})),
                        }
                    }
                    _ => (
                        "400 Bad Request",
                        json!({"error": "Provide one kind and id"}),
                    ),
                },
                _ => match parameter(query, "uri") {
                    Ok(Some(uri)) => match sources.get(&uri) {
                        Some(content) => ("200 OK", json!({"uri": uri, "content": content})),
                        None => ("404 Not Found", json!({"error": "Source not found"})),
                    },
                    _ => (
                        "400 Bad Request",
                        json!({"error": "Provide one source uri"}),
                    ),
                },
            };
            let body = serde_json::to_vec(&result)?;
            respond(
                &mut stream,
                status,
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
    selected_repo(query, roots).map(|repo| repo.model_root.as_path())
}

fn selected_repo<'a>(query: &str, roots: &'a [PortalRepository]) -> Option<&'a PortalRepository> {
    match parameter(query, "workspace").ok()? {
        Some(id) => roots.iter().find(|repo| repo.id == id),
        None => roots.first(),
    }
}

fn numeric_parameter(query: &str, key: &str, default: usize) -> Result<usize, ()> {
    parameter(query, key)?.map_or(Ok(default), |value| value.parse().map_err(|_| ()))
}

fn parameter(query: &str, key: &str) -> Result<Option<String>, ()> {
    let mut found = None;
    for part in query.split('&') {
        let Some((name, value)) = part.split_once('=') else {
            continue;
        };
        if name != key {
            continue;
        }
        if found.is_some() {
            return Err(());
        }
        let mut bytes = Vec::with_capacity(value.len());
        let input = value.as_bytes();
        let mut index = 0;
        while index < input.len() {
            match input[index] {
                b'%' if index + 2 < input.len() => {
                    let hex = std::str::from_utf8(&input[index + 1..index + 3]).map_err(|_| ())?;
                    bytes.push(u8::from_str_radix(hex, 16).map_err(|_| ())?);
                    index += 3;
                }
                b'%' => return Err(()),
                b'+' => {
                    bytes.push(b' ');
                    index += 1;
                }
                byte => {
                    bytes.push(byte);
                    index += 1;
                }
            }
        }
        found = Some(String::from_utf8(bytes).map_err(|_| ())?);
    }
    Ok(found)
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
                model_root: PathBuf::from("/models/one"),
            },
            PortalRepository {
                id: "second".into(),
                root: PathBuf::from("/two"),
                model_root: PathBuf::from("/models/two"),
            },
        ];
        assert_eq!(selected_root("", &roots), Some(Path::new("/models/one")));
        assert_eq!(
            selected_root("workspace=second", &roots),
            Some(Path::new("/models/two"))
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

    #[test]
    fn query_parameters_decode_once_and_reject_ambiguous_values() {
        assert_eq!(
            parameter("id=%2Fentity%3AUser", "id"),
            Ok(Some("/entity:User".into()))
        );
        assert_eq!(
            parameter("q=User+Portal", "q"),
            Ok(Some("User Portal".into()))
        );
        assert!(parameter("q=one&q=two", "q").is_err());
        assert!(parameter("q=%ZZ", "q").is_err());
        assert!(parameter("q=%FF", "q").is_err());
    }
}
