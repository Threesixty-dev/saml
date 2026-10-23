use crate::model::{
    Declaration, Document, Token, TokenKind, Vocabulary, VocabularyKind, RESTRICTED_KEYWORDS,
};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SourceSpan {
    pub uri: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
pub struct ScopePart {
    pub kind: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ConceptProperty {
    pub name: String,
    pub value_kind: Option<String>,
    pub definition: SourceSpan,
    pub value_kind_reference: Option<SourceSpan>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Concept {
    pub word: String,
    pub roles: Vec<String>,
    pub definitions: Vec<SourceSpan>,
    pub properties: Vec<ConceptProperty>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Materialization {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub scope: Vec<ScopePart>,
    pub boundaries: Vec<ScopePart>,
    pub definitions: Vec<SourceSpan>,
    pub occurrences: Vec<MaterializationOccurrence>,
    pub implicit: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct MaterializationOccurrence {
    pub source: SourceSpan,
    pub boundaries: Vec<ScopePart>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Link {
    pub relation: String,
    pub from: String,
    pub to: String,
    pub source: SourceSpan,
    pub crosses_scope: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug, Serialize)]
pub struct Finding {
    pub code: String,
    pub severity: Severity,
    pub message: String,
    pub source: SourceSpan,
}

#[derive(Clone, Debug, Serialize)]
pub struct SemanticMap {
    pub concepts: Vec<Concept>,
    pub materializations: Vec<Materialization>,
    pub links: Vec<Link>,
    pub findings: Vec<Finding>,
}

struct PendingLink {
    relation: String,
    from: String,
    source_scope: Vec<ScopePart>,
    target_kind: String,
    target_name: String,
    source: SourceSpan,
}

pub fn analyze(sources: &HashMap<String, String>) -> SemanticMap {
    let vocabulary = Vocabulary::from_sources(sources);
    let mut documents: Vec<Document> = sources
        .iter()
        .map(|(uri, text)| Document::with_vocabulary(uri.clone(), text.clone(), &vocabulary))
        .collect();
    documents.sort_by(|left, right| left.uri.cmp(&right.uri));

    let mut concepts = BTreeMap::new();
    for (word, entries) in &vocabulary.entries {
        let mut roles = Vec::new();
        if entries
            .iter()
            .any(|entry| entry.kind == VocabularyKind::Concept)
        {
            roles.push("concept".to_string());
        }
        if entries
            .iter()
            .any(|entry| entry.kind == VocabularyKind::Boundary)
        {
            roles.push("boundry".to_string());
        }
        concepts.insert(
            word.clone(),
            Concept {
                word: word.clone(),
                roles,
                definitions: entries
                    .iter()
                    .map(|entry| SourceSpan {
                        uri: entry.uri.clone(),
                        start: entry.start,
                        end: entry.end,
                    })
                    .collect(),
                properties: Vec::new(),
            },
        );
    }

    let mut findings = Vec::new();
    for document in documents.iter().filter(|doc| is_manifest(&doc.uri)) {
        for declaration in &document.declarations {
            if declaration.kind == "meta" {
                findings.push(Finding {
                    code: "obsolete-keyword".into(),
                    severity: Severity::Error,
                    message: "Use 'concept' instead of 'meta' in spec declarations".into(),
                    source: span(
                        document,
                        declaration.start,
                        declaration.start + declaration.kind.len(),
                    ),
                });
                continue;
            }
            if declaration.kind != "concept" {
                continue;
            }
            let word = lower_initial(&declaration.name);
            let Some(concept) = concepts.get_mut(&word) else {
                continue;
            };
            for property in spec_properties(document, declaration) {
                if let Some(previous) = concept
                    .properties
                    .iter()
                    .find(|item| item.name == property.name)
                {
                    if previous.value_kind != property.value_kind {
                        findings.push(Finding {
                            code: "conflicting-property-rule".into(),
                            severity: Severity::Error,
                            message: format!(
                                "Property '{}' of '{}' has conflicting value kinds '{}' and '{}'",
                                property.name,
                                word,
                                previous.value_kind.as_deref().unwrap_or("unspecified"),
                                property.value_kind.as_deref().unwrap_or("unspecified")
                            ),
                            source: property.definition,
                        });
                    }
                } else {
                    concept.properties.push(property);
                }
            }
        }
    }
    for concept in concepts.values() {
        for property in &concept.properties {
            let Some(value_kind) = &property.value_kind else {
                continue;
            };
            if !concepts.contains_key(value_kind) {
                findings.push(Finding {
                    code: "unknown-property-type".into(),
                    severity: Severity::Error,
                    message: format!(
                        "Property '{}' refers to undeclared vocabulary kind '{}'.",
                        property.name, value_kind
                    ),
                    source: property
                        .value_kind_reference
                        .clone()
                        .unwrap_or_else(|| property.definition.clone()),
                });
            }
        }
    }

    let mut materializations: BTreeMap<String, Materialization> = BTreeMap::new();
    let mut pending = Vec::new();
    let mut direct_properties: HashMap<String, HashSet<String>> = HashMap::new();
    for document in documents.iter().filter(|doc| !is_manifest(&doc.uri)) {
        let mut scopes: Vec<Vec<ScopePart>> = Vec::new();
        let mut boundaries: Vec<Vec<ScopePart>> = Vec::new();
        for declaration in &document.declarations {
            let mut scope = Vec::new();
            let mut boundary_path = Vec::new();
            if let Some(parent) = declaration.parent {
                scope = scopes[parent].clone();
                scope.push(part(&document.declarations[parent]));
                boundary_path = boundaries[parent].clone();
                let parent_part = part(&document.declarations[parent]);
                if is_boundary(&vocabulary, &parent_part.kind) {
                    push_unique(&mut boundary_path, parent_part);
                }
            }
            let qualifiers = header_qualifiers(document, declaration, &vocabulary);
            for qualifier in &qualifiers {
                if qualifier.kind != "layer" {
                    push_unique(&mut scope, qualifier.clone());
                }
                push_unique(&mut boundary_path, qualifier.clone());
            }
            let id = materialization_id(&scope, &declaration.kind, &declaration.name);
            let name_span = span(document, declaration.name_start, declaration.name_end);
            let materialization =
                materializations
                    .entry(id.clone())
                    .or_insert_with(|| Materialization {
                        id: id.clone(),
                        kind: declaration.kind.clone(),
                        name: declaration.name.clone(),
                        scope: scope.clone(),
                        boundaries: Vec::new(),
                        definitions: Vec::new(),
                        occurrences: Vec::new(),
                        implicit: false,
                    });
            materialization.implicit = false;
            materialization.definitions.push(name_span.clone());
            materialization.occurrences.push(MaterializationOccurrence {
                source: name_span,
                boundaries: boundary_path.clone(),
            });
            for boundary in &boundary_path {
                push_unique(&mut materialization.boundaries, boundary.clone());
            }
            scopes.push(scope.clone());
            boundaries.push(boundary_path);

            if let Some(parent) = declaration.parent {
                let parent_part = part(&document.declarations[parent]);
                if is_boundary(&vocabulary, &parent_part.kind) {
                    pending.push(PendingLink {
                        relation: "boundary".into(),
                        from: id.clone(),
                        source_scope: scope.clone(),
                        target_kind: parent_part.kind,
                        target_name: parent_part.name,
                        source: span(document, declaration.start, declaration.name_end),
                    });
                }
            }

            for qualifier in &qualifiers {
                if !matches!(qualifier.kind.as_str(), "scope" | "layer") {
                    continue;
                }
                let boundary_id = materialization_id(&[], &qualifier.kind, &qualifier.name);
                materializations
                    .entry(boundary_id.clone())
                    .or_insert_with(|| Materialization {
                        id: boundary_id,
                        kind: qualifier.kind.clone(),
                        name: qualifier.name.clone(),
                        scope: Vec::new(),
                        boundaries: Vec::new(),
                        definitions: Vec::new(),
                        occurrences: Vec::new(),
                        implicit: true,
                    });
            }

            if !concepts.contains_key(&declaration.kind)
                && !RESTRICTED_KEYWORDS.contains(&declaration.kind.as_str())
            {
                findings.push(Finding {
                    code: "unknown-vocabulary".into(),
                    severity: Severity::Warning,
                    message: format!(
                        "'{}' has no concept or boundry declaration in this workspace",
                        declaration.kind
                    ),
                    source: span(
                        document,
                        declaration.start,
                        declaration.start + declaration.kind.len(),
                    ),
                });
            }
            for qualifier in qualifiers {
                pending.push(PendingLink {
                    relation: "boundary".into(),
                    from: id.clone(),
                    source_scope: scope.clone(),
                    target_kind: qualifier.kind,
                    target_name: qualifier.name,
                    source: span(document, declaration.header_start, declaration.start),
                });
            }
            for token in header_extends(document, declaration) {
                pending.push(PendingLink {
                    relation: "extends".into(),
                    from: id.clone(),
                    source_scope: scope.clone(),
                    target_kind: declaration.kind.clone(),
                    target_name: token.value.clone(),
                    source: span(document, token.start, token.end),
                });
            }
            if let Some(concept) = concepts.get(&declaration.kind) {
                let context = {
                    let mut context = scope;
                    context.push(part(declaration));
                    context
                };
                for (key, value) in model_properties(document, declaration) {
                    if let Some(property) = concept
                        .properties
                        .iter()
                        .find(|property| property.name == key.value)
                    {
                        direct_properties
                            .entry(id.clone())
                            .or_default()
                            .insert(property.name.clone());
                        if let Some(value_kind) = &property.value_kind {
                            pending.push(PendingLink {
                                relation: format!("property:{}", property.name),
                                from: id.clone(),
                                source_scope: context.clone(),
                                target_kind: value_kind.clone(),
                                target_name: value.value.clone(),
                                source: span(document, value.start, value.end),
                            });
                        }
                    }
                }
            }
        }
    }

    let mut links = Vec::new();
    for reference in pending {
        match resolve(&materializations, &reference) {
            Ok(target) => {
                let from_scope = &materializations[&reference.from].scope;
                let crosses_scope = reference.relation != "boundary"
                    && from_scope != &target.scope
                    && !target.scope.starts_with(&reference.source_scope);
                links.push(Link {
                    relation: reference.relation,
                    from: reference.from,
                    to: target.id.clone(),
                    source: reference.source,
                    crosses_scope,
                });
            }
            Err(reason) => findings.push(Finding {
                code: reason.code().into(),
                severity: Severity::Error,
                message: reason.message(&reference),
                source: reference.source,
            }),
        }
    }
    let mut bases: HashMap<String, Vec<String>> = HashMap::new();
    for link in &links {
        if link.relation == "extends" {
            bases
                .entry(link.from.clone())
                .or_default()
                .push(link.to.clone());
        }
    }
    for materialization in materializations.values().filter(|item| !item.implicit) {
        let Some(concept) = concepts.get(&materialization.kind) else {
            continue;
        };
        for property in concept
            .properties
            .iter()
            .filter(|item| item.value_kind.is_some())
        {
            if !property_present(
                &materialization.id,
                &property.name,
                &direct_properties,
                &bases,
            ) {
                findings.push(Finding {
                    code: "missing-required-property".into(),
                    severity: Severity::Error,
                    message: format!(
                        "'{}' materialization of '{}' must provide property '{}' directly or through extends",
                        materialization.name, materialization.kind, property.name
                    ),
                    source: materialization.definitions[0].clone(),
                });
            }
        }
    }
    links.sort_by(|left, right| {
        (&left.from, &left.relation, &left.to, left.source.start).cmp(&(
            &right.from,
            &right.relation,
            &right.to,
            right.source.start,
        ))
    });
    findings.sort_by(|left, right| {
        (&left.source.uri, left.source.start, &left.code).cmp(&(
            &right.source.uri,
            right.source.start,
            &right.code,
        ))
    });
    SemanticMap {
        concepts: concepts.into_values().collect(),
        materializations: materializations.into_values().collect(),
        links,
        findings,
    }
}

fn is_manifest(uri: &str) -> bool {
    uri.ends_with(".spec") || uri.ends_with("/spec.saml")
}

fn lower_initial(name: &str) -> String {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    first.to_lowercase().collect::<String>() + chars.as_str()
}

fn span(document: &Document, start: usize, end: usize) -> SourceSpan {
    SourceSpan {
        uri: document.uri.clone(),
        start,
        end,
    }
}

fn part(declaration: &Declaration) -> ScopePart {
    ScopePart {
        kind: declaration.kind.clone(),
        name: declaration.name.clone(),
    }
}

fn push_unique(items: &mut Vec<ScopePart>, value: ScopePart) {
    if !items.contains(&value) {
        items.push(value);
    }
}

fn is_boundary(vocabulary: &Vocabulary, kind: &str) -> bool {
    vocabulary.entries.get(kind).is_some_and(|entries| {
        entries
            .iter()
            .any(|entry| entry.kind == VocabularyKind::Boundary)
    })
}

fn materialization_id(scope: &[ScopePart], kind: &str, name: &str) -> String {
    let mut id = String::new();
    for part in scope {
        id.push_str(&format!(
            "/{kind}:{name}",
            kind = part.kind,
            name = part.name
        ));
    }
    id.push_str(&format!("/{kind}:{name}"));
    id
}

fn header_tokens<'a>(document: &'a Document, declaration: &Declaration) -> Vec<&'a Token> {
    document
        .tokens()
        .iter()
        .filter(|token| {
            token.start >= declaration.header_start && token.end <= declaration.header_end
        })
        .collect()
}

fn header_qualifiers(
    document: &Document,
    declaration: &Declaration,
    vocabulary: &Vocabulary,
) -> Vec<ScopePart> {
    let tokens = header_tokens(document, declaration);
    let mut result = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        if token.start >= declaration.start || !is_boundary(vocabulary, &token.value) {
            continue;
        }
        if let Some(next) = tokens.get(index + 1) {
            if next.kind == TokenKind::Identifier
                && next.value.chars().next().is_some_and(char::is_uppercase)
            {
                push_unique(
                    &mut result,
                    ScopePart {
                        kind: token.value.clone(),
                        name: next.value.clone(),
                    },
                );
            } else if next.value == "[" {
                for item in tokens
                    .iter()
                    .skip(index + 2)
                    .take_while(|item| item.value != "]")
                {
                    if item.kind == TokenKind::Identifier {
                        push_unique(
                            &mut result,
                            ScopePart {
                                kind: token.value.clone(),
                                name: item.value.clone(),
                            },
                        );
                    }
                }
            }
        }
    }
    result
}

fn header_extends<'a>(document: &'a Document, declaration: &Declaration) -> Vec<&'a Token> {
    let tokens = header_tokens(document, declaration);
    let Some(index) = tokens.iter().position(|token| token.value == "extends") else {
        return Vec::new();
    };
    tokens
        .into_iter()
        .skip(index + 1)
        .take_while(|token| {
            token.kind == TokenKind::Identifier
                && token.value.chars().next().is_some_and(char::is_uppercase)
        })
        .collect()
}

