# SAML

SAML stands for **Software Architecture Modeling Language**. Its goal is to reduce semantic ambiguity when modeling software systems. A stable semantic core will let users define metamodels tailored to a particular problem, then enforce their concepts and rules as a system model is built. The model is intended to express a program at a higher level of abstraction, with technical decisions refining it.

The project is in its initial stage. Language syntax, tools, and examples will be documented here as they are added.

An initial [language server](LANGUAGE_SERVER.md) supports editing the evolving `.saml` examples.

The [semantic map](SEMANTIC_MODEL.md) has a Rust analyzer, a JSON CLI, and a local 3D browser viewer.

The [distributed runtime](DISTRIBUTED_RUNTIME.md) includes a portable
`saml-agent` that hosts the local portal and LLM API for multiple registered
repositories. It can send health/version heartbeats to a self-hosted
`saml-hub` with a fleet dashboard; the local service also runs without a hub.
The runtime guide includes setup, credential provisioning, and the hub API.
Signed automatic updates remain planned.

For persistent multi-repository use:

```sh
saml-agent init --config ./agent.json
saml-agent add --config ./agent.json --repo /path/to/repo-a
saml-agent add --config ./agent.json --repo /path/to/repo-b
saml-agent run --config ./agent.json
```

For one-off viewing, `saml-view` can serve multiple repositories at once:

```sh
saml-view /path/to/repo-a /path/to/repo-b
```

Use `/api/workspaces` to list them; `/api/map` and `/api/diagnostics` accept
`?workspace=<id>`. The portal has a repository selector. Each repository is
analyzed independently.

To use SAML with a tool-enabled LLM in any project, see the [agent guide](LLM_GUIDE.md).
The `saml-check` command reports syntax and semantic diagnostics as JSON and
exits with status 1 on errors. The local viewer also serves an agent-facing
guide and diagnostics at `/api/guide` and `/api/diagnostics`.

Install the standalone tools with `cargo install --path .` from this checkout.
In another project, put vocabulary in a `.spec` file (or `spec.saml`) and models
in `.saml` files, then run `saml-check .` or `saml-view .` from that project.
`saml-check` and `saml-map` also accept multiple directories and return one
result per repository in a `workspaces` array; a single directory retains the
original JSON shape.
Give an LLM with file and command access the path to `LLM_GUIDE.md` and instruct
it to run `saml-check .` after edits. An LLM with local HTTP access can also
read the viewer's `/api/guide`, `/api/diagnostics`, and `/api/map` endpoints.

See [competitors](competitors/README.md) for worked examples of existing modeling approaches.
