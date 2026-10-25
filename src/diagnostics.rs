use crate::model::{Document, Vocabulary};
use crate::semantic::{SemanticMap, Severity};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Serialize)]
pub struct WorkspaceDiagnostic {
    pub uri: String,
    pub severity: Severity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub message: String,
    pub range: Value,
}

#[derive(Debug, Serialize)]
pub struct DiagnosticReport {
    pub files: usize,
    pub errors: usize,
    pub warnings: usize,
    pub diagnostics: Vec<WorkspaceDiagnostic>,
}

pub fn diagnostic_report(sources: &HashMap<String, String>, map: &SemanticMap) -> DiagnosticReport {
    let vocabulary = Vocabulary::from_sources(sources);
    let mut diagnostics = Vec::new();
    for (uri, text) in sources {
        let document = Document::with_vocabulary(uri.clone(), text.clone(), &vocabulary);
        for item in &document.diagnostics {
            diagnostics.push(WorkspaceDiagnostic {
                uri: uri.clone(),
                severity: Severity::Error,
                code: None,
                message: item["message"].as_str().unwrap_or("Syntax error").into(),
                range: item["range"].clone(),
            });
        }
        for finding in map.findings.iter().filter(|item| item.source.uri == *uri) {
            diagnostics.push(WorkspaceDiagnostic {
                uri: uri.clone(),
                severity: finding.severity,
                code: Some(finding.code.clone()),
                message: finding.message.clone(),
                range: document.range(finding.source.start, finding.source.end),
            });
        }
    }
    diagnostics.sort_by(|left, right| {
        let position = |item: &WorkspaceDiagnostic| {
            (
                item.range["start"]["line"].as_u64().unwrap_or(0),
                item.range["start"]["character"].as_u64().unwrap_or(0),
            )
        };
        left.uri
            .cmp(&right.uri)
            .then_with(|| position(left).cmp(&position(right)))
            .then_with(|| left.message.cmp(&right.message))
    });
    let errors = diagnostics
        .iter()
        .filter(|item| item.severity == Severity::Error)
        .count();
    DiagnosticReport {
        files: sources.len(),
        errors,
        warnings: diagnostics.len() - errors,
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic::analyze;

    #[test]
    fn report_includes_syntax_and_semantic_findings() {
        let sources = HashMap::from([
            (
                "file:///model.spec".into(),
                "concept Entity {}\nconcept Action { target { type: Entity } }".into(),
            ),
            ("file:///model.saml".into(), "action Start {".into()),
        ]);
        let map = analyze(&sources);
        let report = diagnostic_report(&sources, &map);
        assert_eq!(report.files, 2);
        assert!(report.errors >= 2);
        assert!(report
            .diagnostics
            .iter()
            .any(|item| item.message.contains("Unclosed")));
        assert!(report
            .diagnostics
            .iter()
            .any(|item| item.code.as_deref() == Some("missing-required-property")));
    }
}
