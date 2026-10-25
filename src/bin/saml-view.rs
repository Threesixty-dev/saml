use saml_lsp::diagnostics::diagnostic_report;
use saml_lsp::semantic::analyze;
use saml_lsp::workspace::collect_sources;
use serde_json::json;
use std::collections::HashMap;
use std::env;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

const HTML: &str = include_str!("../../viewer/index.html");
const CSS: &str = include_str!("../../viewer/style.css");
const SCRIPT: &str = include_str!("../../viewer/app.js");
const GUIDE: &str = include_str!("../../LLM_GUIDE.md");

fn main() -> io::Result<()> {
    let input_roots: Vec<PathBuf> = env::args_os().skip(1).map(PathBuf::from).collect();
    let input_roots = if input_roots.is_empty() {
        vec![env::current_dir()?]
    } else {
        input_roots
    };
    let mut roots = Vec::new();
    for input in input_roots {
        let root = input.canonicalize()?;
        if !root.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("workspace must be a directory: {}", root.display()),
            ));
        }
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    let port = env::var("SAML_VIEW_PORT")
        .ok()
        .map(|value| {
            value.parse::<u16>().map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("invalid port: {error}"),
                )
            })
        })
        .transpose()?
        .unwrap_or(8787);
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    println!("SAML semantic atlas: http://127.0.0.1:{port}");
    for (index, root) in roots.iter().enumerate() {
        println!("Workspace {index}: {}", root.display());
    }
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(error) = handle(stream, &roots) {
                    eprintln!("viewer request failed: {error}");
                }
            }
            Err(error) => eprintln!("viewer connection failed: {error}"),
        }
    }
    Ok(())
}

fn handle(mut stream: TcpStream, roots: &[PathBuf]) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    let mut request = String::new();
    BufReader::new(&stream).read_line(&mut request)?;
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
    match route {
        "/api/status" => {
            let body = serde_json::to_vec(&json!({
                "schema_version": 1,
                "service": "saml-view",
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
                .enumerate()
                .map(|(id, root)| {
                    json!({
                        "id": id,
                        "name": root.file_name().unwrap_or(root.as_os_str()).to_string_lossy(),
                        "path": root.to_string_lossy(),
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
            let Some(root) = selected_root(query, roots) else {
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
            let Some(root) = selected_root(query, roots) else {
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

fn selected_root<'a>(query: &str, roots: &'a [PathBuf]) -> Option<&'a Path> {
    let mut selected = None;
    for part in query.split('&') {
        if let Some(value) = part.strip_prefix("workspace=") {
            if selected.is_some() {
                return None;
            }
            selected = Some(value.parse::<usize>().ok()?);
        }
    }
    roots.get(selected.unwrap_or(0)).map(PathBuf::as_path)
}

fn respond(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_selection_is_bounded_and_unambiguous() {
        let roots = [PathBuf::from("/one"), PathBuf::from("/two")];
        assert_eq!(selected_root("", &roots), Some(Path::new("/one")));
        assert_eq!(
            selected_root("workspace=1", &roots),
            Some(Path::new("/two"))
        );
        assert_eq!(selected_root("workspace=2", &roots), None);
        assert_eq!(selected_root("workspace=abc", &roots), None);
        assert_eq!(selected_root("workspace=0&workspace=1", &roots), None);
    }
}
