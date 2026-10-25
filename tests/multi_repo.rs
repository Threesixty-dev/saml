use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("saml-multi-repo-{}-{unique}", std::process::id()));
        fs::create_dir_all(root.join("a")).unwrap();
        fs::create_dir_all(root.join("b")).unwrap();
        fs::write(root.join("a/model.spec"), "concept Entity {}\n").unwrap();
        fs::write(root.join("a/model.saml"), "entity Alpha {}\n").unwrap();
        fs::write(root.join("b/model.saml"), "entity Beta {}\n").unwrap();
        Self(root)
    }

    fn repo(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn batch_commands_isolate_vocabulary_by_repository() {
    let fixture = Fixture::new();
    let roots = [fixture.repo("a"), fixture.repo("b")];
    let check = Command::new(env!("CARGO_BIN_EXE_saml-check"))
        .args(&roots)
        .output()
        .unwrap();
    assert!(check.status.success());
    let check: Value = serde_json::from_slice(&check.stdout).unwrap();
    assert_eq!(check["workspaces"][0]["report"]["warnings"], 0);
    assert_eq!(check["workspaces"][1]["report"]["warnings"], 1);
    assert_eq!(
        check["workspaces"][1]["report"]["diagnostics"][0]["code"],
        "unknown-vocabulary"
    );

    let map = Command::new(env!("CARGO_BIN_EXE_saml-map"))
        .args(&roots)
        .output()
        .unwrap();
    assert!(map.status.success());
    let map: Value = serde_json::from_slice(&map.stdout).unwrap();
    assert_eq!(map["workspaces"][0]["map"]["concepts"][0]["word"], "entity");
    assert!(map["workspaces"][1]["map"]["concepts"]
        .as_array()
        .unwrap()
        .is_empty());
}
