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
    pub many: bool,
    pub definition: SourceSpan,
    pub definitions: Vec<SourceSpan>,
    pub uses: Vec<SourceSpan>,
    pub value_kind_reference: Option<SourceSpan>,
}

#[derive(Clone)]
struct ModelAssignment {
    key: Token,
    value: PropertyValue,
}

#[derive(Clone)]
enum PropertyValue {
    Reference(Token),
    List(Vec<Token>, bool),
    Record(Vec<ModelAssignment>),
    Binding(Token, Vec<ModelAssignment>),
}

#[derive(Clone, Debug, Serialize)]
pub struct Concept {
    pub word: String,
    pub roles: Vec<String>,
    pub definitions: Vec<SourceSpan>,
    pub properties: Vec<ConceptProperty>,
    pub uses: Vec<VocabularyUse>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VocabularyUseKind {
    Materialization,
    Qualifier,
    TypeReference,
}

#[derive(Clone, Debug, Serialize)]
pub struct VocabularyUse {
    pub source: SourceSpan,
    pub kind: VocabularyUseKind,
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
    pub fields: Vec<MaterializationField>,
    pub parameters: Vec<MaterializationParameter>,
    pub implicit: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct MaterializationField {
    pub name: String,
    pub occurrences: Vec<MaterializationFieldOccurrence>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MaterializationFieldOccurrence {
    pub source: SourceSpan,
    pub fragment: SourceSpan,
}

#[derive(Clone, Debug, Serialize)]
pub struct MaterializationParameter {
    pub name: String,
    pub kind: String,
    pub many: bool,
    pub source: SourceSpan,
    pub definitions: Vec<SourceSpan>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MaterializationOccurrence {
    pub source: SourceSpan,
    pub boundaries: Vec<ScopePart>,
    pub visibility: Visibility,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    Public,
    Private,
}

#[derive(Clone, Debug, Serialize)]
pub struct Link {
    pub relation: String,
    pub from: String,
    pub to: String,
    pub source: SourceSpan,
    pub crosses_scope: bool,
    pub crosses_boundary: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Binding {
    pub from: String,
    pub property: String,
    pub target: String,
    pub source: SourceSpan,
    pub arguments: Vec<BindingArgument>,
}

#[derive(Clone, Debug, Serialize)]
pub struct BindingArgument {
    pub name: String,
    pub source: SourceSpan,
    pub values: Vec<BindingValue>,
}

#[derive(Clone, Debug, Serialize)]
pub struct BindingValue {
    pub name: String,
    pub target: Option<String>,
    pub source: SourceSpan,
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
    pub bindings: Vec<Binding>,
    pub findings: Vec<Finding>,
}

struct PendingLink {
    relation: String,
    from: String,
    source_scope: Vec<ScopePart>,
    source_boundaries: Vec<ScopePart>,
    target_kind: String,
    target_name: String,
    source: SourceSpan,
}

struct PendingBinding {
    from: String,
    property: String,
    target_source: SourceSpan,
    source_scope: Vec<ScopePart>,
    source_boundaries: Vec<ScopePart>,
    arguments: Vec<ModelAssignment>,
}

struct HeaderQualifier {
    part: ScopePart,
    source: SourceSpan,
}

struct ParameterSpec {
    kind: String,
    many: bool,
    source: SourceSpan,
    definitions: Vec<SourceSpan>,
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
            roles.push("boundary".to_string());
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
                uses: Vec::new(),
            },
        );
    }

    let mut findings = Vec::new();
    let mut vocabulary_uses: HashMap<String, Vec<VocabularyUse>> = HashMap::new();
    for document in documents.iter().filter(|doc| is_manifest(&doc.uri)) {
        for declaration in &document.declarations {
            let replacement = match declaration.kind.as_str() {
                "meta" => Some("concept"),
                "boundry" => Some("boundary"),
                _ => None,
            };
            if let Some(replacement) = replacement {
                findings.push(Finding {
                    code: "obsolete-keyword".into(),
                    severity: Severity::Error,
                    message: format!(
                        "Use '{}' instead of '{}' in spec declarations",
                        replacement, declaration.kind
                    ),
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
                if let (Some(kind), Some(reference)) =
                    (&property.value_kind, &property.value_kind_reference)
                {
                    vocabulary_uses
                        .entry(kind.clone())
                        .or_default()
                        .push(VocabularyUse {
                            source: reference.clone(),
                            kind: VocabularyUseKind::TypeReference,
                        });
                }
                if let Some(previous) = concept
                    .properties
                    .iter_mut()
                    .find(|item| item.name == property.name)
                {
                    if previous.value_kind != property.value_kind || previous.many != property.many
                    {
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
                            source: property.definition.clone(),
                        });
                    }
                    previous.definitions.push(property.definition);
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
    let mut pending_bindings = Vec::new();
    let mut direct_properties: HashMap<String, HashSet<String>> = HashMap::new();
    let mut parameter_specs: HashMap<String, HashMap<String, ParameterSpec>> = HashMap::new();
    for document in documents.iter().filter(|doc| !is_manifest(&doc.uri)) {
        let mut scopes: Vec<Vec<ScopePart>> = Vec::new();
        let mut boundaries: Vec<Vec<ScopePart>> = Vec::new();
        for declaration in &document.declarations {
            if concepts.contains_key(&declaration.kind) {
                if let Some(keyword) = declaration_keyword(document, declaration) {
                    vocabulary_uses
                        .entry(declaration.kind.clone())
                        .or_default()
                        .push(VocabularyUse {
                            source: span(document, keyword.start, keyword.end),
                            kind: VocabularyUseKind::Materialization,
                        });
                }
            }
            for token in boundary_qualifier_tokens(document, declaration, &vocabulary) {
                vocabulary_uses
                    .entry(token.value.clone())
                    .or_default()
                    .push(VocabularyUse {
                        source: span(document, token.start, token.end),
                        kind: VocabularyUseKind::Qualifier,
                    });
            }
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
                if qualifier.part.kind != "layer" {
                    push_unique(&mut scope, qualifier.part.clone());
                }
                push_unique(&mut boundary_path, qualifier.part.clone());
            }
            let id = materialization_id(&scope, &declaration.kind, &declaration.name);
            let name_span = span(document, declaration.name_start, declaration.name_end);
            let mut reference_boundaries = boundary_path.clone();
            if is_boundary(&vocabulary, &declaration.kind) {
                push_unique(&mut reference_boundaries, part(declaration));
            }
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
                        fields: Vec::new(),
                        parameters: Vec::new(),
                        implicit: false,
                    });
            materialization.implicit = false;
            let visibility = declaration_visibility(document, declaration);
            if materialization
                .occurrences
                .first()
                .is_some_and(|first| first.visibility != visibility)
            {
                findings.push(Finding {
                    code: "conflicting-fragment-visibility".into(),
                    severity: Severity::Error,
                    message: format!(
                        "Fragments of '{}' must agree on Public or Private visibility",
                        declaration.name
                    ),
                    source: name_span.clone(),
                });
            }
            materialization.definitions.push(name_span.clone());
            materialization.occurrences.push(MaterializationOccurrence {
                source: name_span.clone(),
                boundaries: boundary_path.clone(),
                visibility,
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
                        source_boundaries: reference_boundaries.clone(),
                        target_kind: parent_part.kind,
                        target_name: parent_part.name,
                        source: span(document, declaration.start, declaration.name_end),
                    });
                }
            }

            for qualifier in &qualifiers {
                if !matches!(qualifier.part.kind.as_str(), "scope" | "layer") {
                    continue;
                }
                let boundary_id =
                    materialization_id(&[], &qualifier.part.kind, &qualifier.part.name);
                materializations
                    .entry(boundary_id.clone())
                    .or_insert_with(|| Materialization {
                        id: boundary_id,
                        kind: qualifier.part.kind.clone(),
                        name: qualifier.part.name.clone(),
                        scope: Vec::new(),
                        boundaries: Vec::new(),
                        definitions: Vec::new(),
                        occurrences: Vec::new(),
                        fields: Vec::new(),
                        parameters: Vec::new(),
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
                        "'{}' has no concept or boundary declaration in this workspace",
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
                    source_boundaries: reference_boundaries.clone(),
                    target_kind: qualifier.part.kind,
                    target_name: qualifier.part.name,
                    source: qualifier.source,
                });
            }
            for token in header_extends(document, declaration) {
                pending.push(PendingLink {
                    relation: "extends".into(),
                    from: id.clone(),
                    source_scope: scope.clone(),
                    source_boundaries: reference_boundaries.clone(),
                    target_kind: declaration.kind.clone(),
                    target_name: token.value.clone(),
                    source: span(document, token.start, token.end),
                });
            }
            if let Some(concept) = concepts.get_mut(&declaration.kind) {
                let context = {
                    let mut context = scope;
                    context.push(part(declaration));
                    context
                };
                for assignment in model_properties(document, declaration) {
                    let key_source = span(document, assignment.key.start, assignment.key.end);
                    let repeated_in_new_fragment = record_field(
                        materializations.get_mut(&id).unwrap(),
                        &assignment.key.value,
                        key_source.clone(),
                        &name_span,
                    );
                    if let Some(property) = concept
                        .properties
                        .iter_mut()
                        .find(|property| property.name == assignment.key.value)
                    {
                        property.uses.push(key_source.clone());
                        if repeated_in_new_fragment {
                            findings.push(Finding {
                                code: "duplicate-fragment-property".into(),
                                severity: Severity::Error,
                                message: format!(
                                    "Property '{}' is assigned in multiple fragments of '{}'",
                                    property.name, declaration.name
                                ),
                                source: key_source.clone(),
                            });
                        }
                        direct_properties
                            .entry(id.clone())
                            .or_default()
                            .insert(property.name.clone());
                        if let Some(value_kind) = &property.value_kind {
                            if let Some(references) =
                                value_references(&assignment.value, property.many)
                            {
                                for value in references {
                                    pending.push(PendingLink {
                                        relation: format!("property:{}", property.name),
                                        from: id.clone(),
                                        source_scope: context.clone(),
                                        source_boundaries: reference_boundaries.clone(),
                                        target_kind: value_kind.clone(),
                                        target_name: value.value.clone(),
                                        source: span(document, value.start, value.end),
                                    });
                                }
                                if let PropertyValue::Binding(target, arguments) = assignment.value
                                {
                                    pending_bindings.push(PendingBinding {
                                        from: id.clone(),
                                        property: property.name.clone(),
                                        target_source: span(document, target.start, target.end),
                                        source_scope: context.clone(),
                                        source_boundaries: reference_boundaries.clone(),
                                        arguments,
                                    });
                                }
                            } else {
                                findings.push(Finding {
                                    code: "wrong-property-shape".into(),
                                    severity: Severity::Error,
                                    message: format!(
                                        "Property '{}' requires {} reference{}",
                                        property.name,
                                        if property.many { "a list of" } else { "one" },
                                        if property.many { "s" } else { "" }
                                    ),
                                    source: span(
                                        document,
                                        assignment.key.start,
                                        assignment.key.end,
                                    ),
                                });
                            }
                        }
                    } else if let PropertyValue::Record(fields) = &assignment.value {
                        if let Some((kind, many, source)) = parameter_type(fields, document) {
                            vocabulary_uses
                                .entry(kind.clone())
                                .or_default()
                                .push(VocabularyUse {
                                    source: source.clone(),
                                    kind: VocabularyUseKind::TypeReference,
                                });
                            let parameters = parameter_specs.entry(id.clone()).or_default();
                            if let Some(previous) = parameters.get_mut(&assignment.key.value) {
                                previous.definitions.push(key_source.clone());
                                if previous.kind != kind || previous.many != many {
                                    findings.push(Finding {
                                        code: "conflicting-parameter-rule".into(),
                                        severity: Severity::Error,
                                        message: format!(
                                            "Parameter '{}' has conflicting types",
                                            assignment.key.value
                                        ),
                                        source: span(
                                            document,
                                            assignment.key.start,
                                            assignment.key.end,
                                        ),
                                    });
                                }
                            } else {
                                parameters.insert(
                                    assignment.key.value.clone(),
                                    ParameterSpec {
                                        kind,
                                        many,
                                        source,
                                        definitions: vec![key_source],
                                    },
                                );
                            }
                        } else if fields.iter().any(|field| field.key.value == "type") {
                            findings.push(Finding {
                                code: "invalid-parameter-type".into(),
                                severity: Severity::Error,
                                message: format!(
                                    "Input '{}' needs one type, or a one-element type list",
                                    assignment.key.value
                                ),
                                source: span(document, assignment.key.start, assignment.key.end),
                            });
                        }
                    }
                }
            } else {
                for assignment in model_properties(document, declaration) {
                    record_field(
                        materializations.get_mut(&id).unwrap(),
                        &assignment.key.value,
                        span(document, assignment.key.start, assignment.key.end),
                        &name_span,
                    );
                }
            }
        }
    }

