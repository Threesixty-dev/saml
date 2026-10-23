use saml_lsp::model::{Document, Vocabulary, VocabularyKind, RESTRICTED_KEYWORDS};
use saml_lsp::semantic::{
    analyze, Concept, ConceptProperty, Link, Materialization, SemanticMap, Severity, SourceSpan,
    VocabularyUseKind,
};
use saml_lsp::workspace::{collect_sources, uri_to_path};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;

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
        let result = match self.dispatch(method, params) {
            Ok(result) => result,
            Err(error) if message.get("id").is_some() => {
                let code = if error.kind() == io::ErrorKind::InvalidInput {
                    -32602
                } else {
                    -32603
                };
                self.send(&json!({"jsonrpc": "2.0", "id": message["id"],
                    "error": {"code": code, "message": error.to_string()}}))?;
                return Ok(());
            }
            Err(error) => return Err(error),
        };
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
                    "referencesProvider": true,
                    "renameProvider": {"prepareProvider": true},
                    "hoverProvider": true,
                    "completionProvider": {},
                    "semanticTokensProvider": {
                        "legend": {"tokenTypes": ["keyword", "samlVocabulary", "property"],
                                   "tokenModifiers": ["declaration"]},
                        "full": true
                    }
                }, "serverInfo": {"name": "saml-lsp", "version": "0.3.0"}})))
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
                    self.refresh_diagnostics()?;
                }
                Ok(Some(Value::Null))
            }
            "saml/semanticMap" => Ok(Some(json!(analyze(&self.workspace_sources())))),
            "textDocument/documentSymbol" => {
                let result = params["textDocument"]["uri"]
                    .as_str()
                    .and_then(|uri| self.document(uri))
                    .map(|document| document.symbol_tree())
                    .unwrap_or_default();
                Ok(Some(json!(result)))
            }
            "textDocument/semanticTokens/full" => {
                let data = params["textDocument"]["uri"]
                    .as_str()
                    .and_then(|uri| self.document(uri))
                    .map(|document| document.semantic_tokens())
                    .unwrap_or_default();
                Ok(Some(json!({"data": data})))
            }
            "textDocument/definition" => {
                let sources = self.workspace_sources();
                let map = analyze(&sources);
                if let Some((_, property)) = self.property_at(params, &map) {
                    if let Some(text) = sources.get(&property.definition.uri) {
                        let document = Document::new(property.definition.uri.clone(), text.clone());
                        return Ok(Some(json!([{"uri": property.definition.uri,
                            "range": document.range(property.definition.start,
                                                    property.definition.end)}])));
                    }
                }
                if let Some((materialization, _)) = self.materialization_at(params, &map) {
                    return Ok(Some(json!(locations_for(
                        &sources,
                        materialization.definitions.iter()
                    ))));
                }
                if let Some((concept, _)) = self.concept_at(params, &map) {
                    return Ok(Some(json!(locations_for(
                        &sources,
                        concept.definitions.iter()
                    ))));
                }
                Ok(Some(json!([])))
            }
            "textDocument/references" => {
                let sources = self.workspace_sources();
                let map = analyze(&sources);
                let include_declaration = params["context"]["includeDeclaration"]
                    .as_bool()
                    .unwrap_or(false);
                if let Some((materialization, _)) = self.materialization_at(params, &map) {
                    let spans =
                        materialization_spans(&sources, &map, materialization, include_declaration);
                    return Ok(Some(json!(locations_for(&sources, spans.into_iter()))));
                }
                let Some((concept, _)) = self.vocabulary_at(params, &map) else {
                    return Ok(Some(json!([])));
                };
                let mut spans: Vec<&SourceSpan> =
                    concept.uses.iter().map(|usage| &usage.source).collect();
                if include_declaration {
                    spans.extend(&concept.definitions);
                }
                spans.sort_by_key(|source| (&source.uri, source.start, source.end));
                spans.dedup();
                let locations: Vec<Value> = spans
                    .into_iter()
                    .filter_map(|source| {
                        let text = sources.get(&source.uri)?;
                        let document = Document::new(source.uri.clone(), text.clone());
                        Some(json!({"uri": source.uri,
                            "range": document.range(source.start, source.end)}))
                    })
                    .collect();
                Ok(Some(json!(locations)))
            }
            "textDocument/prepareRename" => {
                let sources = self.workspace_sources();
                let map = analyze(&sources);
                if let Some((materialization, source)) = self.materialization_at(params, &map) {
                    if !materialization.implicit && materialization.kind != "state" {
                        if let Some(text) = sources.get(&source.uri) {
                            let document = Document::new(source.uri.clone(), text.clone());
                            return Ok(Some(json!({
                                "range": document.range(source.start, source.end),
                                "placeholder": materialization.name
                            })));
                        }
                    }
                    return Ok(Some(Value::Null));
                }
                let Some((concept, source)) = self.vocabulary_at(params, &map) else {
                    return Ok(Some(Value::Null));
                };
                let Some(text) = sources.get(&source.uri) else {
                    return Ok(Some(Value::Null));
                };
                let document = Document::new(source.uri.clone(), text.clone());
                Ok(Some(
                    json!({"range": document.range(source.start, source.end),
                    "placeholder": upper_initial(&concept.word)}),
                ))
            }
            "textDocument/rename" => {
                let sources = self.workspace_sources();
                let map = analyze(&sources);
                if let Some((materialization, _)) = self.materialization_at(params, &map) {
                    return rename_materialization(
                        &sources,
                        &map,
                        materialization,
                        params["newName"].as_str().unwrap_or_default(),
                    )
                    .map(Some);
                }
                let Some((concept, _)) = self.vocabulary_at(params, &map) else {
                    return Err(invalid_rename("No renameable symbol at this position"));
                };
                let (new_name, new_word) =
                    normalize_vocabulary_name(params["newName"].as_str().unwrap_or_default())?;
                if new_word != concept.word && map.concepts.iter().any(|item| item.word == new_word)
                {
                    return Err(invalid_rename(
                        "A concept or boundary already uses that name",
                    ));
                }
                if new_word != concept.word
                    && (map
                        .materializations
                        .iter()
                        .any(|item| item.kind == new_word)
                        || map
                            .concepts
                            .iter()
                            .flat_map(|item| &item.properties)
                            .any(|property| {
                                property.value_kind.as_deref() == Some(new_word.as_str())
                            }))
                {
                    return Err(invalid_rename(
                        "That vocabulary word is already used elsewhere in this workspace",
                    ));
                }
                let documents: HashMap<_, _> = sources
                    .iter()
                    .map(|(uri, text)| (uri.clone(), Document::new(uri.clone(), text.clone())))
                    .collect();
                let mut changes: BTreeMap<String, Vec<Value>> = BTreeMap::new();
                let mut add_edit = |source: &SourceSpan, replacement: &str| {
                    if let Some(document) = documents.get(&source.uri) {
                        changes.entry(source.uri.clone()).or_default().push(json!({
                            "range": document.range(source.start, source.end),
                            "newText": replacement,
                        }));
                    }
                };
                for definition in &concept.definitions {
                    add_edit(definition, &new_name);
                }
                for usage in &concept.uses {
                    let replacement = match usage.kind {
                        VocabularyUseKind::TypeReference => &new_name,
                        VocabularyUseKind::Materialization | VocabularyUseKind::Qualifier => {
                            &new_word
                        }
                    };
                    add_edit(&usage.source, replacement);
                }
                Ok(Some(json!({"changes": changes})))
            }
            "textDocument/hover" => {
                let sources = self.workspace_sources();
                let map = analyze(&sources);
                if let Some((concept, property)) = self.property_at(params, &map) {
                    let mut description = format!(
                        "property `{}` of concept `{}`",
                        property.name,
                        upper_initial(concept)
                    );
                    if let Some(value_kind) = &property.value_kind {
                        description.push_str(&format!(
                            "\n\nRequires {} materialization{} of `{}` (directly or through `extends`).",
                            if property.many { "a list of" } else { "a" },
                            if property.many { "s" } else { "" },
                            upper_initial(value_kind)
                        ));
                    }
                    return Ok(Some(json!({"contents": {"kind": "markdown",
                        "value": description}})));
                }
                if let Some((materialization, _)) = self.materialization_at(params, &map) {
                    return Ok(Some(json!({"contents": {"kind": "markdown",
                        "value": format!("{} `{}`\n\n`{}`", materialization.kind,
                            materialization.name, materialization.id)}})));
                }
                if let Some((concept, _)) = self.concept_at(params, &map) {
                    return Ok(Some(json!({"contents": {"kind": "markdown",
                        "value": format!("spec vocabulary `{}`: {}", concept.word,
                            concept.roles.join(" and "))}})));
                }
                let Some(name) = self.target_name(params) else {
                    return Ok(Some(Value::Null));
                };
                if RESTRICTED_KEYWORDS.contains(&name.as_str()) {
                    Ok(Some(json!({"contents": {"kind": "markdown",
                        "value": format!("built-in keyword `{name}`")}})))
                } else {
                    Ok(Some(Value::Null))
                }
            }
            "textDocument/completion" => {
                let mut items: Vec<Value> = RESTRICTED_KEYWORDS
                    .iter()
                    .map(|keyword| json!({"label": keyword, "kind": 14}))
                    .collect();
                let vocabulary = Vocabulary::from_sources(&self.workspace_sources());
                for (word, entries) in &vocabulary.entries {
                    if !RESTRICTED_KEYWORDS.contains(&word.as_str()) {
                        items.push(json!({"label": word,
                            "kind": if entries.iter().all(|entry| entry.kind == VocabularyKind::Boundary) { 9 } else { 7 }}));
                    }
                }
                let names: BTreeSet<String> = self
                    .workspace_documents()
                    .iter()
                    .flat_map(|document| &document.declarations)
                    .map(|declaration| declaration.name.clone())
                    .collect();
                for name in names {
                    if !RESTRICTED_KEYWORDS.contains(&name.as_str())
                        && !vocabulary.entries.contains_key(&name)
                    {
                        items.push(json!({"label": name, "kind": 6}));
                    }
                }
                Ok(Some(json!(items)))
            }
            _ => Ok(None),
        }
    }

    fn update(&mut self, uri: &str, text: &str) -> io::Result<()> {
        let mut sources = self.workspace_sources();
        sources.insert(uri.into(), text.into());
        let vocabulary = Vocabulary::from_sources(&sources);
        let document = Document::with_vocabulary(uri.into(), text.into(), &vocabulary);
        self.open.insert(uri.into(), document);
        self.refresh_diagnostics()
    }

    fn refresh_diagnostics(&mut self) -> io::Result<()> {
        let sources = self.workspace_sources();
        let vocabulary = Vocabulary::from_sources(&sources);
        let map = analyze(&sources);
        let documents: Vec<Document> = self
            .open
            .iter()
            .map(|(uri, document)| {
                Document::with_vocabulary(uri.clone(), document.text().into(), &vocabulary)
            })
            .collect();
        for document in documents {
            let mut diagnostics = document.diagnostics.clone();
            for finding in map
                .findings
                .iter()
                .filter(|finding| finding.source.uri == document.uri)
            {
                diagnostics.push(json!({
                    "range": document.range(finding.source.start, finding.source.end),
                    "severity": if finding.severity == Severity::Error { 1 } else { 2 },
                    "source": "saml semantic",
                    "code": finding.code,
                    "message": finding.message,
                }));
            }
            self.publish(&document.uri, &diagnostics)?;
        }
        Ok(())
    }

    fn publish(&mut self, uri: &str, diagnostics: &[Value]) -> io::Result<()> {
        self.send(
            &json!({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics",
                          "params": {"uri": uri, "diagnostics": diagnostics}}),
        )
    }

    fn document(&self, uri: &str) -> Option<Document> {
        let sources = self.workspace_sources();
        let vocabulary = Vocabulary::from_sources(&sources);
        let text = sources.get(uri).cloned().or_else(|| {
            let path = uri_to_path(uri)?;
            fs::read_to_string(path).ok()
        })?;
        Some(Document::with_vocabulary(uri.into(), text, &vocabulary))
    }

    fn target_name(&self, params: &Value) -> Option<String> {
        let uri = params["textDocument"]["uri"].as_str()?;
        let document = self.document(uri)?;
        let position = &params["position"];
        let offset = document.offset(position["line"].as_u64()?, position["character"].as_u64()?);
        document.identifier_at(offset).map(str::to_owned)
    }

    fn property_at<'a>(
        &self,
        params: &Value,
        map: &'a SemanticMap,
    ) -> Option<(&'a str, &'a ConceptProperty)> {
        let uri = params["textDocument"]["uri"].as_str()?;
        let document = self.document(uri)?;
        let position = &params["position"];
        let offset = document.offset(position["line"].as_u64()?, position["character"].as_u64()?);
        for concept in &map.concepts {
            for property in &concept.properties {
                if property.definition.uri == uri
                    && property.definition.start <= offset
                    && offset < property.definition.end
                {
                    return Some((&concept.word, property));
                }
            }
        }
        let (name, kind) = document.property_key_at(offset)?;
        let concept = map.concepts.iter().find(|concept| concept.word == kind)?;
        concept
            .properties
            .iter()
            .find(|property| property.name == name)
            .map(|property| (concept.word.as_str(), property))
    }

    fn concept_at<'a>(
        &self,
        params: &Value,
        map: &'a SemanticMap,
    ) -> Option<(&'a Concept, &'a SourceSpan)> {
        let uri = params["textDocument"]["uri"].as_str()?;
        let document = self.document(uri)?;
        let position = &params["position"];
        let offset = document.offset(position["line"].as_u64()?, position["character"].as_u64()?);
        map.concepts.iter().find_map(|concept| {
            concept
                .definitions
                .iter()
                .chain(concept.uses.iter().map(|usage| &usage.source))
                .find(|source| source.uri == uri && source.start <= offset && offset < source.end)
                .map(|source| (concept, source))
        })
    }

    fn vocabulary_at<'a>(
        &self,
        params: &Value,
        map: &'a SemanticMap,
    ) -> Option<(&'a Concept, &'a SourceSpan)> {
        let (concept, source) = self.concept_at(params, map)?;
        // `state:` is both a property key and a materialization declaration.
        (concept.word != "state" && !RESTRICTED_KEYWORDS.contains(&concept.word.as_str()))
            .then_some((concept, source))
    }

    fn materialization_at<'a>(
        &self,
        params: &Value,
        map: &'a SemanticMap,
    ) -> Option<(&'a Materialization, &'a SourceSpan)> {
        let uri = params["textDocument"]["uri"].as_str()?;
        let document = self.document(uri)?;
        let position = &params["position"];
        let offset = document.offset(position["line"].as_u64()?, position["character"].as_u64()?);
        let contains = |source: &SourceSpan| {
            source.uri == uri && source.start <= offset && offset < source.end
        };
        if let Some(found) = map.materializations.iter().find_map(|materialization| {
            materialization
                .definitions
                .iter()
                .find(|source| contains(source))
                .map(|source| (materialization, source))
        }) {
            return Some(found);
        }
        map.links.iter().find_map(|link| {
            if !contains(&link.source) {
                return None;
            }
            let materialization = map
                .materializations
                .iter()
                .find(|materialization| materialization.id == link.to)?;
            is_name_reference(link, materialization, document.text())
                .then_some((materialization, &link.source))
        })
    }

    fn workspace_documents(&self) -> Vec<Document> {
        let sources = self.workspace_sources();
        let vocabulary = Vocabulary::from_sources(&sources);
        let mut result: Vec<Document> = sources
            .into_iter()
            .map(|(uri, text)| Document::with_vocabulary(uri, text, &vocabulary))
            .collect();
        result.sort_by(|left, right| left.uri.cmp(&right.uri));
        result
    }

    fn workspace_sources(&self) -> HashMap<String, String> {
        let mut sources = HashMap::new();
        for root in &self.roots {
            collect_sources(root, &mut sources);
        }
        for (uri, document) in &self.open {
            sources.insert(uri.clone(), document.text().into());
        }
        sources
    }
}

