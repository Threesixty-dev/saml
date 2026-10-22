mod model;

use model::{Document, KEYWORDS};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

struct Server<R: BufRead, W: Write> {
    reader: R,
    writer: W,
    roots: Vec<PathBuf>,
    open: HashMap<String, Document>,
    stopped: bool,
}

impl<R: BufRead, W: Write> Server<R, W> {
    fn new(reader: R, writer: W) -> Self {
        Self {
            reader,
            writer,
            roots: Vec::new(),
            open: HashMap::new(),
            stopped: false,
        }
    }

    fn run(&mut self) -> io::Result<()> {
        while !self.stopped {
            let Some(message) = self.receive()? else {
                break;
            };
            self.handle(&message)?;
        }
        Ok(())
    }

    fn receive(&mut self) -> io::Result<Option<Value>> {
        let mut length = None;
        loop {
            let mut line = String::new();
            if self.reader.read_line(&mut line)? == 0 {
                return Ok(None);
            }
            if line == "\r\n" || line == "\n" {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                if name.eq_ignore_ascii_case("Content-Length") {
                    length = value.trim().parse::<usize>().ok();
                }
            }
        }
        let length = length
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length"))?;
        if length > 16 * 1024 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "message too large",
            ));
        }
        let mut bytes = vec![0; length];
        self.reader.read_exact(&mut bytes)?;
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    fn send(&mut self, message: &Value) -> io::Result<()> {
        let bytes = serde_json::to_vec(message)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        write!(self.writer, "Content-Length: {}\r\n\r\n", bytes.len())?;
        self.writer.write_all(&bytes)?;
        self.writer.flush()
    }

    fn handle(&mut self, message: &Value) -> io::Result<()> {
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            return Ok(());
        };
        let params = message.get("params").unwrap_or(&Value::Null);
        let result = self.dispatch(method, params)?;
        if let Some(id) = message.get("id") {
            let reply = match result {
                Some(value) => json!({"jsonrpc": "2.0", "id": id, "result": value}),
                None => json!({"jsonrpc": "2.0", "id": id, "error": {
                    "code": -32601, "message": "Method not found"
                }}),
            };
            self.send(&reply)?;
        }
        Ok(())
    }

    fn dispatch(&mut self, method: &str, params: &Value) -> io::Result<Option<Value>> {
        match method {
            "initialize" => {
                let folders = params["workspaceFolders"].as_array();
                self.roots = folders
                    .into_iter()
                    .flatten()
                    .filter_map(|folder| folder["uri"].as_str().and_then(uri_to_path))
                    .collect();
                if self.roots.is_empty() {
                    if let Some(path) = params["rootUri"].as_str().and_then(uri_to_path) {
                        self.roots.push(path);
                    } else if let Some(path) = params["rootPath"].as_str() {
                        self.roots.push(PathBuf::from(path));
                    }
                }
                Ok(Some(json!({"capabilities": {
                    "textDocumentSync": 1,
                    "documentSymbolProvider": true,
                    "definitionProvider": true,
                    "hoverProvider": true,
                    "completionProvider": {}
                }, "serverInfo": {"name": "saml-lsp", "version": "0.1.0"}})))
            }
            "shutdown" => Ok(Some(Value::Null)),
            "exit" => {
                self.stopped = true;
                Ok(Some(Value::Null))
            }
            "initialized" | "$/cancelRequest" | "workspace/didChangeConfiguration" => {
                Ok(Some(Value::Null))
            }
            "textDocument/didOpen" => {
                if let (Some(uri), Some(text)) = (
                    params["textDocument"]["uri"].as_str(),
                    params["textDocument"]["text"].as_str(),
                ) {
                    self.update(uri, text)?;
                }
                Ok(Some(Value::Null))
            }
            "textDocument/didChange" => {
                if let (Some(uri), Some(changes)) = (
                    params["textDocument"]["uri"].as_str(),
                    params["contentChanges"].as_array(),
                ) {
                    if let Some(text) = changes.last().and_then(|change| change["text"].as_str()) {
                        self.update(uri, text)?;
                    }
                }
                Ok(Some(Value::Null))
            }
            "textDocument/didClose" => {
                if let Some(uri) = params["textDocument"]["uri"].as_str() {
                    self.open.remove(uri);
                    self.publish(uri, &[])?;
                }
                Ok(Some(Value::Null))
            }
            "textDocument/documentSymbol" => {
                let result = params["textDocument"]["uri"]
                    .as_str()
                    .and_then(|uri| self.document(uri))
                    .map(|document| document.symbol_tree())
                    .unwrap_or_default();
                Ok(Some(json!(result)))
            }
            "textDocument/definition" => {
                let Some(name) = self.target_name(params) else {
                    return Ok(Some(json!([])));
                };
                let mut locations = Vec::new();
                for document in self.workspace_documents() {
                    for declaration in &document.declarations {
                        if declaration.name == name {
                            locations.push(json!({"uri": document.uri,
                                "range": document.range(declaration.name_start,
                                                        declaration.name_end)}));
                        }
                    }
                }
                Ok(Some(json!(locations)))
            }
            "textDocument/hover" => {
                let Some(name) = self.target_name(params) else {
                    return Ok(Some(Value::Null));
                };
                let descriptions: BTreeSet<String> = self
                    .workspace_documents()
                    .iter()
                    .flat_map(|document| &document.declarations)
                    .filter(|declaration| declaration.name == name)
                    .map(|declaration| format!("{} `{}`", declaration.kind, declaration.name))
                    .collect();
                if descriptions.is_empty() {
                    Ok(Some(Value::Null))
                } else {
                    Ok(Some(json!({"contents": {"kind": "markdown",
                        "value": descriptions.into_iter().collect::<Vec<_>>().join("\n\n")}})))
                }
            }
            "textDocument/completion" => {
                let mut items: Vec<Value> = KEYWORDS
                    .iter()
                    .map(|keyword| json!({"label": keyword, "kind": 14}))
                    .collect();
                let names: BTreeSet<String> = self
                    .workspace_documents()
                    .iter()
                    .flat_map(|document| &document.declarations)
                    .map(|declaration| declaration.name.clone())
                    .collect();
                for name in names {
                    if !KEYWORDS.contains(&name.as_str()) {
                        items.push(json!({"label": name, "kind": 6}));
                    }
                }
                Ok(Some(json!(items)))
            }
            _ => Ok(None),
        }
    }

    fn update(&mut self, uri: &str, text: &str) -> io::Result<()> {
        let document = Document::new(uri.into(), text.into());
        self.publish(uri, &document.diagnostics)?;
        self.open.insert(uri.into(), document);
        Ok(())
    }

    fn publish(&mut self, uri: &str, diagnostics: &[Value]) -> io::Result<()> {
        self.send(
            &json!({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics",
                          "params": {"uri": uri, "diagnostics": diagnostics}}),
        )
    }

    fn document(&self, uri: &str) -> Option<Document> {
        if let Some(document) = self.open.get(uri) {
            return Some(document.clone());
        }
        let path = uri_to_path(uri)?;
        Some(Document::new(uri.into(), fs::read_to_string(path).ok()?))
    }

    fn target_name(&self, params: &Value) -> Option<String> {
        let uri = params["textDocument"]["uri"].as_str()?;
        let document = self.document(uri)?;
        let position = &params["position"];
        let offset = document.offset(position["line"].as_u64()?, position["character"].as_u64()?);
        document.identifier_at(offset).map(str::to_owned)
    }

    fn workspace_documents(&self) -> Vec<Document> {
        let mut documents: HashMap<String, Document> = self.open.clone();
        for root in &self.roots {
            collect_documents(root, &mut documents);
        }
        let mut result: Vec<Document> = documents.into_values().collect();
        result.sort_by(|left, right| left.uri.cmp(&right.uri));
        result
    }
}