fn spec_properties(document: &Document, declaration: &Declaration) -> Vec<ConceptProperty> {
    let tokens = document.tokens();
    let mut depth = 0;
    let mut properties = Vec::new();
    for index in 0..tokens.len() {
        let token = &tokens[index];
        if token.start < declaration.header_end || token.end > declaration.end {
            continue;
        }
        if token.value == "{" {
            depth += 1;
            continue;
        }
        if token.value == "}" {
            depth -= 1;
            continue;
        }
        if depth != 1 || token.kind != TokenKind::Identifier {
            continue;
        }
        if tokens.get(index + 1).is_none_or(|next| next.value != "{") {
            continue;
        }
        let mut field_depth = 1;
        let mut cursor = index + 2;
        let mut value_kind = None;
        let mut value_kind_reference = None;
        while cursor < tokens.len() && field_depth > 0 {
            match tokens[cursor].value.as_str() {
                "{" => field_depth += 1,
                "}" => field_depth -= 1,
                "type" if field_depth == 1 => {
                    let target = if tokens.get(cursor + 1).is_some_and(|next| next.value == ":") {
                        tokens.get(cursor + 2)
                    } else {
                        tokens.get(cursor + 1)
                    };
                    if let Some(target) = target.filter(|item| item.kind == TokenKind::Identifier) {
                        value_kind = Some(lower_initial(&target.value));
                        value_kind_reference = Some(span(document, target.start, target.end));
                    }
                }
                _ => {}
            }
            cursor += 1;
        }
        properties.push(ConceptProperty {
            name: token.value.clone(),
            value_kind,
            definition: span(document, token.start, token.end),
            value_kind_reference,
        });
    }
    properties
}

