use saml_lsp::semantic::analyze;
use saml_lsp::workspace::collect_sources;
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
    let mut sources = HashMap::new();
    for root in roots {
        collect_sources(&root.canonicalize()?, &mut sources);
    }
    let map = analyze(&sources);
    println!("{}", serde_json::to_string_pretty(&map)?);
    Ok(())
}
