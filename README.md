# SAML

SAML (Software Architecture Modeling Language) is an experimental language for
describing software architecture with a vocabulary defined by the model author.
A `.spec` file or `spec.saml` introduces concepts and boundaries; `.saml` files
use those words to describe a system. The current analyzer checks typed
references, required properties, scope, and visibility. It does not verify that
the implementation matches the model.

## Start with the language

- [Language guide](LANGUAGE_SPECIFICATION.md): write a vocabulary and model,
  understand the checked rules, and use SAML in another repository.
- [Working quickstart](examples/quickstart/): a two-file model with two
  boundaries and a typed list of references.
- [Semantic map](SEMANTIC_MODEL.md): details of name resolution, diagnostics,
  and the analyzer's JSON output.

To run the example from source, you need Rust and Cargo; project CI uses Rust
1.99.0. If needed, clone the public repository:

```sh
git clone https://github.com/Threesixty-dev/saml.git
cd saml
```

From the SAML repository root, validate the working example:

```sh
cargo run --locked --bin saml-check -- examples/quickstart
```

The expected result has two files, zero errors, and zero warnings. To inspect
the resolved architecture as JSON, run:

```sh
cargo run --locked --bin saml-map -- examples/quickstart
```

## Use SAML in another repository

Keep the vocabulary and model with the code they describe, for example in
`architecture/saml/spec.saml` and `architecture/saml/system.saml`. Download a
[language release](LANGUAGE_SERVER.md#get-and-install-the-server) for your
platform when one is available; it includes `saml-check`, `saml-map`, and
`saml-lsp`. Then run `saml-check architecture/saml` from the other repository.
Add that command to CI after the first model is valid. See the
[language guide](LANGUAGE_SPECIFICATION.md#using-saml-in-another-repository)
for the workflow and current validation limits.

## Other tooling

The [language server setup guide](LANGUAGE_SERVER.md#get-and-install-the-server)
shows where to obtain `saml-lsp`, how to install it, and how to connect the
included VS Code extension in another repository. The [agent guide](LLM_GUIDE.md)
explains how an LLM can read and check a model. The
[distributed runtime](DISTRIBUTED_RUNTIME.md) and
[MCP server](MCP_SERVER.md) are optional ways to browse and expose models from
multiple repositories; they are not required to write or validate SAML.

The larger [article-publication sketch](project/example-1/) explores language
ideas and currently has known diagnostics. Use the quickstart as a validated
starting point.
