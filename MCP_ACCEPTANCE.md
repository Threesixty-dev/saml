# MCP agent-host acceptance

Run on 2026-10-25 against `project/example-1` from the isolated
`codex/semantic-model-query` worktree. The server was built with
`cargo build --offline --bin saml-agent`, then a temporary registry was created
with `saml-agent init` and `saml-agent add --repo project/example-1`.

An actual Codex CLI agent host launched this server over stdio using an
invocation-local `mcp_servers.saml` configuration. The host used its automatic
approval mode and was instructed to use only SAML MCP tools for model facts,
without reading repository files directly. The host completed all nine
questions below. Its tool traces had no MCP call errors, and the answers were
checked against the returned item/source/diagnostic data.

| Question | Verified answer |
| --- | --- |
| Initiator of the ArticlePublication action? | `/scope:Identity/entity:User`, shown in the scoped action declaration in `articles.saml`. |
| Stages of the ArticlePublication workflow? | `Evaluation`, then `Publication` after successful `Evaluation`. |
| What does UserPortalBackend expose? | Its `RestApi` capability binds `Article` and `Author` data from the Article scope. |
| Environment and technology of ApiService? | `Server` and `Express`. |
| Properties of the Application concept? | `type` and `capability`. |
| Which stages extend CommonStage? | `Evaluation` and `Publication`; `Evaluation` also extends `SpecialStage`. |
| Which items have missing required properties? | Six diagnostics: `UserPortalFrontend`, `CommonStage`, `SpecialStage`, `Publication`, `Dupa`, and `Authentication`. |
| What is RestApi? | Two distinct items: an `applicationInterface` and an `applicationCapability`. |
| Production URL of UserPortalBackend? | The saved model does not specify one; the host abstained. |

The example model has six pre-existing `missing-required-property` errors and
zero warnings. They do not concern the action initiator, either `RestApi` item,
or `UserPortalBackend`. They remain visible through `get_diagnostics`.

Separately, a direct stdio probe completed `server/discover`, `tools/list`, and
`tools/call` with protocol `2026-07-28` and its required per-request metadata.
The legacy `initialize` flow is covered by `tests/mcp_scenarios.rs`. A regression
test now covers modern discovery, a tool call, and rejection of a request
without the required metadata.

The temporary registry and Codex host config are not part of the repository.
To repeat the host run, use the command and client-config template in
`MCP_SERVER.md`, register `project/example-1`, and ask the questions above.