fn model_properties<'a>(
    document: &'a Document,
    declaration: &Declaration,
) -> Vec<(&'a Token, &'a Token)> {
    let tokens = document.tokens();
    let mut depth = 0;
    let mut properties = Vec::new();
    for index in 0..tokens.len() {
        let token = &tokens[index];
        if token.start < declaration.header_end || token.end > declaration.end {
            continue;
        }
        match token.value.as_str() {
            "{" => {
                depth += 1;
                continue;
            }
            "}" => {
                depth -= 1;
                continue;
            }
            _ => {}
        }
        if depth == 1
            && token.kind == TokenKind::Identifier
            && tokens.get(index + 1).is_some_and(|next| next.value == ":")
        {
            if let Some(value) = tokens
                .get(index + 2)
                .filter(|next| next.kind == TokenKind::Identifier)
            {
                properties.push((token, value));
            }
        }
    }
    properties
}

fn property_present(
    materialization_id: &str,
    property_name: &str,
    direct_properties: &HashMap<String, HashSet<String>>,
    bases: &HashMap<String, Vec<String>>,
) -> bool {
    let mut visited = HashSet::new();
    let mut pending = vec![materialization_id];
    while let Some(current) = pending.pop() {
        if !visited.insert(current) {
            continue;
        }
        if direct_properties
            .get(current)
            .is_some_and(|properties| properties.contains(property_name))
        {
            return true;
        }
        if let Some(parents) = bases.get(current) {
            pending.extend(parents.iter().map(String::as_str));
        }
    }
    false
}