fn collect_documents(root: &Path, documents: &mut HashMap<String, Document>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || matches!(name.as_ref(), "node_modules" | "target" | "venv") {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            collect_documents(&path, documents);
        } else if file_type.is_file() && path.extension().is_some_and(|ext| ext == "saml") {
            let uri = path_to_uri(&path);
            if !documents.contains_key(&uri) {
                if let Ok(text) = fs::read_to_string(&path) {
                    documents.insert(uri.clone(), Document::new(uri, text));
                }
            }
        }
    }
}

fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let bytes = rest.as_bytes();
    let mut result = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let value = u8::from_str_radix(&rest[index + 1..index + 3], 16).ok()?;
            result.push(value);
            index += 3;
        } else {
            result.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(result).ok().map(PathBuf::from)
}

fn path_to_uri(path: &Path) -> String {
    let mut result = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            result.push(byte as char);
        } else {
            result.push_str(&format!("%{byte:02X}"));
        }
    }
    result
}

fn main() -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    Server::new(BufReader::new(stdin.lock()), stdout.lock()).run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn example_is_structurally_valid() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("project/example-1");
        let mut documents = HashMap::new();
        collect_documents(&root, &mut documents);
        assert_eq!(documents.len(), 6);
        assert!(documents
            .values()
            .all(|document| document.diagnostics.is_empty()));
        assert!(documents
            .values()
            .flat_map(|document| &document.declarations)
            .any(|declaration| declaration.kind == "state" && declaration.name == "Successful"));
    }

    #[test]
    fn utf16_positions_and_diagnostics() {
        let document = Document::new("file:///x.saml".into(), "// 😀\nentity 😀Name {".into());
        assert_eq!(document.position(19), json!({"line": 1, "character": 9}));
        assert_eq!(document.offset(1, 9), 19);
        assert_eq!(document.diagnostics.len(), 1);
        assert_eq!(document.diagnostics[0]["message"], "Unclosed '{'");
    }

    #[test]
    fn protocol_handles_initialize_and_shutdown() {
        let mut input = Vec::new();
        for message in [
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "shutdown"}),
            json!({"jsonrpc": "2.0", "method": "exit"}),
        ] {
            let body = serde_json::to_vec(&message).unwrap();
            write!(input, "Content-Length: {}\r\n\r\n", body.len()).unwrap();
            input.extend(body);
        }
        let mut server = Server::new(BufReader::new(Cursor::new(input)), Vec::new());
        server.run().unwrap();
        assert!(server.stopped);
        assert!(String::from_utf8(server.writer)
            .unwrap()
            .contains("saml-lsp"));
    }

    #[test]
    fn unsaved_changes_are_used_for_symbols() {
        let mut server = Server::new(BufReader::new(Cursor::new(Vec::new())), Vec::new());
        server
            .dispatch(
                "textDocument/didOpen",
                &json!({"textDocument": {
                    "uri": "file:///draft.saml", "text": "entity Draft {}"
                }}),
            )
            .unwrap();
        let symbols = server
            .dispatch(
                "textDocument/documentSymbol",
                &json!({
                    "textDocument": {"uri": "file:///draft.saml"}
                }),
            )
            .unwrap()
            .unwrap();
        assert_eq!(symbols[0]["name"], "Draft");
        server
            .dispatch(
                "textDocument/didChange",
                &json!({
                    "textDocument": {"uri": "file:///draft.saml"},
                    "contentChanges": [{"text": "entity Changed {"}]
                }),
            )
            .unwrap();
        assert_eq!(server.open["file:///draft.saml"].diagnostics.len(), 1);
    }

    #[test]
    fn definition_resolves_across_example_files() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("project/example-1");
        let uri = path_to_uri(&root.join("applications.saml"));
        let mut server = Server::new(BufReader::new(Cursor::new(Vec::new())), Vec::new());
        server.roots.push(root);
        let locations = server
            .dispatch(
                "textDocument/definition",
                &json!({
                    "textDocument": {"uri": uri},
                    "position": {"line": 14, "character": 16}
                }),
            )
            .unwrap()
            .unwrap();
        assert_eq!(locations.as_array().unwrap().len(), 1);
        assert_eq!(locations[0]["range"]["start"]["line"], 3);
    }
}
