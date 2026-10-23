use saml_lsp::semantic::analyze;
use saml_lsp::workspace::collect_sources;
use std::collections::HashMap;
use std::env;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

const HTML: &str = include_str!("../../viewer/index.html");
const CSS: &str = include_str!("../../viewer/style.css");
const SCRIPT: &str = include_str!("../../viewer/app.js");

fn main() -> io::Result<()> {
    let mut args = env::args_os().skip(1);
    let root = args
        .next()
        .map(PathBuf::from)
        .unwrap_or(env::current_dir()?);
    if args.next().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: saml-view [workspace-directory]",
        ));
    }
    let root = root.canonicalize()?;
    if !root.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "workspace must be a directory",
        ));
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
    println!("Workspace: {}", root.display());
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(error) = handle(stream, &root) {
                    eprintln!("viewer request failed: {error}");
                }
            }
            Err(error) => eprintln!("viewer connection failed: {error}"),
        }
    }
    Ok(())
}

fn handle(mut stream: TcpStream, root: &Path) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    let mut request = String::new();
    BufReader::new(&stream).read_line(&mut request)?;
    let mut parts = request.split_whitespace();
    let method = parts.next().unwrap_or("");
    let route = parts.next().unwrap_or("").split('?').next().unwrap_or("");
    if method != "GET" {
        return respond(
            &mut stream,
            "405 Method Not Allowed",
            "text/plain",
            b"GET only",
        );
    }
    match route {
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
        "/api/map" => {
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
        "/favicon.ico" => respond(&mut stream, "204 No Content", "text/plain", b""),
        _ => respond(&mut stream, "404 Not Found", "text/plain", b"Not found"),
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
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)
}