fn is_manifest(uri: &str) -> bool {
    uri.ends_with(".spec") || uri.ends_with("/spec.saml")
}

fn is_name_reference(link: &Link, target: &Materialization, text: &str) -> bool {
    link.relation != "boundary"
        || text.get(link.source.start..link.source.end) == Some(target.name.as_str())
}

fn materialization_spans<'a>(
    sources: &HashMap<String, String>,
    map: &'a SemanticMap,
    materialization: &'a Materialization,
    include_declarations: bool,
) -> Vec<&'a SourceSpan> {
    let mut spans: Vec<&SourceSpan> = map
        .links
        .iter()
        .filter(|link| {
            link.to == materialization.id
                && sources
                    .get(&link.source.uri)
                    .is_some_and(|text| is_name_reference(link, materialization, text))
                && !materialization.definitions.contains(&link.source)
        })
        .map(|link| &link.source)
        .collect();
    if include_declarations {
        spans.extend(&materialization.definitions);
    }
    spans.sort_by_key(|source| (&source.uri, source.start, source.end));
    spans.dedup();
    spans
}

fn locations_for<'a>(
    sources: &HashMap<String, String>,
    spans: impl IntoIterator<Item = &'a SourceSpan>,
) -> Vec<Value> {
    let mut spans: Vec<_> = spans.into_iter().collect();
    spans.sort_by_key(|source| (&source.uri, source.start, source.end));
    spans.dedup();
    spans
        .into_iter()
        .filter_map(|source| {
            let text = sources.get(&source.uri)?;
            let document = Document::new(source.uri.clone(), text.clone());
            Some(json!({"uri": source.uri,
                "range": document.range(source.start, source.end)}))
        })
        .collect()
}

