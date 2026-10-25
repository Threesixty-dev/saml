# MCP server test scenarios

These scenarios exercise `saml-agent mcp` as an external MCP client would: start
the binary with a temporary registry, exchange newline-delimited JSON-RPC over
stdio, and inspect the actual tool responses. Each scenario gets an isolated
registry and model fixture. Each ID has a matching integration test in
`tests/mcp_scenarios.rs`; run them with
`cargo test --offline --test mcp_scenarios`. The acceptance target is all 20
scenarios passing.

| ID | Scenario | Action and expected result |
| --- | --- | --- |
| MCP-01 | Invalid registry at startup | Start with a missing or invalid config. The process exits nonzero, writes an actionable error to stderr, and emits no JSON-RPC on stdout. |
| MCP-02 | Initialization and lifecycle | Send `initialize` with a supported protocol version, then `notifications/initialized`. The server identifies itself, advertises tools and investigation instructions, answers subsequent requests, and exits after stdin closes. |
| MCP-03 | Tool discovery and schemas | `tools/list` exposes exactly the eight documented tool names. Parameterized tools declare their required arguments and reject unexpected fields. |
| MCP-04 | Investigation workflow | `get_workflow` gives ordered workspace, search, item, source, and diagnostics steps. The names and argument hints lead to callable tools; the graph and guide are discoverable as optional steps. |
| MCP-05 | Result envelopes | Successful JSON tools return `structuredContent` and a parseable text fallback with equivalent data. Tool-level failures use `isError` and explain how to recover. |
| MCP-06 | Empty registry | `list_workspaces` on a valid registry with no repositories returns an empty array, while `get_workflow` and `get_guide` still work. |
| MCP-07 | External model root | Register a source repository with a separate model root. Listing reports both paths and search finds only model-root content. The repository ID remains stable across calls. |
| MCP-08 | Exact and case-insensitive search | Search for a model name with different casing. The exact name ranks first, the total matches the returned items, and each match includes an exact `get_model_item` follow-up. |
| MCP-09 | Keyword search across model metadata | Search by concept role/property and materialization kind/scope/field with multiple terms. All terms must match one item; results must be deterministic. |
| MCP-10 | No matches and invalid queries | A valid missing term returns zero matches. Empty, whitespace-only, and oversized queries return tool errors rather than scanning the entire map. |
| MCP-11 | Same name in different scopes | Two materializations named alike in different scopes yield different IDs. Following each result returns the correct scope and source declaration. |
| MCP-12 | Bounded large result set | More than 50 matching elements produce a correct `total`, at most 50 matches per page, stable ordering, and a `next_page` call that reaches the remaining IDs. |
| MCP-13 | Concept detail | An exact concept ID returns roles, properties, definitions, its materializations, and source follow-ups. Every materialization follow-up can be called. |
| MCP-14 | Materialization relationships | An exact materialization ID returns outgoing and incoming links, bindings when present, and related-element follow-ups that resolve to the expected IDs. |
| MCP-15 | Source evidence and UTF-8 spans | Follow a source URI from an item. The returned saved file contains the declaration, and its source span byte offsets select the expected UTF-8 text. |
| MCP-16 | Diagnostics on broken model | Introduce a syntax or semantic defect. `get_diagnostics` reports the right file, error count, severity, and range, without making search and source reads unusable. |
| MCP-17 | Complete map and guide | `get_semantic_map` includes the same concept, materialization, and relationship identities returned by focused tools. `get_guide` returns the agent guide as text. |
| MCP-18 | Bad arguments and unknown IDs | Missing/unknown arguments, an invalid item kind, an unknown workspace, and an unknown item produce protocol or tool errors without terminating the server. A valid call afterward succeeds. |
| MCP-19 | Source isolation | A URI from another registered workspace, a forged local file URI, and a non-model file cannot be read through `read_model_source`. A valid URI in the selected workspace succeeds. |
| MCP-20 | Live refresh and clean shutdown | Add a repository and edit a saved model while the MCP process stays open. The next calls see the new workspace/content; closing stdin terminates the process cleanly. |

## Counter-scenarios

Each counter-scenario records a gap in the first version of the test suite.
They are retained as adversarial checks; the expanded tests exercise the
supported behaviors. MCP-03 and MCP-12 also exposed behavior that was fixed.

