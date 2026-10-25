# SAML MCP server

`saml-agent mcp` runs a Model Context Protocol server over standard input and
output. An MCP client launches one agent process and uses its registered
repositories. The MCP mode does not require a hub, heartbeat token, or the
local HTTP portal.

## Set up

Install the binaries, create a registry, and register a repository:

```sh
cargo install --path .
saml-agent init --config /absolute/path/to/agent.json
saml-agent add --config /absolute/path/to/agent.json --repo /absolute/path/to/repo
```

If the model lives outside the source repository, add `--model-root
/absolute/path/to/model` to the `add` command. Configure your MCP client to
launch the executable with these arguments (the surrounding client config
format varies):

```json
{
  "command": "saml-agent",
  "args": ["mcp", "--config", "/absolute/path/to/agent.json"]
}
```

For local development, use `cargo run --bin saml-agent -- mcp --config
/absolute/path/to/agent.json`. Standard output is reserved for MCP JSON-RPC
messages; diagnostic output goes to standard error. The process exits when its
MCP client closes standard input.

## Tools

The server advertises tool descriptions and input schemas during MCP tool
discovery. Legacy `initialize` and modern `server/discover` both return concise
usage instructions. The server accepts MCP protocol versions `2025-06-18`,
`2025-11-25`, and `2026-07-28`; modern requests include the standard
per-request `_meta` protocol, client-capabilities, and client-info fields.

| Tool | Purpose | Arguments |
| --- | --- | --- |
| `list_workspaces` | List registered repositories and persistent IDs | None |
| `get_workflow` | Return ordered instructions for investigating a model element | None |
| `search_model` | Find concepts and materializations by name, kind, scope, field, or property keywords | `workspace_id`, `query`, optional `cursor` |
| `get_model_item` | Inspect an exact concept or materialization, resolved links, bindings, and source locations | `workspace_id`, `kind`, `id` |
| `read_model_source` | Read one saved `.spec` or `.saml` file from the selected model workspace | `workspace_id`, `uri` |
| `get_diagnostics` | Check syntax and semantic findings | `workspace_id` |
| `get_semantic_map` | Retrieve the complete semantic map | `workspace_id` |
| `get_guide` | Read the SAML agent guide | None |

Start with `get_workflow` and `list_workspaces`. For a question such as “what
does User reference?”, call `search_model` with `User`, follow the returned
`next_tool` arguments to `get_model_item`, inspect `outgoing_links`, and read
a relevant source URI with `read_model_source`. Check `get_diagnostics` before
relying on the answer. Search is case-insensitive keyword lookup, so the
calling agent extracts a model term from a broader question. It does not
synthesize an answer itself.

Search returns at most 50 matches per call. When more remain, pass the returned
`next_page` tool name and arguments back to the server. Its `cursor` is opaque
and bound to the workspace, query, and ordered result IDs. If those results
change before the next call, the server returns an error asking the agent to
restart the search. The last page has no `next_page` or `next_cursor`.

Tool results include JSON `structuredContent` and a text fallback. Lookup
failures return MCP tool errors with actionable messages. Materialization IDs
include scope, so equal names in different scopes can be inspected separately.
Source spans use UTF-8 byte offsets in the returned saved source. The registry
is reloaded for each call, and each model is analyzed from saved files; edits
to the registry or model do not require restarting the MCP process.

The server reads only repositories registered in the local agent config.
`read_model_source` accepts only exact URIs found among the selected model's
`.spec` and `.saml` files; it cannot read arbitrary local paths.
Tools without inputs reject extra arguments. The underlying MCP SDK ignores
unparsable JSON lines and continues reading; malformed JSON does not receive a
JSON-RPC error response, but later valid requests still work.