fn rename_materialization(
    sources: &HashMap<String, String>,
    map: &SemanticMap,
    materialization: &Materialization,
    new_name: &str,
) -> io::Result<Value> {
    if materialization.implicit || materialization.kind == "state" {
        return Err(invalid_rename(
            "This materialization cannot be renamed safely yet",
        ));
    }
    let mut chars = new_name.chars();
    if !chars.next().is_some_and(char::is_uppercase)
        || !chars.all(|ch| ch.is_alphanumeric() || ch == '_')
    {
        return Err(invalid_rename(
            "Materialization name must begin with an uppercase letter and contain only letters, digits, or underscores",
        ));
    }
    if new_name != materialization.name
        && map.materializations.iter().any(|other| {
            other.id != materialization.id
                && other.kind == materialization.kind
                && other.name == new_name
        })
    {
        return Err(invalid_rename(
            "Another materialization of this kind already has that name",
        ));
    }
    if map.findings.iter().any(|finding| {
        matches!(
            finding.code.as_str(),
            "unresolved-reference"
                | "ambiguous-reference"
                | "inaccessible-reference"
                | "wrong-reference-kind"
        ) && sources
            .get(&finding.source.uri)
            .and_then(|text| text.get(finding.source.start..finding.source.end))
            .is_some_and(|name| name == materialization.name || name == new_name)
    }) {
        return Err(invalid_rename(
            "Resolve other references with this name before renaming",
        ));
    }
    if new_name != materialization.name
        && [materialization.name.as_str(), new_name]
            .into_iter()
            .any(|name| has_untracked_occurrences(sources, map, name))
    {
        return Err(invalid_rename(
            "Cannot rename while this name has untyped or otherwise untracked uses",
        ));
    }
    let spans = materialization_spans(sources, map, materialization, true);
    let mut changes: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for source in spans {
        if let Some(text) = sources.get(&source.uri) {
            let document = Document::new(source.uri.clone(), text.clone());
            changes.entry(source.uri.clone()).or_default().push(json!({
                "range": document.range(source.start, source.end),
                "newText": new_name,
            }));
        }
    }
    Ok(json!({"changes": changes}))
}

