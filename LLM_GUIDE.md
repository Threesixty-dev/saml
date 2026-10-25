# SAML guide for LLM agents

SAML models software architecture. The `.spec` manifest defines a workspace's
vocabulary; `.saml` files use that vocabulary to describe a model. Treat the
saved files and the semantic analyzer as the source of truth. Do not infer that
a model is valid just because its prose sounds plausible.

The tools can be installed with `cargo install --path /path/to/saml` from a
checkout of the SAML repository. A host LLM needs file and command access, or
local HTTP access to a running `saml-view` instance, to use these interfaces.

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
   `/api/guide`, `/api/diagnostics`, `/api/map`, `/api/workspaces`, and
   `/api/status` from the same address. List IDs with `/api/workspaces` and
   pass `?workspace=<id>` to `/api/map` or `/api/diagnostics`. The agent's
   repository IDs remain stable as other repositories are added or removed.

The portal reads saved files on each API request. Save changes before fetching
its data. The current analyzer checks specified structural and typed-reference
rules; it does not prove that an implementation fulfills the architecture or
validate every unrestricted prose relation.
