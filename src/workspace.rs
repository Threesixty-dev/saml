use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub fn collect_sources(root: &Path, sources: &mut HashMap<String, String>) {
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
            if path.join(".git").exists() {
                continue;
            }
            collect_sources(&path, sources);
        } else if file_type.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext == "saml" || ext == "spec")
        {
            let uri = path_to_uri(&path);
            if let Ok(text) = fs::read_to_string(&path) {
                sources.insert(uri, text);
            }
        }
    }
}

pub fn collect_sources_checked(
    root: &Path,
    sources: &mut HashMap<String, String>,
) -> std::io::Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || matches!(name.as_ref(), "node_modules" | "target" | "venv") {
            continue;
        }
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            if !path.join(".git").exists() {
                collect_sources_checked(&path, sources)?;
            }
        } else if file_type.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext == "saml" || ext == "spec")
        {
            sources.insert(path_to_uri(&path), fs::read_to_string(&path)?);
        }
    }
    Ok(())
}

pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let url = reqwest::Url::parse(uri).ok()?;
    if url.scheme() != "file" || url.query().is_some() || url.fragment().is_some() {
        return None;
    }
    url.to_file_path().ok()
}

pub fn path_to_uri(path: &Path) -> String {
    reqwest::Url::from_file_path(path)
        .expect("workspace file path must be absolute")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn file_uri_round_trips_platform_paths() {
        let path = std::env::current_dir()
            .unwrap()
            .join("project/example-1/spec.saml");
        let uri = path_to_uri(&path);
        assert!(uri.ends_with("/spec.saml"));
        assert_eq!(uri_to_path(&uri), Some(path));
    }

    #[test]
    fn nested_repository_does_not_contribute_to_parent_workspace() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("saml-workspace-{}-{unique}", std::process::id()));
        let nested = root.join("nested");
        fs::create_dir_all(nested.join(".git")).unwrap();
        fs::write(root.join("model.spec"), "concept Entity {}").unwrap();
        fs::write(nested.join("model.spec"), "concept Foreign {}").unwrap();
        let mut sources = HashMap::new();
        collect_sources(&root, &mut sources);
        assert!(sources.contains_key(&path_to_uri(&root.join("model.spec"))));
        assert!(!sources.contains_key(&path_to_uri(&nested.join("model.spec"))));
        fs::remove_dir_all(root).unwrap();
    }
}
