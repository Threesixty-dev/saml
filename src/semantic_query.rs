use crate::semantic::{SemanticMap, SourceSpan};
use serde_json::{json, Value};
use std::collections::{hash_map::DefaultHasher, BTreeSet};
use std::hash::{Hash, Hasher};

pub const SEARCH_PAGE_SIZE: usize = 50;

pub fn workflow() -> Value {
    json!({
        "schema_version": 1,
        "steps": [
            {"action": "select_workspace", "request": "GET /api/workspaces", "instruction": "Choose one workspace ID and reuse it in every following request."},
            {"action": "find_item", "request": "GET /api/query?workspace={workspace_id}&q={url_encoded_keywords}", "instruction": "Extract a model name, concept, or property from the question. Search matches keywords; it does not interpret natural language. Follow next_url until the intended item is found or no page remains."},
            {"action": "inspect_item", "request": "GET /api/item?workspace={workspace_id}&kind={kind}&id={url_encoded_id}", "instruction": "Follow a detail_url from search. Inspect definitions, properties, links, and bindings."},
            {"action": "read_source", "request": "GET /api/source?workspace={workspace_id}&uri={url_encoded_source_uri}", "instruction": "Follow a source URL from item details to verify saved declaration text. Spans use UTF-8 byte offsets."},
            {"action": "check_model", "request": "GET /api/diagnostics?workspace={workspace_id}", "instruction": "Check syntax and semantic findings before relying on the answer."}
        ],
        "repository_steps": [
            {"action": "select_workspace", "request": "GET /api/workspaces", "instruction": "Choose a registered repository ID."},
            {"action": "inspect_repository", "request": "GET /api/repository?workspace={workspace_id}", "instruction": "Read identity, model inventory and diagnostics summary, Git state, branches, and recent commits. Counts describe the full repository even if lists are truncated."},
            {"action": "inspect_git", "request": "GET /api/git?workspace={workspace_id}", "instruction": "Use for a focused Git view."},
            {"action": "page_git_history", "request": "GET /api/git/history?workspace={workspace_id}&offset={offset}&limit={limit}", "instruction": "For older commits, start at offset 0 and follow next_offset until null. Use /api/map and /api/diagnostics for complete model data."}
        ],
        "full_map": "GET /api/map?workspace={workspace_id}"
    })
}

pub fn encode(value: &str) -> String {
    let mut output = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            output.push(byte as char);
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}

fn item_url(workspace: &str, kind: &str, id: &str) -> String {
    format!(
        "/api/item?workspace={}&kind={kind}&id={}",
        encode(workspace),
        encode(id)
    )
}

fn source_urls(workspace: &str, spans: &[&SourceSpan]) -> Vec<Value> {
    let uris: BTreeSet<_> = spans.iter().map(|span| span.uri.as_str()).collect();
    uris.into_iter()
        .map(|uri| {
            json!({
                "uri": uri,
                "url": format!("/api/source?workspace={}&uri={}", encode(workspace), encode(uri)),
            })
        })
        .collect()
}

pub fn search_with_cursor(
    map: &SemanticMap,
    workspace: &str,
    query: &str,
    cursor: Option<&str>,
) -> Result<Value, &'static str> {
    let expected = cursor.map(parse_search_cursor).transpose()?;
    let offset = expected.map_or(0, |(offset, _)| offset);
    let (mut result, next_offset, fingerprint) = search_page(map, workspace, query, offset);
    if let Some((offset, expected_fingerprint)) = expected {
        let total = result["total"].as_u64().unwrap_or(0) as usize;
        if expected_fingerprint != fingerprint || offset >= total {
            return Err("Search cursor is stale or does not match this workspace and query; restart the search");
        }
    }
    if let Some(next_offset) = next_offset {
        result["next_cursor"] = json!(format!("{next_offset:x}.{fingerprint:x}"));
    }
    Ok(result)
}