| ID | Contradictory case to try | Gap in the initial test |
| --- | --- | --- |
| MCP-01 | Start with malformed JSON or an unsupported registry version. | The initial test used only a missing file and checked that stderr was nonempty; an unhelpful error could pass. |
| MCP-02 | Call a tool before `notifications/initialized`, or negotiate a different protocol version. | Initially, only the successful handshake with one version and clean EOF was exercised. |
| MCP-03 | Send an extra field to a no-argument tool, or a wrong JSON type to a typed tool. | The initial test checked one typed tool's extra field. We observed `list_workspaces` accept an extra argument even though its schema had no inputs. |
| MCP-04 | Follow the entire advertised sequence through `get_diagnostics` on a broken model. | The initial test verified step names but stopped the actual chain after `read_model_source`. |
| MCP-05 | Compare text fallback with `structuredContent` for every JSON tool, including nested item and map results. | Initially only `list_workspaces` was compared; another tool could return mismatched or absent fallback text. |
| MCP-06 | Call `search_model` against an empty registry and confirm an actionable error. | Listing, workflow, and guide alone could pass while a lookup crashed or returned stale data. |
| MCP-07 | Ask `read_model_source` for a `.spec` URI from the source repository when a different model root is registered. | Search excluded the decoy file, but source isolation for this arrangement was untested. |
| MCP-08 | Follow every match from a two-hit search, including the lower-ranked result. | The initial test validated only the first match's `next_tool`; the second could be missing or wrong. |
| MCP-09 | Search separately by role, kind, scope, field, and property, then use terms drawn from different items. | The initial two queries did not establish that each metadata dimension works independently or that cross-item terms are rejected. |
| MCP-10 | Send a query of exactly 200 UTF-8 bytes and one of 201 bytes. | The initial test checked only 201 ASCII bytes; the advertised upper boundary could be rejected or counted as characters. |
| MCP-11 | Add same-named scoped elements with different outgoing links and follow each link. | IDs, scopes, and source lines differ in the test, but link resolution could still cross scopes incorrectly. |
| MCP-12 | Search for 51 matches without knowing how to narrow the query, then try to inspect the omitted item. | The server originally returned 50 matches with no cursor, leaving the omitted ID unavailable for follow-up. |
| MCP-13 | Declare several roles and fragments for one concept, then inspect all returned rules and locations. | The initial test never asserted `roles` and checked only that some property and definition existed. |
| MCP-14 | Bind multiple arguments to different targets and verify each resolved target and source span. | A binding with wrong argument values could pass because only its count was checked. |
| MCP-15 | Put multibyte text before several source spans and verify every span against the returned file. | The initial test validated only one name span; later or cross-file offsets could be wrong. |
| MCP-16 | Assert the exact diagnostic code and line for a known defect, then fix it and confirm the diagnostic disappears. | Any error in the right file with a numeric range satisfied the initial test, even if the reported finding was unrelated. |
| MCP-17 | Compare every focused concept, materialization, link, and binding with the complete map. | The initial test checked one concept, one materialization, and one link; other map entries could be omitted. |
| MCP-18 | Send malformed JSON-RPC, an unknown method, and wrong argument types, then make a valid call. | The initial test covered typed argument and ID failures, but not framing or unknown-method recovery. |
| MCP-19 | Try URI aliases, encoded paths, and a source file replaced by a symlink between calls. | Three denied URIs established useful boundaries but did not cover alternate representations or file changes. |
| MCP-20 | Delete or rename a model file and remove a repository while the process stays open. | Additions and edits refreshed in the initial test; removals and stale results remained unchecked. |

The follow-up suite now checks registry errors, alternate protocol versions,
modern `server/discover` and required request metadata,
strict tool inputs, the complete workflow, fallbacks, empty and external model
roots, all search follow-ups, keyword dimensions and byte limits, scoped links,
pagination and stale cursors, concept fragments, binding targets and spans,
exact diagnostics and their removal, map consistency, source URI aliases and
symlinks, and live removals. The MCP SDK intentionally ignores syntactically
invalid JSON lines; MCP-18 verifies that a valid request still works afterward.
Calls before initialization are outside the supported client sequence.

The temporary fixtures must not depend on the repository's example model being
diagnostic-free. An additional manual smoke run against `project/example-1`
checks that the server can inspect a real model and report its existing
diagnostics. The Rust suite, formatting check, and diff check complete the
verification pass.
