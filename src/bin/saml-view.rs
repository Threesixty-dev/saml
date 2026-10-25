use saml_lsp::portal::{serve, PortalSource};
use std::env;
use std::io;
use std::net::TcpListener;
use std::path::PathBuf;

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
            value
                .parse::<u16>()
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
        })
        .transpose()?
        .unwrap_or(8787);
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    println!("SAML semantic atlas: http://127.0.0.1:{port}");
    for (index, root) in roots.iter().enumerate() {
        println!("Workspace {index}: {}", root.display());
    }
    serve(listener, PortalSource::Static(roots))
}
