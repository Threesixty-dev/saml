use crate::diagnostics::diagnostic_report;
use crate::distributed::RegisteredRepository;
use crate::semantic::analyze;
use crate::workspace::{collect_sources_checked, path_to_uri, uri_to_path};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;

#[derive(Clone, Copy)]
pub struct Limits {
    pub entries: usize,
    pub diagnostics: usize,
    pub commits: usize,
}

pub const LOCAL_LIMITS: Limits = Limits {
    entries: 100,
    diagnostics: 100,
    commits: 30,
};
pub const HUB_LIMITS: Limits = Limits {
    entries: 20,
    diagnostics: 10,
    commits: 10,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RepositoryInfo {
    pub id: String,
    pub name: String,
    pub path: String,
    pub model_root: String,
    pub model: ModelInfo,
    pub git: GitInfo,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ModelInfo {
    pub available: bool,
    pub source_file_count: usize,
    pub source_uris: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path_base: String,
    pub concept_count: usize,
    pub concepts: Vec<String>,
    pub materialization_count: usize,
    pub materializations: Vec<ModelItem>,
    pub link_count: usize,
    pub binding_count: usize,
    pub errors: usize,
    pub warnings: usize,
    pub diagnostics: Vec<ModelDiagnostic>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ModelItem {
    pub id: String,
    pub kind: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ModelGraph {
    pub available: bool,
    pub node_count: usize,
    pub link_count: usize,
    pub nodes: Vec<ModelItem>,
    pub links: Vec<GraphLink>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GraphLink {
    pub from: String,
    pub to: String,
    pub relation: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ModelDiagnostic {
    pub uri: String,
    pub severity: String,
    pub code: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GitInfo {
    pub available: bool,
    pub worktree_root: Option<String>,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub dirty: bool,
    pub change_count: usize,
    pub changes: Vec<GitChange>,
    pub branch_count: usize,
    pub branches: Vec<String>,
    pub recent_commits: Vec<GitCommit>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GitChange {
    pub status: String,
    pub path: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GitCommit {
    pub id: String,
    pub authored_at: String,
    pub subject: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GitHistoryPage {
    pub available: bool,
    pub offset: usize,
    pub commits: Vec<GitCommit>,
    pub next_offset: Option<usize>,
}

pub fn inspect(repo: &RegisteredRepository, limits: Limits) -> RepositoryInfo {
    let model_root = repo.model_root.as_deref().unwrap_or(&repo.root);
    RepositoryInfo {
        id: repo.id.clone(),
        name: repo
            .root
            .file_name()
            .unwrap_or(repo.root.as_os_str())
            .to_string_lossy()
            .into_owned(),
        path: repo.root.to_string_lossy().into_owned(),
        model_root: model_root.to_string_lossy().into_owned(),
        model: inspect_model(model_root, limits),
        git: inspect_git(&repo.root, limits),
    }
}

impl RepositoryInfo {
    pub fn for_hub(mut self, repo: &RegisteredRepository) -> Self {
        let model_root = repo.model_root.as_deref().unwrap_or(&repo.root);
        let in_repository = model_root.starts_with(&repo.root);
        let base = if in_repository {
            &repo.root
        } else {
            model_root
        };
        self.model.path_base = if in_repository {
            "repository"
        } else {
            "model_root"
        }
        .into();
        self.model.source_paths = self
            .model
            .source_uris
            .iter()
            .filter_map(|uri| relative_model_path(uri, base))
            .collect();
        self.model.source_uris.clear();
        for diagnostic in &mut self.model.diagnostics {
            diagnostic.uri = relative_model_path(&diagnostic.uri, base)
                .unwrap_or_else(|| "unknown source".into());
        }
        self.path = ".".into();
        self.model_root = if in_repository {
            let relative = model_root.strip_prefix(&repo.root).unwrap_or(Path::new(""));
            if relative.as_os_str().is_empty() {
                ".".into()
            } else {
                relative.to_string_lossy().into_owned()
            }
        } else {
            "external".into()
        };
        self.git.worktree_root = None;
        self
    }
}

fn relative_model_path(uri: &str, base: &Path) -> Option<String> {
    let path = uri_to_path(uri)?;
    // URI round trips remove the Windows extended-path prefix from canonical roots.
    let base = uri_to_path(&path_to_uri(base))?;
    Some(
        path.strip_prefix(&base)
            .ok()?
            .to_string_lossy()
            .replace('\\', "/"),
    )
}

pub fn inspect_graph(repo: &RegisteredRepository) -> ModelGraph {
    const NODE_LIMIT: usize = 500;
    const LINK_LIMIT: usize = 2_000;
    let root = repo.model_root.as_deref().unwrap_or(&repo.root);
    let mut sources = HashMap::new();
    if collect_sources_checked(root, &mut sources).is_err() {
        return ModelGraph {
            available: false,
            node_count: 0,
            link_count: 0,
            nodes: Vec::new(),
            links: Vec::new(),
            truncated: false,
        };
    }
    let map = analyze(&sources);
    let node_count = map.materializations.len();
    let link_count = map.links.len();
    let mut nodes: Vec<_> = map
        .materializations
        .iter()
        .map(|item| ModelItem {
            id: item.id.clone(),
            kind: item.kind.clone(),
            name: item.name.clone(),
        })
        .collect();
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    nodes.truncate(NODE_LIMIT);
    let included: HashSet<_> = nodes.iter().map(|item| item.id.as_str()).collect();
    let mut links: Vec<_> = map
        .links
        .iter()
        .filter(|link| included.contains(link.from.as_str()) && included.contains(link.to.as_str()))
        .map(|link| GraphLink {
            from: link.from.clone(),
            to: link.to.clone(),
            relation: link.relation.clone(),
        })
        .collect();
    links.sort_by(|a, b| (&a.from, &a.to, &a.relation).cmp(&(&b.from, &b.to, &b.relation)));
    links.truncate(LINK_LIMIT);
    ModelGraph {
        available: true,
        node_count,
        link_count,
        truncated: nodes.len() < node_count || links.len() < link_count,
        nodes,
        links,
    }
}

fn inspect_model(root: &Path, limits: Limits) -> ModelInfo {
    let empty = || ModelInfo {
        available: false,
        source_file_count: 0,
        source_uris: Vec::new(),
        source_paths: Vec::new(),
        path_base: String::new(),
        concept_count: 0,
        concepts: Vec::new(),
        materialization_count: 0,
        materializations: Vec::new(),
        link_count: 0,
        binding_count: 0,
        errors: 0,
        warnings: 0,
        diagnostics: Vec::new(),
        truncated: false,
    };
    let mut sources = HashMap::new();
    if collect_sources_checked(root, &mut sources).is_err() {
        return empty();
    }
    let map = analyze(&sources);
    let report = diagnostic_report(&sources, &map);
    let mut source_uris: Vec<_> = sources.keys().cloned().collect();
    source_uris.sort();
    let mut concepts: Vec<_> = map.concepts.iter().map(|item| item.word.clone()).collect();
    concepts.sort();
    let mut materializations: Vec<_> = map
        .materializations
        .iter()
        .map(|item| ModelItem {
            id: item.id.clone(),
            kind: item.kind.clone(),
            name: item.name.clone(),
        })
        .collect();
    materializations.sort_by(|a, b| a.id.cmp(&b.id));
    let truncated = source_uris.len() > limits.entries
        || concepts.len() > limits.entries
        || materializations.len() > limits.entries
        || report.diagnostics.len() > limits.diagnostics;
    ModelInfo {
        available: true,
        source_file_count: source_uris.len(),
        source_uris: source_uris.into_iter().take(limits.entries).collect(),
        source_paths: Vec::new(),
        path_base: String::new(),
        concept_count: concepts.len(),
        concepts: concepts.into_iter().take(limits.entries).collect(),
        materialization_count: materializations.len(),
        materializations: materializations.into_iter().take(limits.entries).collect(),
        link_count: map.links.len(),
        binding_count: map.bindings.len(),
        errors: report.errors,
        warnings: report.warnings,
        diagnostics: report
            .diagnostics
            .into_iter()
            .take(limits.diagnostics)
            .map(|item| ModelDiagnostic {
                uri: item.uri,
                severity: match item.severity {
                    crate::semantic::Severity::Error => "error".into(),
                    crate::semantic::Severity::Warning => "warning".into(),
                },
                code: item.code,
                message: item.message,
            })
            .collect(),
        truncated,
    }
}

pub fn inspect_git(root: &Path, limits: Limits) -> GitInfo {
    let unavailable = || GitInfo {
        available: false,
        worktree_root: None,
        branch: None,
        head: None,
        dirty: false,
        change_count: 0,
        changes: Vec::new(),
        branch_count: 0,
        branches: Vec::new(),
        recent_commits: Vec::new(),
        truncated: false,
    };
    let Some(worktree_root) = git(root, &["rev-parse", "--show-toplevel"]) else {
        return unavailable();
    };
    let Some(status) = git(
        root,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--no-renames",
            "--untracked-files=normal",
            "--",
            ".",
        ],
    ) else {
        return unavailable();
    };
    let changes: Vec<_> = status
        .split(|byte| *byte == 0)
        .filter(|entry| entry.len() >= 4 && entry[2] == b' ')
        .map(|entry| GitChange {
            status: String::from_utf8_lossy(&entry[..2]).into_owned(),
            path: String::from_utf8_lossy(&entry[3..]).into_owned(),
        })
        .collect();
    let branches: Vec<_> = git(
        root,
        &[
            "for-each-ref",
            "--sort=refname",
            "--format=%(refname:short)",
            "refs/heads",
            "refs/remotes",
        ],
    )
    .unwrap_or_default()
    .split(|byte| *byte == b'\n')
    .filter(|line| !line.is_empty())
    .map(|line| String::from_utf8_lossy(line).into_owned())
    .collect();
    let commits = parse_git_commits(
        git(
            root,
            &[
                "log",
                "-n",
                &limits.commits.to_string(),
                "--format=%H%x09%aI%x09%s",
            ],
        )
        .unwrap_or_default(),
    );
    let branch = git(root, &["symbolic-ref", "--short", "-q", "HEAD"])
        .map(|value| String::from_utf8_lossy(&value).trim().to_owned());
    let head = git(root, &["rev-parse", "--verify", "HEAD"])
        .map(|value| String::from_utf8_lossy(&value).trim().to_owned());
    GitInfo {
        available: true,
        worktree_root: Some(String::from_utf8_lossy(&worktree_root).trim().to_owned()),
        branch,
        head,
        dirty: !changes.is_empty(),
        change_count: changes.len(),
        changes: changes.iter().take(limits.entries).cloned().collect(),
        branch_count: branches.len(),
        branches: branches.iter().take(limits.entries).cloned().collect(),
        recent_commits: commits,
        truncated: changes.len() > limits.entries || branches.len() > limits.entries,
    }
}

pub fn inspect_git_history(root: &Path, offset: usize, limit: usize) -> GitHistoryPage {
    let unavailable = || GitHistoryPage {
        available: false,
        offset,
        commits: Vec::new(),
        next_offset: None,
    };
    if limit == 0 || limit > 100 || git(root, &["rev-parse", "--show-toplevel"]).is_none() {
        return unavailable();
    }
    if git(root, &["rev-parse", "--verify", "HEAD"]).is_none() {
        return GitHistoryPage {
            available: true,
            offset,
            commits: Vec::new(),
            next_offset: None,
        };
    }
    let skip = format!("--skip={offset}");
    let count = format!("-n{}", limit + 1);
    let Some(output) = git(root, &["log", &skip, &count, "--format=%H%x09%aI%x09%s"]) else {
        return unavailable();
    };
    let mut commits = parse_git_commits(output);
    let has_more = commits.len() > limit;
    commits.truncate(limit);
    GitHistoryPage {
        available: true,
        offset,
        commits,
        next_offset: has_more.then(|| offset.checked_add(limit)).flatten(),
    }
}

fn parse_git_commits(output: Vec<u8>) -> Vec<GitCommit> {
    output
        .split(|byte| *byte == b'\n')
        .filter_map(|line| {
            let line = String::from_utf8_lossy(line);
            let mut parts = line.splitn(3, '\t');
            Some(GitCommit {
                id: parts.next()?.into(),
                authored_at: parts.next()?.into(),
                subject: parts.next()?.chars().take(200).collect(),
            })
        })
        .collect()
}

fn git(root: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let output = Command::new("git")
        .arg("-c")
        .arg("core.fsmonitor=false")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use uuid::Uuid;

    fn run_git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn separate_model_root_and_non_git_directory_are_reported() {
        let root = std::env::temp_dir().join(format!("saml-repo-info-{}", Uuid::new_v4()));
        let source = root.join("source");
        let model = root.join("model");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&model).unwrap();
        fs::write(model.join("model.spec"), "concept Entity {}").unwrap();
        let info = inspect(
            &RegisteredRepository {
                id: Uuid::new_v4().to_string(),
                root: source.clone(),
                model_root: Some(model.clone()),
            },
            LOCAL_LIMITS,
        );
        assert_eq!(info.name, "source");
        assert_eq!(info.model_root, model.to_string_lossy());
        assert_eq!(info.model.source_file_count, 1);
        assert_eq!(info.model.concepts, vec!["entity"]);
        assert!(!info.git.available);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn hub_paths_are_relative_and_external_models_are_labeled() {
        let root = std::env::temp_dir().join(format!("saml-repo-paths-{}", Uuid::new_v4()));
        let source = root.join("source");
        let model = root.join("model");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&model).unwrap();
        fs::write(model.join("spec.saml"), "concept Entity {}").unwrap();
        let external = RegisteredRepository {
            id: Uuid::new_v4().to_string(),
            root: source.clone(),
            model_root: Some(model.clone()),
        };
        let info = inspect(&external, HUB_LIMITS).for_hub(&external);
        assert_eq!(info.path, ".");
        assert_eq!(info.model_root, "external");
        assert_eq!(info.model.path_base, "model_root");
        assert_eq!(info.model.source_paths, ["spec.saml"]);
        assert!(info.model.source_uris.is_empty());
        assert!(!serde_json::to_string(&info)
            .unwrap()
            .contains(&root.to_string_lossy().to_string()));

        fs::create_dir_all(source.join("models")).unwrap();
        fs::write(source.join("models/local.saml"), "concept Local {}").unwrap();
        let internal = RegisteredRepository {
            id: Uuid::new_v4().to_string(),
            root: source.clone(),
            model_root: Some(source.join("models")),
        };
        let info = inspect(&internal, HUB_LIMITS).for_hub(&internal);
        assert_eq!(info.model_root, "models");
        assert_eq!(info.model.path_base, "repository");
        assert_eq!(info.model.source_paths, ["models/local.saml"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn graph_report_contains_resolved_links() {
        let root = std::env::temp_dir().join(format!("saml-repo-graph-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("model.spec"),
            "concept Data {}\nconcept Action { target { type: Data } }",
        )
        .unwrap();
        fs::write(
            root.join("model.saml"),
            "data Shared {}\naction Use { target: Shared }",
        )
        .unwrap();
        let repo = RegisteredRepository {
            id: Uuid::new_v4().to_string(),
            root: root.clone(),
            model_root: None,
        };
        let graph = inspect_graph(&repo);
        assert!(graph.available);
        assert_eq!(graph.node_count, 2);
        assert_eq!(graph.link_count, 1);
        assert_eq!(graph.links[0].relation, "property:target");
        assert!(graph
            .nodes
            .iter()
            .any(|node| node.id == graph.links[0].from));
        assert!(graph.nodes.iter().any(|node| node.id == graph.links[0].to));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn git_state_branches_and_history_are_bounded() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let root = std::env::temp_dir().join(format!("saml-repo-git-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        run_git(&root, &["init", "-q", "-b", "main"]);
        assert!(inspect_git_history(&root, 0, 10).available);
        assert!(inspect_git_history(&root, 0, 10).commits.is_empty());
        fs::write(root.join("README.md"), "first\n").unwrap();
        run_git(&root, &["add", "README.md"]);
        run_git(
            &root,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-qm",
                "First commit",
            ],
        );
        run_git(&root, &["branch", "topic"]);
        fs::write(root.join("README.md"), "second\n").unwrap();
        fs::write(root.join("new.txt"), "new\n").unwrap();
        let info = inspect_git(
            &root,
            Limits {
                entries: 1,
                diagnostics: 1,
                commits: 1,
            },
        );
        assert!(info.available);
        assert_eq!(info.branch.as_deref(), Some("main"));
        assert_eq!(info.change_count, 2);
        assert!(info.dirty);
        assert_eq!(info.changes.len(), 1);
        assert_eq!(info.branch_count, 2);
        assert_eq!(info.recent_commits[0].subject, "First commit");
        assert!(info.truncated);
        fs::write(root.join("README.md"), "third\n").unwrap();
        run_git(&root, &["add", "README.md"]);
        run_git(
            &root,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-qm",
                "Second commit",
            ],
        );
        let first = inspect_git_history(&root, 0, 1);
        assert_eq!(first.commits[0].subject, "Second commit");
        assert_eq!(first.next_offset, Some(1));
        let second = inspect_git_history(&root, 1, 1);
        assert_eq!(second.commits[0].subject, "First commit");
        assert_eq!(second.next_offset, None);
        fs::remove_dir_all(root).unwrap();
    }
}
