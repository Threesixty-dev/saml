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

pub fn path_to_uri(path: &Path) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

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
