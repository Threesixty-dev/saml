use saml_lsp::diagnostics::diagnostic_report;
use saml_lsp::semantic::analyze;
use saml_lsp::workspace::collect_sources;
use serde_json::json;
use std::collections::HashMap;
use std::env;
use std::io;
use std::path::PathBuf;

fn main() -> io::Result<()> {
    let roots: Vec<PathBuf> = env::args_os().skip(1).map(PathBuf::from).collect();
    let roots = if roots.is_empty() {
        vec![env::current_dir()?]
    } else {
        roots
    };
    let mut reports = Vec::new();
    for root in roots {
        let root = root.canonicalize()?;
        if !root.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("workspace must be a directory: {}", root.display()),
            ));
        }
        let mut sources = HashMap::new();
        collect_sources(&root, &mut sources);
        reports.push((root, diagnostic_report(&sources, &analyze(&sources))));
    }
    if reports.len() == 1 && reports[0].1.files == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "workspace has no .saml or .spec files",
        ));
    }
    let has_errors = reports.iter().any(|(_, report)| report.errors > 0);
    if reports.len() == 1 {
        println!("{}", serde_json::to_string_pretty(&reports[0].1)?);
    } else {
        let workspaces: Vec<_> = reports
            .into_iter()
            .map(|(root, report)| json!({"root": root, "report": report}))
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({"workspaces": workspaces}))?
        );
    }
    if has_errors {
        std::process::exit(1);
    }
    Ok(())
}