fn parse_search_cursor(cursor: &str) -> Result<(usize, u64), &'static str> {
    if cursor.len() > 64 {
        return Err("Invalid search cursor; restart the search");
    }
    let (offset, fingerprint) = cursor
        .split_once('.')
        .ok_or("Invalid search cursor; restart the search")?;
    let offset = usize::from_str_radix(offset, 16)
        .map_err(|_| "Invalid search cursor; restart the search")?;
    let fingerprint = u64::from_str_radix(fingerprint, 16)
        .map_err(|_| "Invalid search cursor; restart the search")?;
    if offset == 0 || offset % SEARCH_PAGE_SIZE != 0 {
        return Err("Invalid search cursor; restart the search");
    }
    Ok((offset, fingerprint))
}

pub fn search_page(
    map: &SemanticMap,
    workspace: &str,
    query: &str,
    offset: usize,
) -> (Value, Option<usize>, u64) {
    let terms: Vec<_> = query.split_whitespace().map(str::to_lowercase).collect();
    let needle = query.trim().to_lowercase();
    let mut matches = Vec::new();

    for concept in &map.concepts {
        let searchable = format!(
            "{} {} {}",
            concept.word,
            concept.roles.join(" "),
            concept
                .properties
                .iter()
                .map(|property| property.name.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        )
        .to_lowercase();
        if terms.iter().all(|term| searchable.contains(term)) {
            let rank = rank(&concept.word, &needle);
            matches.push((
                rank,
                concept.word.clone(),
                json!({
                    "kind": "concept", "id": concept.word, "name": concept.word,
                    "roles": concept.roles, "definitions": concept.definitions,
                    "detail_url": item_url(workspace, "concept", &concept.word),
                }),
            ));
        }
    }

    for item in &map.materializations {
        let searchable = format!(
            "{} {} {} {} {}",
            item.name,
            item.kind,
            item.id,
            item.fields
                .iter()
                .map(|field| field.name.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            item.parameters
                .iter()
                .map(|parameter| parameter.name.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        )
        .to_lowercase();
        if terms.iter().all(|term| searchable.contains(term)) {
            let rank = rank(&item.name, &needle);
            matches.push((
                rank,
                item.id.clone(),
                json!({
                    "kind": "materialization", "id": item.id, "name": item.name,
                    "model_kind": item.kind, "scope": item.scope,
                    "definitions": item.definitions,
                    "detail_url": item_url(workspace, "materialization", &item.id),
                }),
            ));
        }
    }

    matches.sort_by(|left, right| (left.0, &left.1).cmp(&(right.0, &right.1)));
    let total = matches.len();
    let mut hasher = DefaultHasher::new();
    workspace.hash(&mut hasher);
    query.hash(&mut hasher);
    for (rank, id, _) in &matches {
        rank.hash(&mut hasher);
        id.hash(&mut hasher);
    }
    let fingerprint = hasher.finish();
    let next_offset = offset
        .checked_add(SEARCH_PAGE_SIZE)
        .filter(|next| *next < total);
    let result = json!({
        "schema_version": 1,
        "workspace": workspace,
        "query": query,
        "total": total,
        "matches": matches.into_iter().skip(offset).take(SEARCH_PAGE_SIZE).map(|(_, _, value)| value).collect::<Vec<_>>(),
    });
    (result, next_offset, fingerprint)
}

fn rank(name: &str, needle: &str) -> u8 {
    let name = name.to_lowercase();
    if name == needle {
        0
    } else if name.starts_with(needle) {
        1
    } else {
        2
    }
}

pub fn item(map: &SemanticMap, workspace: &str, kind: &str, id: &str) -> Option<Value> {
    match kind {
        "concept" => {
            let concept = map.concepts.iter().find(|item| item.word == id)?;
            let mut spans: Vec<_> = concept.definitions.iter().collect();
            spans.extend(concept.uses.iter().map(|use_site| &use_site.source));
            for property in &concept.properties {
                spans.extend(property.definitions.iter());
                spans.extend(property.uses.iter());
            }
            let materializations: Vec<_> = map
                .materializations
                .iter()
                .filter(|item| item.kind == concept.word)
                .map(|item| {
                    json!({
                        "id": item.id,
                        "name": item.name,
                        "detail_url": item_url(workspace, "materialization", &item.id),
                    })
                })
                .collect();
            Some(json!({
                "schema_version": 1, "workspace": workspace, "kind": kind,
                "item": concept, "materializations": materializations,
                "sources": source_urls(workspace, &spans),
            }))
        }
        "materialization" => {
            let item = map.materializations.iter().find(|item| item.id == id)?;
            let outgoing_links: Vec<_> = map.links.iter().filter(|link| link.from == id).collect();
            let incoming_links: Vec<_> = map.links.iter().filter(|link| link.to == id).collect();
            let bindings: Vec<_> = map
                .bindings
                .iter()
                .filter(|binding| binding.from == id || binding.target == id)
                .collect();
            let mut spans: Vec<_> = item.definitions.iter().collect();
            spans.extend(item.parameters.iter().map(|parameter| &parameter.source));
            for field in &item.fields {
                spans.extend(
                    field
                        .occurrences
                        .iter()
                        .map(|occurrence| &occurrence.source),
                );
            }
            spans.extend(
                outgoing_links
                    .iter()
                    .chain(incoming_links.iter())
                    .map(|link| &link.source),
            );
            spans.extend(bindings.iter().map(|binding| &binding.source));
            let related_ids: BTreeSet<_> = outgoing_links
                .iter()
                .map(|link| link.to.as_str())
                .chain(incoming_links.iter().map(|link| link.from.as_str()))
                .chain(bindings.iter().map(|binding| {
                    if binding.from == id {
                        binding.target.as_str()
                    } else {
                        binding.from.as_str()
                    }
                }))
                .filter(|related| *related != id)
                .collect();
            let related: Vec<_> = related_ids
                .into_iter()
                .map(|related| {
                    json!({
                        "id": related,
                        "detail_url": item_url(workspace, "materialization", related),
                    })
                })
                .collect();
            Some(json!({
                "schema_version": 1, "workspace": workspace, "kind": kind,
                "item": item, "outgoing_links": outgoing_links,
                "incoming_links": incoming_links, "bindings": bindings,
                "related": related, "sources": source_urls(workspace, &spans),
            }))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic::analyze;
    use std::collections::HashMap;

    #[test]
    fn search_and_detail_expose_exact_identity_and_links() {
        let sources = HashMap::from([
            (
                "file:///model.spec".into(),
                "concept Entity { peer { type: Entity } }".into(),
            ),
            (
                "file:///model.saml".into(),
                "entity User { peer: Admin }\nentity Admin { peer: User }".into(),
            ),
        ]);
        let map = analyze(&sources);
        let results = search_with_cursor(&map, "repo-1", "User", None).unwrap();
        assert_eq!(results["total"], 1);
        assert_eq!(results["matches"][0]["id"], "/entity:User");
        let detail = item(&map, "repo-1", "materialization", "/entity:User").unwrap();
        assert!(detail["outgoing_links"]
            .as_array()
            .unwrap()
            .iter()
            .any(|link| link["to"] == "/entity:Admin"));
        assert_eq!(
            detail["related"][0]["detail_url"],
            "/api/item?workspace=repo-1&kind=materialization&id=%2Fentity%3AAdmin"
        );
        assert!(item(&map, "repo-1", "materialization", "/entity:Missing").is_none());
    }

    #[test]
    fn same_name_in_different_scopes_has_distinct_details() {
        let sources = HashMap::from([
            (
                "file:///model.spec".into(),
                "boundary Scope {}\nconcept Entity {}".into(),
            ),
            (
                "file:///model.saml".into(),
                "scope First entity User {}\nscope Second entity User {}".into(),
            ),
        ]);
        let map = analyze(&sources);
        let results = search_with_cursor(&map, "repo-1", "User", None).unwrap();
        assert_eq!(results["total"], 2);
        let ids: Vec<_> = results["matches"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["id"].as_str().unwrap())
            .collect();
        assert_ne!(ids[0], ids[1]);
        assert!(ids
            .iter()
            .all(|id| item(&map, "repo-1", "materialization", id).is_some()));
    }
}