fn has_untracked_occurrences(
    sources: &HashMap<String, String>,
    map: &SemanticMap,
    name: &str,
) -> bool {
    let mut known: HashSet<(String, usize, usize)> = map
        .materializations
        .iter()
        .flat_map(|materialization| &materialization.definitions)
        .map(|source| (source.uri.clone(), source.start, source.end))
        .collect();
    for link in &map.links {
        let Some(target) = map
            .materializations
            .iter()
            .find(|materialization| materialization.id == link.to)
        else {
            continue;
        };
        if sources
            .get(&link.source.uri)
            .is_some_and(|text| is_name_reference(link, target, text))
        {
            known.insert((link.source.uri.clone(), link.source.start, link.source.end));
        }
    }
    sources
        .iter()
        .filter(|(uri, _)| !is_manifest(uri))
        .any(|(uri, text)| {
            Document::new(uri.clone(), text.clone())
                .identifier_spans(name)
                .into_iter()
                .any(|(start, end)| !known.contains(&(uri.clone(), start, end)))
        })
}

fn upper_initial(word: &str) -> String {
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    first.to_uppercase().collect::<String>() + chars.as_str()
}

fn lower_initial(word: &str) -> String {
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    first.to_lowercase().collect::<String>() + chars.as_str()
}

fn invalid_rename(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn normalize_vocabulary_name(input: &str) -> io::Result<(String, String)> {
    let mut chars = input.chars();
    let Some(first) = chars.next() else {
        return Err(invalid_rename("Name cannot be empty"));
    };
    if !first.is_alphabetic() || !chars.all(|ch| ch.is_alphanumeric() || ch == '_') {
        return Err(invalid_rename(
            "Name must be an identifier beginning with a letter",
        ));
    }
    let name = upper_initial(input);
    let word = lower_initial(&name);
    if RESTRICTED_KEYWORDS.contains(&word.as_str())
        || matches!(word.as_str(), "extends" | "uses" | "via")
    {
        return Err(invalid_rename("Name conflicts with a SAML keyword"));
    }
    Ok((name, word))
}

fn main() -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    Server::new(BufReader::new(stdin.lock()), stdout.lock()).run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use saml_lsp::workspace::path_to_uri;
    use std::io::Cursor;
    use std::path::Path;

    #[test]
    fn rename_concept_updates_workspace_uses_without_touching_text_matches() {
        let spec_uri = "file:///model.spec";
        let model_uri = "file:///model.saml";
        let spec = "boundary Interface {}\nconcept Interface {}\nconcept Endpoint { target { type: Interface } }\nconcept Contract {}";
        let model = "interface RestApi {}\ninterface Public endpoint Entry { target: RestApi }\ninterface Interface {}\n// interface Comment\nendpoint Other { note: \"Interface\" }";
        let mut server = Server::new(BufReader::new(Cursor::new(Vec::new())), Vec::new());
        for (uri, text) in [(spec_uri, spec), (model_uri, model)] {
            server
                .dispatch(
                    "textDocument/didOpen",
                    &json!({"textDocument": {"uri": uri, "text": text}}),
                )
                .unwrap();
        }
        let position = json!({"textDocument": {"uri": model_uri},
            "position": {"line": 0, "character": 2}});
        let prepared = server
            .dispatch("textDocument/prepareRename", &position)
            .unwrap()
            .unwrap();
        assert_eq!(prepared["placeholder"], "Interface");
        assert_eq!(prepared["range"]["start"]["character"], 0);
        let materialization_name = server
            .dispatch(
                "textDocument/prepareRename",
                &json!({"textDocument": {"uri": model_uri},
                    "position": {"line": 2, "character": 11}}),
            )
            .unwrap();
        assert_eq!(materialization_name.unwrap()["placeholder"], "Interface");
        let references = server
            .dispatch(
                "textDocument/references",
                &json!({"textDocument": {"uri": model_uri},
                    "position": {"line": 0, "character": 2},
                    "context": {"includeDeclaration": false}}),
            )
            .unwrap()
            .unwrap();
        let references = references.as_array().unwrap();
        assert_eq!(references.len(), 4);
        assert_eq!(
            references
                .iter()
                .filter(|item| item["uri"] == spec_uri)
                .count(),
            1
        );
        assert_eq!(
            references
                .iter()
                .filter(|item| item["uri"] == model_uri)
                .count(),
            3
        );
        let references_with_declarations = server
            .dispatch(
                "textDocument/references",
                &json!({"textDocument": {"uri": spec_uri},
                    "position": {"line": 1, "character": 10},
                    "context": {"includeDeclaration": true}}),
            )
            .unwrap()
            .unwrap();
        let references_with_declarations = references_with_declarations.as_array().unwrap();
        assert_eq!(references_with_declarations.len(), 6);
        assert_eq!(references_with_declarations[0]["range"]["start"]["line"], 0);
        assert_eq!(references_with_declarations[1]["range"]["start"]["line"], 1);
        let name_references = server
            .dispatch(
                "textDocument/references",
                &json!({"textDocument": {"uri": model_uri},
                    "position": {"line": 2, "character": 11},
                    "context": {"includeDeclaration": true}}),
            )
            .unwrap();
        assert_eq!(name_references.unwrap().as_array().unwrap().len(), 1);
        let edit = server
            .dispatch(
                "textDocument/rename",
                &json!({"textDocument": {"uri": model_uri},
                    "position": {"line": 0, "character": 2}, "newName": "Contracted"}),
            )
            .unwrap()
            .unwrap();
        assert_eq!(edit["changes"][spec_uri].as_array().unwrap().len(), 3);
        assert_eq!(edit["changes"][model_uri].as_array().unwrap().len(), 3);
        assert_eq!(apply_edits(spec_uri, spec, &edit["changes"][spec_uri]),
            "boundary Contracted {}\nconcept Contracted {}\nconcept Endpoint { target { type: Contracted } }\nconcept Contract {}");
        assert_eq!(apply_edits(model_uri, model, &edit["changes"][model_uri]),
            "contracted RestApi {}\ncontracted Public endpoint Entry { target: RestApi }\ncontracted Interface {}\n// interface Comment\nendpoint Other { note: \"Interface\" }");

        let collision = server.dispatch(
            "textDocument/rename",
            &json!({"textDocument": {"uri": model_uri},
                "position": {"line": 0, "character": 2}, "newName": "Contract"}),
        );
        assert!(collision.is_err());
        let invalid = server.dispatch(
            "textDocument/rename",
            &json!({"textDocument": {"uri": model_uri},
                "position": {"line": 0, "character": 2}, "newName": "Two Words"}),
        );
        assert!(invalid.is_err());
        let reserved = server.dispatch(
            "textDocument/rename",
            &json!({"textDocument": {"uri": model_uri},
                "position": {"line": 0, "character": 2}, "newName": "Boundary"}),
        );
        assert!(reserved.is_err());
        server
            .dispatch(
                "textDocument/didChange",
                &json!({"textDocument": {"uri": model_uri},
                    "contentChanges": [{"text": format!("{model}\ncontracted Ghost {{}}") }]}),
            )
            .unwrap();
        let unknown_word_collision = server.dispatch(
            "textDocument/rename",
            &json!({"textDocument": {"uri": model_uri},
                "position": {"line": 0, "character": 2}, "newName": "Contracted"}),
        );
        assert!(unknown_word_collision.is_err());
    }

    fn apply_edits(uri: &str, text: &str, edits: &Value) -> String {
        let document = Document::new(uri.into(), text.into());
        let mut changes: Vec<_> = edits
            .as_array()
            .unwrap()
            .iter()
            .map(|edit| {
                let start = &edit["range"]["start"];
                let end = &edit["range"]["end"];
                (
                    document.offset(
                        start["line"].as_u64().unwrap(),
                        start["character"].as_u64().unwrap(),
                    ),
                    document.offset(
                        end["line"].as_u64().unwrap(),
                        end["character"].as_u64().unwrap(),
                    ),
                    edit["newText"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        changes.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
        let mut result = text.to_owned();
        for (start, end, replacement) in changes {
            result.replace_range(start..end, &replacement);
        }
        result
    }

    fn at_line(uri: &str, text: &str, line: usize, name: &str) -> Value {
        let content = text.lines().nth(line).unwrap();
        let start = content.find(name).unwrap();
        let character = content[..start + 1].encode_utf16().count();
        json!({"textDocument": {"uri": uri},
            "position": {"line": line, "character": character}})
    }

    #[test]
    fn materialization_navigation_and_rename_follow_resolved_identity() {
        let spec_uri = "file:///model.spec";
        let model_uri = "file:///model.saml";
        let spec = "boundary Scope {}\nconcept Data {}\nconcept Action { target { type: Data } }\nconcept Holder { asset { type: Data } }";
        let model = "scope Alpha Public data Record {}\nscope Beta Public data Record {}\nscope Beta Public data Other {}\ndata Catalog { entry: { type: Data } }\nscope Alpha action UseAlpha { target: Record }\nscope Beta action UseBeta { target: Record }\nscope Alpha data Derived extends Record {}\nscope Alpha holder H { asset: Catalog { entry: Record } }\n// Record";
        let mut server = Server::new(BufReader::new(Cursor::new(Vec::new())), Vec::new());
        for (uri, text) in [(spec_uri, spec), (model_uri, model)] {
            server
                .dispatch(
                    "textDocument/didOpen",
                    &json!({"textDocument": {"uri": uri, "text": text}}),
                )
                .unwrap();
        }
        let alpha = at_line(model_uri, model, 4, "Record");
        let beta = at_line(model_uri, model, 5, "Record");
        let binding = at_line(model_uri, model, 7, "Record");
        for (position, declaration_line) in [(&alpha, 0), (&beta, 1), (&binding, 0)] {
            let locations = server
                .dispatch("textDocument/definition", position)
                .unwrap()
                .unwrap();
            assert_eq!(locations.as_array().unwrap().len(), 1);
            assert_eq!(locations[0]["range"]["start"]["line"], declaration_line);
        }
        let own_definition = server
            .dispatch(
                "textDocument/definition",
                &at_line(model_uri, model, 0, "Record"),
            )
            .unwrap()
            .unwrap();
        assert_eq!(own_definition.as_array().unwrap().len(), 1);
        assert_eq!(own_definition[0]["range"]["start"]["line"], 0);
        let hover = server
            .dispatch("textDocument/hover", &alpha)
            .unwrap()
            .unwrap();
        assert!(hover["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("/scope:Alpha/data:Record"));
        let references = |server: &mut Server<_, _>, include_declaration| {
            server
                .dispatch(
                    "textDocument/references",
                    &json!({"textDocument": {"uri": model_uri},
                        "position": alpha["position"],
                        "context": {"includeDeclaration": include_declaration}}),
                )
                .unwrap()
                .unwrap()
        };
        let uses = references(&mut server, false);
        let lines: Vec<_> = uses
            .as_array()
            .unwrap()
            .iter()
            .map(|location| location["range"]["start"]["line"].as_u64().unwrap())
            .collect();
        assert_eq!(lines, [4, 6, 7]);
        let with_declaration = references(&mut server, true);
        assert_eq!(with_declaration.as_array().unwrap().len(), 4);
        assert_eq!(with_declaration[0]["range"]["start"]["line"], 0);
        let prepared = server
            .dispatch("textDocument/prepareRename", &binding)
            .unwrap()
            .unwrap();
        assert_eq!(prepared["placeholder"], "Record");
        let edit = server
            .dispatch(
                "textDocument/rename",
                &json!({"textDocument": {"uri": model_uri},
                    "position": binding["position"], "newName": "Revised"}),
            )
            .unwrap()
            .unwrap();
        assert_eq!(edit["changes"][model_uri].as_array().unwrap().len(), 4);
        let changed = apply_edits(model_uri, model, &edit["changes"][model_uri]);
        assert!(changed.contains("scope Alpha Public data Revised {}"));
        assert!(changed.contains("scope Beta Public data Record {}"));
        assert!(changed.contains("scope Beta action UseBeta { target: Record }"));
        assert!(changed.contains("scope Alpha data Derived extends Revised {}"));
        assert!(changed.contains("entry: Revised"));
        assert!(changed.ends_with("// Record"));
        for new_name in ["Other", "lowercase", "Two Words"] {
            assert!(server
                .dispatch(
                    "textDocument/rename",
                    &json!({"textDocument": {"uri": model_uri},
                    "position": alpha["position"], "newName": new_name})
                )
                .is_err());
        }

        let updated = format!("{model}\nscope Gamma action Ambiguous {{ target: Record }}\nscope Alpha data Secret {{}}\nscope Beta action Blocked {{ target: Secret }}");
        server
            .dispatch(
                "textDocument/didChange",
                &json!({"textDocument": {"uri": model_uri},
                "contentChanges": [{"text": updated}]}),
            )
            .unwrap();
        for (line, name) in [(9, "Record"), (11, "Secret")] {
            let position = at_line(model_uri, &updated, line, name);
            let definitions = server
                .dispatch("textDocument/definition", &position)
                .unwrap()
                .unwrap();
            assert_eq!(definitions, json!([]));
            let prepared = server
                .dispatch("textDocument/prepareRename", &position)
                .unwrap()
                .unwrap();
            assert_eq!(prepared, Value::Null);
        }
        assert!(server
            .dispatch(
                "textDocument/rename",
                &json!({"textDocument": {"uri": model_uri},
                "position": alpha["position"], "newName": "Revised"})
            )
            .is_err());
    }

    #[test]
    fn repeated_materialization_declarations_are_renamed_together() {
        let first_uri = "file:///first.saml";
        let second_uri = "file:///second.saml";
        let first = "data Shared {}";
        let second = "data Shared {}\naction Use { target: Shared }";
        let mut server = Server::new(BufReader::new(Cursor::new(Vec::new())), Vec::new());
        for (uri, text) in [
            (
                "file:///model.spec",
                "concept Data {}\nconcept Action { target { type: Data } }",
            ),
            (first_uri, first),
            (second_uri, second),
        ] {
            server
                .dispatch(
                    "textDocument/didOpen",
                    &json!({"textDocument": {"uri": uri, "text": text}}),
                )
                .unwrap();
        }
        let reference = at_line(second_uri, second, 1, "Shared");
        let definitions = server
            .dispatch("textDocument/definition", &reference)
            .unwrap()
            .unwrap();
        assert_eq!(definitions.as_array().unwrap().len(), 2);
        assert_eq!(definitions[0]["uri"], first_uri);
        assert_eq!(definitions[1]["uri"], second_uri);
        let references = server
            .dispatch(
                "textDocument/references",
                &json!({"textDocument": {"uri": second_uri},
                "position": reference["position"],
                "context": {"includeDeclaration": true}}),
            )
            .unwrap()
            .unwrap();
        assert_eq!(references.as_array().unwrap().len(), 3);
        let edit = server
            .dispatch(
                "textDocument/rename",
                &json!({"textDocument": {"uri": second_uri},
                "position": reference["position"], "newName": "Renamed"}),
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            apply_edits(first_uri, first, &edit["changes"][first_uri]),
            "data Renamed {}"
        );
        assert_eq!(
            apply_edits(second_uri, second, &edit["changes"][second_uri]),
            "data Renamed {}\naction Use { target: Renamed }"
        );
    }

    #[test]
    fn explicit_boundary_qualifier_names_are_precise_references() {
        let uri = "file:///model.saml";
        let model = "scope Alpha {}\nscope Alpha data One {}\nscope [Alpha, Beta] data Two {}";
        let mut server = Server::new(BufReader::new(Cursor::new(Vec::new())), Vec::new());
        for (source_uri, text) in [
            ("file:///model.spec", "boundary Scope {}\nconcept Data {}"),
            (uri, model),
        ] {
            server
                .dispatch(
                    "textDocument/didOpen",
                    &json!({"textDocument": {"uri": source_uri, "text": text}}),
                )
                .unwrap();
        }
        let qualifier = at_line(uri, model, 2, "Alpha");
        let definitions = server
            .dispatch("textDocument/definition", &qualifier)
            .unwrap()
            .unwrap();
        assert_eq!(definitions.as_array().unwrap().len(), 1);
        assert_eq!(definitions[0]["range"]["start"]["line"], 0);
        let references = server
            .dispatch(
                "textDocument/references",
                &json!({"textDocument": {"uri": uri},
                "position": qualifier["position"],
                "context": {"includeDeclaration": false}}),
            )
            .unwrap()
            .unwrap();
        assert_eq!(references.as_array().unwrap().len(), 2);
        let edit = server
            .dispatch(
                "textDocument/rename",
                &json!({"textDocument": {"uri": uri},
                "position": qualifier["position"], "newName": "Renamed"}),
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            apply_edits(uri, model, &edit["changes"][uri]),
            "scope Renamed {}\nscope Renamed data One {}\nscope [Renamed, Beta] data Two {}"
        );
    }

    #[test]
    fn materialization_rename_rejects_untracked_identifiers() {
        let uri = "file:///model.saml";
        let model = "data Shared {}\naction Use { target: Shared note: Mentioned }";
        let mut server = Server::new(BufReader::new(Cursor::new(Vec::new())), Vec::new());
        for (source_uri, text) in [
            (
                "file:///model.spec",
                "concept Data {}\nconcept Action { target { type: Data } }",
            ),
            (uri, model),
        ] {
            server
                .dispatch(
                    "textDocument/didOpen",
                    &json!({"textDocument": {"uri": source_uri, "text": text}}),
                )
                .unwrap();
        }
        let position = at_line(uri, model, 0, "Shared");
        let rename = |server: &mut Server<_, _>, new_name| {
            server.dispatch(
                "textDocument/rename",
                &json!({"textDocument": {"uri": uri},
                    "position": position["position"], "newName": new_name}),
            )
        };
        assert!(rename(&mut server, "Mentioned").is_err());
        assert!(rename(&mut server, "Renamed").is_ok());
        server
            .dispatch(
                "textDocument/didChange",
                &json!({"textDocument": {"uri": uri},
                "contentChanges": [{"text": model.replace("note: Mentioned", "note: Shared") }]}),
            )
            .unwrap();
        assert!(rename(&mut server, "Renamed").is_err());
    }

    #[test]
    fn typed_concept_reference_does_not_navigate_to_same_named_materialization() {
        let spec_uri = "file:///model.spec";
        let model_uri = "file:///model.saml";
        let model = "data Data {}\ndata Catalog { item: { type: Data } }";
        let mut server = Server::new(BufReader::new(Cursor::new(Vec::new())), Vec::new());
        for (uri, text) in [(spec_uri, "concept Data {}"), (model_uri, model)] {
            server
                .dispatch(
                    "textDocument/didOpen",
                    &json!({"textDocument": {"uri": uri, "text": text}}),
                )
                .unwrap();
        }
        let position = at_line(model_uri, model, 1, "Data");
        let definitions = server
            .dispatch("textDocument/definition", &position)
            .unwrap()
            .unwrap();
        assert_eq!(definitions.as_array().unwrap().len(), 1);
        assert_eq!(definitions[0]["uri"], spec_uri);
        let hover = server
            .dispatch("textDocument/hover", &position)
            .unwrap()
            .unwrap();
        assert_eq!(
            hover["contents"]["value"],
            "spec vocabulary `data`: concept"
        );
    }

    #[test]
    fn state_declaration_is_not_a_separate_reference_or_rename_target() {
        let uri = "file:///model.saml";
        let model = "workflowStage Evaluation { state: Initial }";
        let mut server = Server::new(BufReader::new(Cursor::new(Vec::new())), Vec::new());
        for (source_uri, text) in [
            (
                "file:///model.spec",
                "concept State {}\nconcept WorkflowStage { state { type: State } }",
            ),
            (uri, model),
        ] {
            server
                .dispatch(
                    "textDocument/didOpen",
                    &json!({"textDocument": {"uri": source_uri, "text": text}}),
                )
                .unwrap();
        }
        let position = at_line(uri, model, 0, "Initial");
        let references = server
            .dispatch(
                "textDocument/references",
                &json!({"textDocument": {"uri": uri},
                "position": position["position"],
                "context": {"includeDeclaration": false}}),
            )
            .unwrap()
            .unwrap();
        assert_eq!(references, json!([]));
        let prepared = server
            .dispatch("textDocument/prepareRename", &position)
            .unwrap()
            .unwrap();
        assert_eq!(prepared, Value::Null);
    }

    #[test]
    fn state_rename_is_disabled_until_implicit_materializations_are_modeled() {
        let uri = "file:///model.spec";
        let mut server = Server::new(BufReader::new(Cursor::new(Vec::new())), Vec::new());
        server
            .dispatch(
                "textDocument/didOpen",
                &json!({"textDocument": {"uri": uri, "text": "concept State {}"}}),
            )
            .unwrap();
        let prepared = server
            .dispatch(
                "textDocument/prepareRename",
                &json!({"textDocument": {"uri": uri},
                    "position": {"line": 0, "character": 9}}),
            )
            .unwrap();
        assert_eq!(prepared, Some(Value::Null));
    }

    #[test]
    fn property_hover_and_definition_do_not_resolve_to_value_concept() {
        let spec_uri = "file:///model.spec";
        let model_uri = "file:///model.saml";
        let spec = "concept Environment {}\nconcept ApplicationType {\n  environment {\n    type: Environment\n  }\n}";
        let model =
            "environment Browser {}\napplicationType ApiService {\n  environment: Browser\n}";
        let mut server = Server::new(BufReader::new(Cursor::new(Vec::new())), Vec::new());
        for (uri, text) in [(spec_uri, spec), (model_uri, model)] {
            server
                .dispatch(
                    "textDocument/didOpen",
                    &json!({"textDocument": {"uri": uri, "text": text}}),
                )
                .unwrap();
        }
        let spec_property = json!({"textDocument": {"uri": spec_uri},
            "position": {"line": 2, "character": 4}});
        let model_property = json!({"textDocument": {"uri": model_uri},
            "position": {"line": 2, "character": 4}});
        for position in [&spec_property, &model_property] {
            let hover = server
                .dispatch("textDocument/hover", position)
                .unwrap()
                .unwrap();
            assert_eq!(hover["contents"]["value"],
                "property `environment` of concept `ApplicationType`\n\nRequires a materialization of `Environment` (directly or through `extends`).");
            let definitions = server
                .dispatch("textDocument/definition", position)
                .unwrap()
                .unwrap();
            assert_eq!(definitions.as_array().unwrap().len(), 1);
            assert_eq!(definitions[0]["uri"], spec_uri);
            assert_eq!(definitions[0]["range"]["start"]["line"], 2);
        }
        let value_kind = json!({"textDocument": {"uri": spec_uri},
            "position": {"line": 3, "character": 11}});
        let definitions = server
            .dispatch("textDocument/definition", &value_kind)
            .unwrap()
            .unwrap();
        assert_eq!(definitions[0]["range"]["start"]["line"], 0);
        let tokens = server
            .dispatch(
                "textDocument/semanticTokens/full",
                &json!({"textDocument": {"uri": spec_uri}}),
            )
            .unwrap()
            .unwrap();
        assert!(tokens["data"]
            .as_array()
            .unwrap()
            .chunks(5)
            .any(|token| { token[3] == 2 && token[4] == 1 }));
    }

    #[test]
    fn example_is_structurally_valid() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("project/example-1");
        let mut sources = HashMap::new();
        collect_sources(&root, &mut sources);
        assert_eq!(sources.len(), 6);
        let vocabulary = Vocabulary::from_sources(&sources);
        let documents: Vec<Document> = sources
            .into_iter()
            .map(|(uri, text)| Document::with_vocabulary(uri, text, &vocabulary))
            .collect();
        assert!(documents
            .iter()
            .all(|document| document.diagnostics.is_empty()));
        assert!(documents
            .iter()
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
                    "uri": "file:///model.spec", "text": "concept Entity {}"
                }}),
            )
            .unwrap();
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
    fn spec_edits_change_vocabulary_and_semantic_tokens() {
        let mut server = Server::new(BufReader::new(Cursor::new(Vec::new())), Vec::new());
        server
            .dispatch(
                "textDocument/didOpen",
                &json!({"textDocument": {
                    "uri": "file:///model.spec", "text": "boundary Scope {}\nconcept Entity {}"
                }}),
            )
            .unwrap();
        server
            .dispatch(
                "textDocument/didOpen",
                &json!({"textDocument": {
                    "uri": "file:///model.saml", "text": "scope Sample { entity Item {} }"
                }}),
            )
            .unwrap();
        let params = json!({"textDocument": {"uri": "file:///model.saml"}});
        let symbols = server
            .dispatch("textDocument/documentSymbol", &params)
            .unwrap()
            .unwrap();
        assert_eq!(symbols[0]["name"], "Sample");
        assert_eq!(symbols[0]["children"][0]["name"], "Item");
        let tokens = server
            .dispatch("textDocument/semanticTokens/full", &params)
            .unwrap()
            .unwrap();
        let types: Vec<u64> = tokens["data"]
            .as_array()
            .unwrap()
            .chunks(5)
            .map(|chunk| chunk[3].as_u64().unwrap())
            .collect();
        assert_eq!(types, vec![1, 1]);
        server
            .dispatch(
                "textDocument/didChange",
                &json!({
                    "textDocument": {"uri": "file:///model.spec"},
                    "contentChanges": [{"text": "boundary Scope {}\nconcept Other {}"}]
                }),
            )
            .unwrap();
        let tokens = server
            .dispatch("textDocument/semanticTokens/full", &params)
            .unwrap()
            .unwrap();
        assert_eq!(tokens["data"].as_array().unwrap().len(), 5);
    }

    #[test]
    fn manifest_marks_reserved_word_and_declared_name_separately() {
        let uri = "file:///model.spec".to_string();
        let text = "concept Entity {}".to_string();
        let sources = HashMap::from([(uri.clone(), text.clone())]);
        let vocabulary = Vocabulary::from_sources(&sources);
        let document = Document::with_vocabulary(uri, text, &vocabulary);
        let data = document.semantic_tokens();
        assert_eq!(data.len(), 10);
        assert_eq!(data[3..5], [0, 0]);
        assert_eq!(data[8..10], [1, 1]);
    }

    #[test]
    fn entity_composes_concept_and_boundary_roles() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("project/example-1");
        let uri = path_to_uri(&root.join("articles.saml"));
        let manifest_uri = path_to_uri(&root.join("spec.saml"));
        let articles = std::fs::read_to_string(root.join("articles.saml")).unwrap();
        let target_line = articles
            .lines()
            .position(|line| line.trim_start().starts_with("entity Author"))
            .unwrap();
        let target_character = articles
            .lines()
            .nth(target_line)
            .unwrap()
            .find("entity Author")
            .unwrap()
            + 1;
        let spec = std::fs::read_to_string(root.join("spec.saml")).unwrap();
        let declaration_line = |prefix: &str| {
            spec.lines()
                .position(|line| line.trim_start().starts_with(prefix))
                .unwrap()
        };
        let mut server = Server::new(BufReader::new(Cursor::new(Vec::new())), Vec::new());
        server.roots.push(root);
        let target = json!({
            "textDocument": {"uri": uri},
            "position": {"line": target_line, "character": target_character}
        });
        let locations = server
            .dispatch("textDocument/definition", &target)
            .unwrap()
            .unwrap();
        assert_eq!(locations.as_array().unwrap().len(), 2);
        assert!(locations.as_array().unwrap().iter().any(|location| {
            location["uri"] == manifest_uri
                && location["range"]["start"]["line"] == declaration_line("concept Entity {")
        }));
        assert!(locations.as_array().unwrap().iter().any(|location| {
            location["uri"] == manifest_uri
                && location["range"]["start"]["line"] == declaration_line("boundary Entity {")
        }));
        let hover = server
            .dispatch("textDocument/hover", &target)
            .unwrap()
            .unwrap();
        assert_eq!(
            hover["contents"]["value"],
            "spec vocabulary `entity`: concept and boundary"
        );
    }

    #[test]
    fn opening_a_spec_refreshes_model_semantic_diagnostics() {
        let mut server = Server::new(BufReader::new(Cursor::new(Vec::new())), Vec::new());
        server
            .dispatch(
                "textDocument/didOpen",
                &json!({"textDocument": {
                    "uri": "file:///model.saml", "text": "entity Item {}"
                }}),
            )
            .unwrap();
        assert!(String::from_utf8_lossy(&server.writer).contains("unknown-vocabulary"));
        server.writer.clear();
        server
            .dispatch(
                "textDocument/didOpen",
                &json!({"textDocument": {
                    "uri": "file:///model.spec", "text": "concept Entity {}"
                }}),
            )
            .unwrap();
        let messages = String::from_utf8_lossy(&server.writer);
        assert!(messages.contains("file:///model.saml"));
        assert!(!messages.contains("unknown-vocabulary"));
        let map = server
            .dispatch("saml/semanticMap", &Value::Null)
            .unwrap()
            .unwrap();
        assert_eq!(map["concepts"][0]["word"], "entity");
        assert_eq!(map["materializations"][0]["name"], "Item");
    }
}
