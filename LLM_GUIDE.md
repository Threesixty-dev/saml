# SAML guide for LLM agents

SAML models software architecture. The `.spec` manifest defines a workspace's
vocabulary; `.saml` files use that vocabulary to describe a model. Treat the
saved files and the semantic analyzer as the source of truth. Do not infer that
a model is valid just because its prose sounds plausible.

The tools can be installed with `cargo install --path /path/to/saml` from a
checkout of the SAML repository. An MCP client can launch `saml-agent mcp`;
other hosts need file and command access or local HTTP access to `saml-view`.

## Language essentials

- The built-in declaration words are `concept`, `boundary`, and `rule`.
- Put vocabulary declarations in a `.spec` file or a file named `spec.saml`.
  `concept Entity {}` introduces the model word `entity`; `boundary Scope {}`
  introduces `scope`. Every manifest in one repository contributes vocabulary.
  Repositories are independent; do not use a concept from one repository to
  interpret another.
- Use the introduced word to create a named materialization: `entity User {}`.
- A block inside a concept declares a property. For example,
  `concept Action { initiator { type: Entity } }` requires an `initiator` value
  on every `action` materialization. `type: [Entity]` requires a list.
- Names can be split into fragments across files. Fragments of one
  materialization must agree on visibility and input types, and must not assign
  the same declared property twice.
- Materializations inside boundaries are private by default. Write `Public`
  immediately before the kind when another boundary needs to reference one.

The full current rules and known gaps are in `LANGUAGE_SPECIFICATION.md`,
`LANGUAGE_SERVER.md`, and `SEMANTIC_MODEL.md` in the SAML repository.

## Agent workflow

1. Read the workspace's `.spec` files and any `spec.saml` before writing model
   files. Reuse existing concepts and property rules.
2. Edit `.saml` and `.spec` files with ordinary file tools. Keep the intended
   architecture explicit in names, boundaries, typed properties, and links.
3. Run `saml-check /path/to/workspace` after changes. Its JSON output combines
   syntax and semantic diagnostics. Exit code 1 means errors; warnings alone do
   not fail. Fix errors before treating the model as ready.
4. Run `saml-map /path/to/workspace` when you need the full JSON map. Inspect
   `concepts`, `materializations`, `links`, `bindings`, and `findings` rather
   than guessing what a reference resolved to.
5. For one-off use, run `saml-view /path/to/workspace`. For a persistent
   multi-repository service, register roots with `saml-agent add` and run
   `saml-agent run --config /path/to/agent.json`. Open
   `http://127.0.0.1:8787` for the graph. A tool-enabled LLM can fetch
   `/api/guide`, `/api/workflow`, `/api/diagnostics`, `/api/map`,
   `/api/workspaces`, `/api/repository`, `/api/git`, and `/api/status` from the same address. The agent's
   repository IDs remain stable as other repositories are added or removed.

## Ask about a model element over MCP

The MCP server is available with `saml-agent mcp --config
/absolute/path/to/agent.json`. Register repositories with `saml-agent add`
first. MCP clients discover typed tool inputs and receive short usage
instructions during initialization. Follow the returned `get_workflow` steps:

1. `list_workspaces` selects a persistent workspace ID.
2. `search_model` finds a concept or materialization by name, kind, scope,
   field, or property keyword. Follow `next_page` while more matches remain,
   then follow the intended match's `next_tool` arguments. If a cursor is
   rejected after the model changes, restart the search.
3. `get_model_item` returns the exact item, its incoming and outgoing links,
   bindings, and source URIs. Related items include their own `next_tool`.
4. `read_model_source` reads an exact source URI from that workspace; compare
   the saved text with the item's UTF-8 byte spans.
5. `get_diagnostics` checks findings. `get_semantic_map` provides the complete
   graph when needed; `get_guide` returns this guide.

For repository-wide questions, call `list_workspaces`, then
`get_repository_info`. It reports identity and paths, a bounded semantic
inventory and diagnostics summary, working-tree changes, branches, and recent
commits. Use `get_repository_git` for a focused Git response and
`get_repository_git_history` to page through older commits. Counts describe
the full repository even when lists are truncated; use `get_semantic_map` and
`get_diagnostics` for complete model data.

The server reports analyzer facts and does keyword lookup. Extract a model
term from a broader question and explain any inference separately. See
`MCP_SERVER.md` for client configuration and the full tool contract.

## Ask about a model element over HTTP

`GET /api/workflow` returns a machine-readable sequence for looking up one
piece of a saved model. Use the same sequence with `saml-agent` or `saml-view`:

1. Call `GET /api/workspaces` and choose a workspace ID. Include
   `workspace=<id>` in subsequent requests. Without it, the first registered
   workspace is selected.
2. Extract a model name, concept word, or property name from the question and
   call `GET /api/query?workspace=<id>&q=<URL-encoded keywords>`. Search is
   case-insensitive and requires every keyword to occur in a concept or
   materialization's indexed names. It does not interpret natural-language
   questions. Results include a `kind`, an exact `id`, source definitions,
   and a `detail_url`; at most 50 matches are returned, with `total` giving
   the full count. Follow `next_url` until the intended item is found or no
   page remains. If a cursor is rejected after the model changes, restart
   the search.
3. Follow a result's `detail_url`, or call
   `GET /api/item?workspace=<id>&kind=<kind>&id=<URL-encoded id>`. Concept
   details include property rules and materializations. Materialization
   details include merged fields, parameters, incoming and outgoing resolved
   links, and bindings. Equal names in different scopes have different IDs.
4. Follow a URL in `sources` to retrieve the exact saved `.spec` or `.saml`
   text with `GET /api/source?workspace=<id>&uri=<URL-encoded file URI>`.
   Source spans use UTF-8 byte offsets in that text. Only sources belonging
   to the selected model workspace can be retrieved.
5. Call `GET /api/diagnostics?workspace=<id>` to check findings. Use
   `GET /api/map?workspace=<id>` when the complete graph is needed.

`GET /api/repository?workspace=<id>` and `GET /api/git?workspace=<id>` provide
the same repository and Git snapshots to local HTTP clients. For older commits,
use `GET /api/git/history?workspace=<id>&offset=0&limit=50` and follow
`next_offset` until it is null.

For example, to answer what `User` references, search for `User`, follow its
materialization `detail_url`, inspect `outgoing_links`, then read the linked
source and diagnostics. These endpoints report analyzer facts; the caller
should explain any inference separately.

If model files live outside the source repository, register the repository
with `--model-root`:

```sh
saml-agent add --config /path/to/agent.json --repo /path/to/repo --model-root /path/to/model
```

The local portal names the source repository and analyzes the separate model
workspace.

The portal reads saved files on each API request. Save changes before fetching
its data. The current analyzer checks specified structural and typed-reference
rules; it does not prove that an implementation fulfills the architecture or
validate every unrestricted prose relation.