    let mut links = Vec::new();
    for reference in pending {
        match resolve(&materializations, &reference) {
            Ok(target) => {
                let from_scope = &materializations[&reference.from].scope;
                let crosses_boundary = reference.relation != "boundary"
                    && target_crosses_boundary(target, &reference.source_boundaries);
                let crosses_scope = reference.relation != "boundary"
                    && from_scope != &target.scope
                    && !target.scope.starts_with(&reference.source_scope);
                links.push(Link {
                    relation: reference.relation,
                    from: reference.from,
                    to: target.id.clone(),
                    source: reference.source,
                    crosses_scope,
                    crosses_boundary,
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
    for parameters in parameter_specs.values() {
        for (name, parameter) in parameters {
            if !concepts.contains_key(&parameter.kind) {
                findings.push(Finding {
                    code: "unknown-parameter-type".into(),
                    severity: Severity::Error,
                    message: format!(
                        "Parameter '{}' refers to undeclared kind '{}'",
                        name, parameter.kind
                    ),
                    source: parameter.source.clone(),
                });
            }
        }
    }
    let mut bindings = Vec::new();
    for binding in &pending_bindings {
        let relation = format!("property:{}", binding.property);
        let Some(target_link) = links
            .iter()
            .find(|link| {
                link.from == binding.from
                    && link.relation == relation
                    && link.source == binding.target_source
            })
            .cloned()
        else {
            continue;
        };
        let parameters = parameter_specs.get(&target_link.to);
        let mut seen = HashSet::new();
        let mut arguments = Vec::new();
        for argument in &binding.arguments {
            let source = span_from_token(&binding.target_source.uri, &argument.key);
            if !seen.insert(&argument.key.value) {
                findings.push(Finding {
                    code: "duplicate-binding-argument".into(),
                    severity: Severity::Error,
                    message: format!("Binding argument '{}' is repeated", argument.key.value),
                    source,
                });
                continue;
            }
            let Some(parameter) = parameters.and_then(|items| items.get(&argument.key.value))
            else {
                findings.push(Finding {
                    code: "unknown-binding-argument".into(),
                    severity: Severity::Error,
                    message: format!(
                        "'{}' is not an input of '{}'",
                        argument.key.value, materializations[&target_link.to].name
                    ),
                    source,
                });
                continue;
            };
            let references = match &argument.value {
                PropertyValue::Binding(_, _) => None,
                value => value_references(value, parameter.many),
            };
            let Some(references) = references else {
                findings.push(Finding {
                    code: "wrong-binding-shape".into(),
                    severity: Severity::Error,
                    message: format!(
                        "Input '{}' requires {} reference{}",
                        argument.key.value,
                        if parameter.many { "a list of" } else { "one" },
                        if parameter.many { "s" } else { "" }
                    ),
                    source,
                });
                continue;
            };
            let mut values = Vec::new();
            for token in references {
                let reference = PendingLink {
                    relation: format!("binding:{}.{}", binding.property, argument.key.value),
                    from: binding.from.clone(),
                    source_scope: binding.source_scope.clone(),
                    source_boundaries: binding.source_boundaries.clone(),
                    target_kind: parameter.kind.clone(),
                    target_name: token.value.clone(),
                    source: span_from_token(&binding.target_source.uri, token),
                };
                let target = match resolve(&materializations, &reference) {
                    Ok(materialization) => {
                        let from_scope = &materializations[&binding.from].scope;
                        let crosses_scope = from_scope != &materialization.scope
                            && !materialization.scope.starts_with(&binding.source_scope);
                        let crosses_boundary =
                            target_crosses_boundary(materialization, &binding.source_boundaries);
                        links.push(Link {
                            relation: reference.relation,
                            from: reference.from,
                            to: materialization.id.clone(),
                            source: reference.source.clone(),
                            crosses_scope,
                            crosses_boundary,
                        });
                        Some(materialization.id.clone())
                    }
                    Err(reason) => {
                        findings.push(Finding {
                            code: reason.code().into(),
                            severity: Severity::Error,
                            message: reason.message(&reference),
                            source: reference.source.clone(),
                        });
                        None
                    }
                };
                values.push(BindingValue {
                    name: token.value.clone(),
                    target,
                    source: reference.source,
                });
            }
            arguments.push(BindingArgument {
                name: argument.key.value.clone(),
                source,
                values,
            });
        }
        if let Some(parameters) = parameters {
            for name in parameters.keys().filter(|name| !seen.contains(*name)) {
                findings.push(Finding {
                    code: "missing-binding-argument".into(),
                    severity: Severity::Error,
                    message: format!(
                        "Binding to '{}' must provide input '{}'",
                        materializations[&target_link.to].name, name
                    ),
                    source: binding.target_source.clone(),
                });
            }
        }
        bindings.push(Binding {
            from: binding.from.clone(),
            property: binding.property.clone(),
            target: target_link.to.clone(),
            source: binding.target_source.clone(),
            arguments,
        });
    }
    for link in links
        .iter()
        .filter(|link| link.relation.starts_with("property:"))
    {
        let Some(parameters) = parameter_specs.get(&link.to) else {
            continue;
        };
        if pending_bindings
            .iter()
            .any(|binding| binding.from == link.from && binding.target_source == link.source)
        {
            continue;
        }
        for name in parameters.keys() {
            findings.push(Finding {
                code: "missing-binding-argument".into(),
                severity: Severity::Error,
                message: format!(
                    "Binding to '{}' must provide input '{}'",
                    materializations[&link.to].name, name
                ),
                source: link.source.clone(),
            });
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
    bindings.sort_by(|left, right| {
        (&left.source.uri, left.source.start).cmp(&(&right.source.uri, right.source.start))
    });
    findings.sort_by(|left, right| {
        (&left.source.uri, left.source.start, &left.code).cmp(&(
            &right.source.uri,
            right.source.start,
            &right.code,
        ))
    });
    for (word, mut uses) in vocabulary_uses {
        if let Some(concept) = concepts.get_mut(&word) {
            uses.sort_by(|left, right| {
                (&left.source.uri, left.source.start).cmp(&(&right.source.uri, right.source.start))
            });
            uses.dedup_by(|left, right| left.source == right.source);
            concept.uses = uses;
        }
    }
    for concept in concepts.values_mut() {
        for property in &mut concept.properties {
            property
                .definitions
                .sort_by(|left, right| (&left.uri, left.start).cmp(&(&right.uri, right.start)));
            property.definitions.dedup();
            property
                .uses
                .sort_by(|left, right| (&left.uri, left.start).cmp(&(&right.uri, right.start)));
            property.uses.dedup();
        }
    }
    for (id, parameters) in parameter_specs {
        if let Some(materialization) = materializations.get_mut(&id) {
            let mut parameters: Vec<_> = parameters
                .into_iter()
                .map(|(name, parameter)| MaterializationParameter {
                    name,
                    kind: parameter.kind,
                    many: parameter.many,
                    source: parameter.source,
                    definitions: parameter.definitions,
                })
                .collect();
            parameters.sort_by(|left, right| left.name.cmp(&right.name));
            materialization.parameters = parameters;
        }
    }
    for materialization in materializations.values_mut() {
        materialization
            .fields
            .sort_by(|left, right| left.name.cmp(&right.name));
        for field in &mut materialization.fields {
            field.occurrences.sort_by(|left, right| {
                (&left.source.uri, left.source.start).cmp(&(&right.source.uri, right.source.start))
            });
        }
        for parameter in &mut materialization.parameters {
            parameter
                .definitions
                .sort_by(|left, right| (&left.uri, left.start).cmp(&(&right.uri, right.start)));
        }
    }
    SemanticMap {
        concepts: concepts.into_values().collect(),
        materializations: materializations.into_values().collect(),
        links,
        bindings,
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

fn span_from_token(uri: &str, token: &Token) -> SourceSpan {
    SourceSpan {
        uri: uri.to_string(),
        start: token.start,
        end: token.end,
    }
}

fn record_field(
    materialization: &mut Materialization,
    name: &str,
    source: SourceSpan,
    fragment: &SourceSpan,
) -> bool {
    let field = match materialization
        .fields
        .iter_mut()
        .find(|field| field.name == name)
    {
        Some(field) => field,
        None => {
            materialization.fields.push(MaterializationField {
                name: name.to_string(),
                occurrences: Vec::new(),
            });
            materialization.fields.last_mut().unwrap()
        }
    };
    let repeated_in_new_fragment = !field.occurrences.is_empty()
        && !field
            .occurrences
            .iter()
            .any(|occurrence| occurrence.fragment == *fragment);
    field.occurrences.push(MaterializationFieldOccurrence {
        source,
        fragment: fragment.clone(),
    });
    repeated_in_new_fragment
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

fn declaration_keyword<'a>(document: &'a Document, declaration: &Declaration) -> Option<&'a Token> {
    let tokens = document.tokens();
    let index = tokens
        .iter()
        .position(|token| token.start == declaration.start)?;
    let keyword = &tokens[index];
    (keyword.value == declaration.kind && tokens.get(index + 1)?.value != ":").then_some(keyword)
}

fn declaration_visibility(document: &Document, declaration: &Declaration) -> Visibility {
    match document
        .access_modifier(declaration)
        .map(|token| token.value.as_str())
    {
        Some("Public") => Visibility::Public,
        _ => Visibility::Private,
    }
}

fn boundary_qualifier_tokens<'a>(
    document: &'a Document,
    declaration: &Declaration,
    vocabulary: &Vocabulary,
) -> Vec<&'a Token> {
    let tokens = header_tokens(document, declaration);
    tokens
        .iter()
        .enumerate()
        .filter_map(|(index, token)| {
            if token.start >= declaration.start || !is_boundary(vocabulary, &token.value) {
                return None;
            }
            let next = tokens.get(index + 1)?;
            (next.kind == TokenKind::Identifier
                && next.value.chars().next().is_some_and(char::is_uppercase)
                || next.value == "[")
                .then_some(*token)
        })
        .collect()
}

fn header_qualifiers(
    document: &Document,
    declaration: &Declaration,
    vocabulary: &Vocabulary,
) -> Vec<HeaderQualifier> {
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
                result.push(HeaderQualifier {
                    part: ScopePart {
                        kind: token.value.clone(),
                        name: next.value.clone(),
                    },
                    source: span(document, next.start, next.end),
                });
            } else if next.value == "[" {
                for item in tokens
                    .iter()
                    .skip(index + 2)
                    .take_while(|item| item.value != "]")
                {
                    if item.kind == TokenKind::Identifier {
                        result.push(HeaderQualifier {
                            part: ScopePart {
                                kind: token.value.clone(),
                                name: item.value.clone(),
                            },
                            source: span(document, item.start, item.end),
                        });
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
        let mut value_many = false;
        let mut value_kind_reference = None;
        while cursor < tokens.len() && field_depth > 0 {
            match tokens[cursor].value.as_str() {
                "{" => field_depth += 1,
                "}" => field_depth -= 1,
                "type" if field_depth == 1 => {
                    let next = if tokens.get(cursor + 1).is_some_and(|next| next.value == ":") {
                        cursor + 2
                    } else {
                        cursor + 1
                    };
                    let (target, many) = if tokens.get(next).is_some_and(|item| item.value == "[") {
                        (tokens.get(next + 1), true)
                    } else {
                        (tokens.get(next), false)
                    };
                    if let Some(target) = target.filter(|item| item.kind == TokenKind::Identifier) {
                        value_kind = Some(lower_initial(&target.value));
                        value_many = many;
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
            many: value_many,
            definition: span(document, token.start, token.end),
            definitions: vec![span(document, token.start, token.end)],
            uses: Vec::new(),
            value_kind_reference,
        });
    }
    properties
}

fn model_properties(document: &Document, declaration: &Declaration) -> Vec<ModelAssignment> {
    let tokens = document.tokens();
    let Some(open) = tokens
        .iter()
        .position(|token| token.start == declaration.header_end && token.value == "{")
    else {
        return Vec::new();
    };
    let end = tokens
        .iter()
        .position(|token| token.end == declaration.end && token.value == "}")
        .unwrap_or(tokens.len());
    let mut cursor = open + 1;
    parse_assignments(tokens, &mut cursor, end)
}

fn value_references(value: &PropertyValue, many: bool) -> Option<Vec<&Token>> {
    match (value, many) {
        (PropertyValue::Reference(token), false) | (PropertyValue::Binding(token, _), false) => {
            Some(vec![token])
        }
        (PropertyValue::List(tokens, true), true) if !tokens.is_empty() => {
            Some(tokens.iter().collect())
        }
        _ => None,
    }
}

fn parameter_type(
    fields: &[ModelAssignment],
    document: &Document,
) -> Option<(String, bool, SourceSpan)> {
    let value = &fields.iter().find(|field| field.key.value == "type")?.value;
    let (token, many) = match value {
        PropertyValue::Reference(token) => (token, false),
        PropertyValue::List(tokens, true) if tokens.len() == 1 => (&tokens[0], true),
        _ => return None,
    };
    Some((
        lower_initial(&token.value),
        many,
        span(document, token.start, token.end),
    ))
}

fn parse_assignments(tokens: &[Token], cursor: &mut usize, end: usize) -> Vec<ModelAssignment> {
    let mut assignments = Vec::new();
    while *cursor < end {
        if tokens[*cursor].value == "}" {
            *cursor += 1;
            break;
        }
        if tokens[*cursor].kind == TokenKind::Identifier
            && tokens
                .get(*cursor + 1)
                .is_some_and(|next| next.value == ":")
        {
            let key = tokens[*cursor].clone();
            *cursor += 2;
            if let Some(value) = parse_property_value(tokens, cursor, end) {
                assignments.push(ModelAssignment { key, value });
            }
            continue;
        }
        match tokens[*cursor].value.as_str() {
            "{" => skip_group(tokens, cursor, end, "{", "}"),
            "[" => skip_group(tokens, cursor, end, "[", "]"),
            _ => *cursor += 1,
        }
    }
    assignments
}

fn parse_property_value(tokens: &[Token], cursor: &mut usize, end: usize) -> Option<PropertyValue> {
    let token = tokens.get(*cursor)?.clone();
    if *cursor >= end {
        return None;
    }
    if token.kind == TokenKind::Identifier {
        *cursor += 1;
        if tokens.get(*cursor).is_some_and(|next| next.value == "{") {
            *cursor += 1;
            return Some(PropertyValue::Binding(
                token,
                parse_assignments(tokens, cursor, end),
            ));
        }
        return Some(PropertyValue::Reference(token));
    }
    match token.value.as_str() {
        "[" => {
            *cursor += 1;
            let mut items = Vec::new();
            let mut expect_item = true;
            let mut valid = true;
            while *cursor < end && tokens[*cursor].value != "]" {
                let current = &tokens[*cursor];
                if expect_item && current.kind == TokenKind::Identifier {
                    items.push(current.clone());
                    expect_item = false;
                } else if !expect_item && current.value == "," {
                    expect_item = true;
                } else {
                    valid = false;
                }
                *cursor += 1;
            }
            if *cursor < end && !expect_item {
                *cursor += 1;
            } else {
                valid = false;
            }
            Some(PropertyValue::List(items, valid))
        }
        "{" => {
            *cursor += 1;
            Some(PropertyValue::Record(parse_assignments(
                tokens, cursor, end,
            )))
        }
        _ => {
            *cursor += 1;
            None
        }
    }
}

fn skip_group(tokens: &[Token], cursor: &mut usize, end: usize, open: &str, close: &str) {
    let mut depth = 0;
    while *cursor < end {
        if tokens[*cursor].value == open {
            depth += 1;
        } else if tokens[*cursor].value == close {
            depth -= 1;
            if depth == 0 {
                *cursor += 1;
                break;
            }
        }
        *cursor += 1;
    }
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
    Inaccessible,
}

impl ResolveError {
    fn code(&self) -> &'static str {
        match self {
            Self::Missing => "unresolved-reference",
            Self::WrongKind => "wrong-reference-kind",
            Self::Ambiguous => "ambiguous-reference",
            Self::Inaccessible => "inaccessible-reference",
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
            Self::Inaccessible => {
                format!("'{}' is private to another boundary", reference.target_name)
            }
        }
    }
}

fn target_crosses_boundary(target: &Materialization, source: &[ScopePart]) -> bool {
    !target.implicit
        && target.occurrences.iter().all(|occurrence| {
            occurrence
                .boundaries
                .iter()
                .any(|boundary| !source.contains(boundary))
        })
}

fn target_accessible(target: &Materialization, source: &[ScopePart]) -> bool {
    !target_crosses_boundary(target, source)
        || (!target.occurrences.is_empty()
            && target
                .occurrences
                .iter()
                .all(|occurrence| occurrence.visibility == Visibility::Public))
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
    let accessible: Vec<&Materialization> = candidates
        .iter()
        .copied()
        .filter(|materialization| {
            reference.relation == "boundary"
                || target_accessible(materialization, &reference.source_boundaries)
        })
        .collect();
    if accessible.is_empty() {
        return Err(ResolveError::Inaccessible);
    }
    let visible: Vec<&Materialization> = accessible
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
    if accessible.len() == 1 {
        Ok(accessible[0])
    } else {
        Err(ResolveError::Ambiguous)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vocabulary_uses_keep_definition_type_and_model_roles_separate() {
        let spec = "boundary Interface {}\nconcept Interface {}\nconcept Endpoint { target { type: Interface } }";
        let model = "interface RestApi {}\ninterface Public endpoint Entry { target: RestApi }\n// interface Comment\nendpoint Other { note: \"Interface\" }";
        let map = analyze(&HashMap::from([
            ("file:///model.spec".into(), spec.into()),
            ("file:///model.saml".into(), model.into()),
        ]));
        let interface = map
            .concepts
            .iter()
            .find(|concept| concept.word == "interface")
            .unwrap();
        assert_eq!(interface.definitions.len(), 2);
        assert_eq!(interface.uses.len(), 3);
        assert!(interface
            .uses
            .iter()
            .any(|usage| matches!(usage.kind, VocabularyUseKind::TypeReference)));
        assert!(interface
            .uses
            .iter()
            .any(|usage| matches!(usage.kind, VocabularyUseKind::Materialization)));
        assert!(interface
            .uses
            .iter()
            .any(|usage| matches!(usage.kind, VocabularyUseKind::Qualifier)));
        let entry = map
            .materializations
            .iter()
            .find(|item| item.id == "/interface:Public/endpoint:Entry")
            .unwrap();
        assert_eq!(entry.occurrences[0].visibility, Visibility::Private);
    }

    #[test]
    fn access_modifiers_are_keyword_tokens_only_in_declaration_position() {
        let spec = "boundary Scope {}\nboundary Interface {}\nconcept Data {}\nconcept Endpoint {}";
        let model = "scope Alpha Public data Shared {}\ninterface Public endpoint Entry {}";
        let sources = HashMap::from([("file:///model.spec".into(), spec.into())]);
        let vocabulary = Vocabulary::from_sources(&sources);
        let document =
            Document::with_vocabulary("file:///model.saml".into(), model.into(), &vocabulary);
        let mut line = 0;
        let mut column = 0;
        let mut keyword_positions = Vec::new();
        for token in document.semantic_tokens().as_chunks::<5>().0 {
            line += token[0];
            column = if token[0] == 0 {
                column + token[1]
            } else {
                token[1]
            };
            if token[3] == 0 {
                keyword_positions.push((line, column));
            }
        }
        assert!(keyword_positions.contains(&(0, 12)));
        assert!(!keyword_positions.contains(&(1, 10)));
    }

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
    fn misspelled_boundary_keyword_reports_rename() {
        let sources = HashMap::from([("file:///model.spec".into(), "boundry Entity {}".into())]);
        let map = analyze(&sources);
        assert!(map.concepts.is_empty());
        assert!(map.findings.iter().any(|finding| {
            finding.code == "obsolete-keyword"
                && finding.message == "Use 'boundary' instead of 'boundry' in spec declarations"
        }));
    }

    #[test]
    fn concept_materializations_compose_with_boundaries_and_cross_scope_links() {
        let sources = HashMap::from([
            (
                "file:///model.spec".into(),
                "boundary Scope {}\nboundary Entity {}\nconcept Entity {}\nconcept Action { initiator { type: Entity } }".into(),
            ),
            (
                "file:///model.saml".into(),
                "scope Identity Public entity User {}\nscope Article action Publish { initiator: User }".into(),
            ),
        ]);
        let map = analyze(&sources);
        let entity = map
            .concepts
            .iter()
            .find(|item| item.word == "entity")
            .unwrap();
        assert_eq!(entity.roles, ["concept", "boundary"]);
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
                && link.crosses_boundary
        }));
        assert!(map.findings.is_empty(), "{:?}", map.findings);
    }

    #[test]
    fn private_materialization_resolves_locally_but_not_across_boundaries() {
        let map = analyze(&HashMap::from([
            ("file:///model.spec".into(),
                "boundary Scope {}\nconcept Data {}\nconcept Action { target { type: Data } }".into()),
            ("file:///model.saml".into(),
                "scope Alpha { data Secret {} action Local { target: Secret } }\nscope Beta action Outside { target: Secret }".into()),
        ]));
        assert!(map
            .links
            .iter()
            .any(|link| link.relation == "property:target"
                && link.from == "/scope:Alpha/action:Local"
                && !link.crosses_boundary));
        assert!(!map
            .links
            .iter()
            .any(|link| link.relation == "property:target"
                && link.from == "/scope:Beta/action:Outside"));
        assert!(map
            .findings
            .iter()
            .any(|finding| finding.code == "inaccessible-reference"
                && finding.message.contains("Secret")));
    }

    #[test]
    fn public_materialization_is_visible_across_boundaries() {
        let map = analyze(&HashMap::from([
            (
                "file:///model.spec".into(),
                "boundary Scope {}\nconcept Data {}\nconcept Action { target { type: Data } }"
                    .into(),
            ),
            (
                "file:///model.saml".into(),
                "scope Alpha Public data Shared {}\nscope Beta action Outside { target: Shared }"
                    .into(),
            ),
        ]));
        let link = map
            .links
            .iter()
            .find(|link| link.relation == "property:target")
            .unwrap();
        assert_eq!(link.to, "/scope:Alpha/data:Shared");
        assert!(link.crosses_boundary);
        assert_eq!(
            map.materializations
                .iter()
                .find(|item| item.id == link.to)
                .unwrap()
                .occurrences[0]
                .visibility,
            Visibility::Public
        );
        assert!(map.findings.is_empty(), "{:?}", map.findings);
    }

    #[test]
    fn a_layer_still_restricts_access_within_the_same_scope() {
        let map = analyze(&HashMap::from([
            ("file:///model.spec".into(),
                "boundary Scope {}\nboundary Layer {}\nconcept Data {}\nconcept Action { target { type: Data } }".into()),
            ("file:///model.saml".into(),
                "layer Infrastructure scope Alpha data Secret {}\nlayer Application scope Alpha action Outside { target: Secret }".into()),
        ]));
        assert!(map
            .findings
            .iter()
            .any(|finding| finding.code == "inaccessible-reference"));
    }

    #[test]
    fn an_inaccessible_candidate_does_not_hide_a_visible_workspace_candidate() {
        let map = analyze(&HashMap::from([
            ("file:///model.spec".into(),
                "boundary Scope {}\nboundary Layer {}\nconcept Data {}\nconcept Action { target { type: Data } }".into()),
            ("file:///model.saml".into(),
                "data Shared {}\nlayer Infrastructure scope Alpha data Shared {}\nlayer Application scope Alpha action Use { target: Shared }".into()),
        ]));
        let link = map
            .links
            .iter()
            .find(|link| link.relation == "property:target")
            .unwrap();
        assert_eq!(link.to, "/data:Shared");
        assert!(!link.crosses_boundary);
        assert!(map.findings.is_empty(), "{:?}", map.findings);
    }

    #[test]
    fn explicit_private_stays_local_and_public_crosses_nested_boundaries() {
        let map = analyze(&HashMap::from([
            ("file:///model.spec".into(),
                "boundary Scope {}\nboundary Layer {}\nconcept Data {}\nconcept Action { target { type: Data } }".into()),
            ("file:///model.saml".into(),
                "layer Infrastructure scope Alpha { Private data Hidden {} Public data Shared {} action Local { target: Hidden } }\nscope Beta action Outside { target: Shared }".into()),
        ]));
        let links: Vec<_> = map
            .links
            .iter()
            .filter(|link| link.relation == "property:target")
            .collect();
        assert_eq!(links.len(), 2);
        assert!(links
            .iter()
            .any(|link| link.to == "/scope:Alpha/data:Hidden" && !link.crosses_boundary));
        assert!(links
            .iter()
            .any(|link| link.to == "/scope:Alpha/data:Shared" && link.crosses_boundary));
        let hidden = map
            .materializations
            .iter()
            .find(|item| item.name == "Hidden")
            .unwrap();
        assert_eq!(hidden.occurrences[0].visibility, Visibility::Private);
        assert!(map.findings.is_empty(), "{:?}", map.findings);
    }

    #[test]
    fn extends_resolves_nearest_scope_and_reports_ambiguous_crossings() {
        let sources = HashMap::from([
            ("file:///model.spec".into(), "boundary Scope {}\nconcept Stage {}".into()),
            (
                "file:///model.saml".into(),
                "scope Alpha Public stage Base {}\nscope Beta Public stage Base {}\nscope Alpha stage Child extends Base {}\nscope Gamma stage Other extends Base {}".into(),
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
    fn binding_arguments_resolve_against_materialization_input_schema() {
        let map = analyze(&HashMap::from([
            ("file:///model.spec".into(),
                "concept Data {}\nconcept ApplicationCapability {}\nconcept Application { capability { type: ApplicationCapability } }".into()),
            ("file:///model.saml".into(),
                "data Article {}\ndata Author {}\napplicationCapability RestApi { expose: { type: [Data] } }\napplication Portal { capability: RestApi { expose: [Article, Author] } }".into()),
        ]));
        assert!(map.findings.is_empty(), "{:?}", map.findings);
        let binding = &map.bindings[0];
        assert_eq!(binding.from, "/application:Portal");
        assert_eq!(binding.target, "/applicationCapability:RestApi");
        assert_eq!(binding.arguments[0].name, "expose");
        assert_eq!(binding.arguments[0].values.len(), 2);
        assert_eq!(
            binding.arguments[0].values[0].target.as_deref(),
            Some("/data:Article")
        );
        assert_eq!(
            binding.arguments[0].values[1].target.as_deref(),
            Some("/data:Author")
        );
        assert_eq!(
            map.links
                .iter()
                .filter(|link| link.relation == "binding:capability.expose")
                .count(),
            2
        );
        let capability = map
            .materializations
            .iter()
            .find(|item| item.id == "/applicationCapability:RestApi")
            .unwrap();
        assert_eq!(capability.parameters[0].kind, "data");
        assert!(capability.parameters[0].many);
    }

    #[test]
    fn concept_property_list_resolves_each_entry() {
        let map = analyze(&HashMap::from([
            ("file:///model.spec".into(),
                "concept Data {}\nconcept Collection { members { type: [Data] } }".into()),
            ("file:///model.saml".into(),
                "data Article {}\ndata Author {}\ncollection Library { members: [Article, Author] }".into()),
        ]));
        assert!(map.findings.is_empty(), "{:?}", map.findings);
        assert_eq!(
            map.links
                .iter()
                .filter(|link| link.relation == "property:members")
                .count(),
            2
        );
        assert!(
            map.concepts
                .iter()
                .find(|item| item.word == "collection")
                .unwrap()
                .properties[0]
                .many
        );
    }

    #[test]
    fn malformed_reference_list_is_rejected() {
        let map = analyze(&HashMap::from([
            (
                "file:///model.spec".into(),
                "concept Data {}\nconcept Collection { members { type: [Data] } }".into(),
            ),
            (
                "file:///model.saml".into(),
                "data Article {}\ndata Author {}\ncollection Library { members: [Article Author] }"
                    .into(),
            ),
        ]));
        assert!(map
            .findings
            .iter()
            .any(|item| item.code == "wrong-property-shape"));
    }

    #[test]
    fn binding_input_shape_and_name_are_checked() {
        let map = analyze(&HashMap::from([
            ("file:///model.spec".into(),
                "concept Data {}\nconcept ApplicationCapability {}\nconcept Application { capability { type: ApplicationCapability } }".into()),
            ("file:///model.saml".into(),
                "data Article {}\napplicationCapability RestApi { expose: { type: Data } }\napplication Portal { capability: RestApi { expose: [Article] extra: Article } }".into()),
        ]));
        assert!(map
            .findings
            .iter()
            .any(|item| item.code == "wrong-binding-shape"));
        assert!(map
            .findings
            .iter()
            .any(|item| item.code == "unknown-binding-argument"));
    }

    #[test]
    fn required_binding_input_is_checked_with_or_without_a_block() {
        let map = analyze(&HashMap::from([
            ("file:///model.spec".into(),
                "concept Data {}\nconcept ApplicationCapability {}\nconcept Application { capability { type: ApplicationCapability } }".into()),
            ("file:///model.saml".into(),
                "applicationCapability RestApi { expose: { type: [Data] } }\napplication Portal { capability: RestApi }\napplication Admin { capability: RestApi {} }".into()),
        ]));
        assert_eq!(
            map.findings
                .iter()
                .filter(|item| item.code == "missing-binding-argument")
                .count(),
            2
        );
    }

    #[test]
    fn binding_list_entries_report_unresolved_or_wrong_kinds() {
        let map = analyze(&HashMap::from([
            ("file:///model.spec".into(),
                "concept Data {}\nconcept ApplicationCapability {}\nconcept Application { capability { type: ApplicationCapability } }".into()),
            ("file:///model.saml".into(),
                "data Article {}\napplicationCapability RestApi { expose: { type: [Data] } }\napplication Portal { capability: RestApi { expose: [Article, Missing, RestApi] } }".into()),
        ]));
        assert!(map
            .findings
            .iter()
            .any(|item| item.code == "unresolved-reference"));
        assert!(map
            .findings
            .iter()
            .any(|item| item.code == "wrong-reference-kind"));
        assert_eq!(map.bindings[0].arguments[0].values.len(), 3);
    }

    #[test]
    fn repeated_materializations_keep_each_boundary_occurrence() {
        let sources = HashMap::from([
            (
                "file:///model.spec".into(),
                "boundary Layer {}\nboundary Scope {}".into(),
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
    fn fragments_merge_distinct_properties_and_keep_each_source() {
        let map = analyze(&HashMap::from([
            (
                "file:///model.spec".into(),
                "concept Data {}\nconcept Application { primary { type: Data } fallback { type: Data } }".into(),
            ),
            (
                "file:///a.saml".into(),
                "data One {}\napplication Portal { primary: One }".into(),
            ),
            (
                "file:///b.saml".into(),
                "application Portal { fallback: One }".into(),
            ),
        ]));
        let portal = map
            .materializations
            .iter()
            .find(|item| item.id == "/application:Portal")
            .unwrap();
        assert_eq!(portal.definitions.len(), 2);
        assert_eq!(portal.fields.len(), 2);
        for (name, uri) in [
            ("primary", "file:///a.saml"),
            ("fallback", "file:///b.saml"),
        ] {
            let field = portal
                .fields
                .iter()
                .find(|field| field.name == name)
                .unwrap();
            assert_eq!(field.occurrences.len(), 1);
            assert_eq!(field.occurrences[0].source.uri, uri);
            assert_eq!(field.occurrences[0].fragment.uri, uri);
            assert!(map.links.iter().any(|link| {
                link.from == portal.id && link.relation == format!("property:{name}")
            }));
        }
        assert!(map.findings.is_empty(), "{:?}", map.findings);
    }

    #[test]
    fn repeated_typed_property_in_another_fragment_is_an_error() {
        let map = analyze(&HashMap::from([
            (
                "file:///model.spec".into(),
                "concept Data {}\nconcept Application { target { type: Data } }".into(),
            ),
            (
                "file:///a.saml".into(),
                "data One {}\napplication Portal { target: One }".into(),
            ),
            (
                "file:///b.saml".into(),
                "application Portal { target: One }".into(),
            ),
        ]));
        let duplicate = map
            .findings
            .iter()
            .find(|finding| finding.code == "duplicate-fragment-property")
            .unwrap();
        assert_eq!(duplicate.source.uri, "file:///b.saml");
        let portal = map
            .materializations
            .iter()
            .find(|item| item.id == "/application:Portal")
            .unwrap();
        assert_eq!(portal.fields[0].occurrences.len(), 2);
    }

    #[test]
    fn input_schemas_compose_and_incompatible_fragments_report_a_conflict() {
        let spec = "concept Data {}\nconcept Capability {}";
        let first = "capability Rest { expose: { type: Data } }";
        let second = "capability Rest { timeout: { type: Data } expose: { type: Data } }";
        let sources = HashMap::from([
            ("file:///model.spec".into(), spec.into()),
            ("file:///a.saml".into(), first.into()),
            ("file:///b.saml".into(), second.into()),
        ]);
        let map = analyze(&sources);
        let rest = map
            .materializations
            .iter()
            .find(|item| item.id == "/capability:Rest")
            .unwrap();
        assert_eq!(rest.parameters.len(), 2);
        assert_eq!(
            rest.parameters
                .iter()
                .find(|parameter| parameter.name == "expose")
                .unwrap()
                .definitions
                .len(),
            2
        );
        assert!(map.findings.is_empty(), "{:?}", map.findings);

        let mut conflict = sources;
        conflict.insert(
            "file:///b.saml".into(),
            second.replace("expose: { type: Data }", "expose: { type: [Data] }"),
        );
        let map = analyze(&conflict);
        assert!(map.findings.iter().any(|finding| {
            finding.code == "conflicting-parameter-rule" && finding.source.uri == "file:///b.saml"
        }));
    }

    #[test]
    fn fragments_must_agree_on_visibility_before_export() {
        let spec = "boundary Scope {}\nconcept Data {}\nconcept Action { target { type: Data } }";
        let first = "scope Alpha Public data Shared {}";
        let second = "scope Alpha data Shared {}";
        let action = "action Use { target: Shared }";
        let sources = HashMap::from([
            ("file:///model.spec".into(), spec.into()),
            ("file:///a.saml".into(), first.into()),
            ("file:///b.saml".into(), second.into()),
            ("file:///c.saml".into(), action.into()),
        ]);
        let map = analyze(&sources);
        assert!(map.findings.iter().any(|finding| {
            finding.code == "conflicting-fragment-visibility"
                && finding.source.uri == "file:///b.saml"
        }));
        assert!(map.findings.iter().any(|finding| {
            finding.code == "inaccessible-reference" && finding.source.uri == "file:///c.saml"
        }));

        let mut exported = sources;
        exported.insert("file:///b.saml".into(), first.into());
        let map = analyze(&exported);
        assert!(!map.findings.iter().any(|finding| {
            matches!(
                finding.code.as_str(),
                "conflicting-fragment-visibility" | "inaccessible-reference"
            )
        }));
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
                "boundary Scope {}\nconcept Entity {}".into(),
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