enum ResolveError {
    Missing,
    WrongKind,
    Ambiguous,
}

impl ResolveError {
    fn code(&self) -> &'static str {
        match self {
            Self::Missing => "unresolved-reference",
            Self::WrongKind => "wrong-reference-kind",
            Self::Ambiguous => "ambiguous-reference",
        }
    }

    fn message(&self, reference: &PendingLink) -> String {
        match self {
            Self::Missing => format!("Cannot resolve '{}'", reference.target_name),
            Self::WrongKind => format!(
                "'{}' is not a '{}' materialization",
                reference.target_name, reference.target_kind
            ),
            Self::Ambiguous => format!(
                "'{}' has multiple '{}' materializations visible from this scope",
                reference.target_name, reference.target_kind
            ),
        }
    }
}

fn resolve<'a>(
    materializations: &'a BTreeMap<String, Materialization>,
    reference: &PendingLink,
) -> Result<&'a Materialization, ResolveError> {
    let same_name: Vec<&Materialization> = materializations
        .values()
        .filter(|materialization| materialization.name == reference.target_name)
        .collect();
    let candidates: Vec<&Materialization> = same_name
        .iter()
        .copied()
        .filter(|materialization| materialization.kind == reference.target_kind)
        .collect();
    if candidates.is_empty() {
        return Err(if same_name.is_empty() {
            ResolveError::Missing
        } else {
            ResolveError::WrongKind
        });
    }
    let visible: Vec<&Materialization> = candidates
        .iter()
        .copied()
        .filter(|materialization| reference.source_scope.starts_with(&materialization.scope))
        .collect();
    if !visible.is_empty() {
        let closest = visible
            .iter()
            .map(|materialization| materialization.scope.len())
            .max()
            .unwrap();
        let closest: Vec<&Materialization> = visible
            .into_iter()
            .filter(|materialization| materialization.scope.len() == closest)
            .collect();
        return if closest.len() == 1 {
            Ok(closest[0])
        } else {
            Err(ResolveError::Ambiguous)
        };
    }
    if candidates.len() == 1 {
        Ok(candidates[0])
    } else {
        Err(ResolveError::Ambiguous)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_property_can_be_inherited_through_extends() {
        let spec =
            "concept Environment {}\nconcept ApplicationType { environment { type: Environment } }";
        let model = "environment Browser {}\napplicationType Base { environment: Browser }\napplicationType Derived extends Base {}\napplicationType Missing {}";
        let map = analyze(&HashMap::from([
            ("file:///model.spec".into(), spec.into()),
            ("file:///model.saml".into(), model.into()),
        ]));
        let property = &map
            .concepts
            .iter()
            .find(|concept| concept.word == "applicationType")
            .unwrap()
            .properties[0];
        assert_eq!(property.name, "environment");
        assert_eq!(property.value_kind.as_deref(), Some("environment"));
        assert_eq!(
            property.definition.start,
            spec.find("environment {").unwrap()
        );
        assert_eq!(
            property.value_kind_reference.as_ref().unwrap().start,
            spec.find("type: Environment").unwrap() + "type: ".len()
        );
        let missing: Vec<_> = map
            .findings
            .iter()
            .filter(|finding| finding.code == "missing-required-property")
            .collect();
        assert_eq!(missing.len(), 1);
        assert!(missing[0].message.contains("'Missing'"));
    }

    #[test]
    fn old_meta_keyword_reports_rename() {
        let sources = HashMap::from([("file:///model.spec".into(), "meta Entity {}".into())]);
        let map = analyze(&sources);
        assert!(map.concepts.is_empty());
        assert!(map.findings.iter().any(|finding| {
            finding.code == "obsolete-keyword"
                && finding.message == "Use 'concept' instead of 'meta' in spec declarations"
        }));
    }

    #[test]
    fn concept_materializations_compose_with_boundaries_and_cross_scope_links() {
        let sources = HashMap::from([
            (
                "file:///model.spec".into(),
                "boundry Scope {}\nboundry Entity {}\nconcept Entity {}\nconcept Action { initiator { type: Entity } }".into(),
            ),
            (
                "file:///model.saml".into(),
                "scope Identity entity User {}\nscope Article action Publish { initiator: User }".into(),
            ),
        ]);
        let map = analyze(&sources);
        let entity = map
            .concepts
            .iter()
            .find(|item| item.word == "entity")
            .unwrap();
        assert_eq!(entity.roles, ["concept", "boundry"]);
        let user = map
            .materializations
            .iter()
            .find(|item| item.id == "/scope:Identity/entity:User")
            .unwrap();
        assert_eq!(user.scope[0].name, "Identity");
        assert!(map
            .materializations
            .iter()
            .any(|item| item.id == "/scope:Identity" && item.implicit));
        assert!(map.links.iter().any(|link| {
            link.relation == "property:initiator"
                && link.from == "/scope:Article/action:Publish"
                && link.to == user.id
                && link.crosses_scope
        }));
        assert!(map.findings.is_empty(), "{:?}", map.findings);
    }

    #[test]
    fn extends_resolves_nearest_scope_and_reports_ambiguous_crossings() {
        let sources = HashMap::from([
            ("file:///model.spec".into(), "boundry Scope {}\nconcept Stage {}".into()),
            (
                "file:///model.saml".into(),
                "scope Alpha stage Base {}\nscope Beta stage Base {}\nscope Alpha stage Child extends Base {}\nscope Gamma stage Other extends Base {}".into(),
            ),
        ]);
        let map = analyze(&sources);
        assert!(map.links.iter().any(|link| {
            link.relation == "extends"
                && link.from == "/scope:Alpha/stage:Child"
                && link.to == "/scope:Alpha/stage:Base"
        }));
        assert!(map
            .findings
            .iter()
            .any(|finding| finding.code == "ambiguous-reference"));
    }

    #[test]
    fn property_rules_enforce_target_concept_kind() {
        let sources = HashMap::from([
            (
                "file:///model.spec".into(),
                "concept Entity {}\nconcept Action { initiator { type: Entity } }".into(),
            ),
            (
                "file:///model.saml".into(),
                "action Publish { initiator: Publish }".into(),
            ),
        ]);
        let map = analyze(&sources);
        assert!(map
            .findings
            .iter()
            .any(|finding| finding.code == "wrong-reference-kind"));
    }

    #[test]
    fn repeated_materializations_keep_each_boundary_occurrence() {
        let sources = HashMap::from([
            (
                "file:///model.spec".into(),
                "boundry Layer {}\nboundry Scope {}".into(),
            ),
            (
                "file:///model.saml".into(),
                "layer Business scope Article {}\nlayer Infrastructure scope Article {}".into(),
            ),
        ]);
        let map = analyze(&sources);
        let article = map
            .materializations
            .iter()
            .find(|item| item.id == "/scope:Article")
            .unwrap();
        assert_eq!(article.occurrences.len(), 2);
        assert!(article.occurrences.iter().any(|occurrence| {
            occurrence
                .boundaries
                .iter()
                .any(|part| part.name == "Business")
        }));
        assert!(article.occurrences.iter().any(|occurrence| {
            occurrence
                .boundaries
                .iter()
                .any(|part| part.name == "Infrastructure")
        }));
        assert!(map.findings.is_empty(), "{:?}", map.findings);
    }

    #[test]
    fn nested_property_target_stays_in_its_parent_scope() {
        let sources = HashMap::from([
            (
                "file:///model.spec".into(),
                "concept WorkflowStage { state { type: State } }\nconcept State {}".into(),
            ),
            (
                "file:///model.saml".into(),
                "workflowStage Evaluation { state: Initial }".into(),
            ),
        ]);
        let map = analyze(&sources);
        let link = map
            .links
            .iter()
            .find(|link| link.relation == "property:state")
            .unwrap();
        assert!(!link.crosses_scope);
        assert!(map.findings.is_empty(), "{:?}", map.findings);
    }

    #[test]
    fn nested_materialization_links_to_parent_boundary() {
        let sources = HashMap::from([
            (
                "file:///model.spec".into(),
                "boundry Scope {}\nconcept Entity {}".into(),
            ),
            (
                "file:///model.saml".into(),
                "scope Article { entity Author {} }".into(),
            ),
        ]);
        let map = analyze(&sources);
        assert!(map.links.iter().any(|link| {
            link.relation == "boundary"
                && link.from == "/scope:Article/entity:Author"
                && link.to == "/scope:Article"
                && !link.crosses_scope
        }));
        assert!(map.findings.is_empty(), "{:?}", map.findings);
    }
}
