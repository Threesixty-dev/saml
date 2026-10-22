# SAML language server (prototype)

Build and test from the repository root:

```sh
cargo build
cargo test
```

The server binary is `target/debug/saml-lsp`. It uses standard LSP messages on
stdin and stdout. The `vscode-saml` extension starts it automatically for
`*.saml` files when this repository is open in VS Code. Its TextMate grammar
colors comments, strings, declaration and control keywords, properties, named
types, and punctuation. Set `saml.serverPath` in VS Code if the binary lives
elsewhere.

The server supports full-document synchronization, diagnostics, document
symbols, go to definition, hover, and completion. Definitions are found across
all `.saml` files in the workspace, including open unsaved buffers. If a name
has several declarations, go to definition returns all of them.

The current diagnostics cover unmatched `{}`, `[]`, `()`, and unterminated quoted
strings. This intentionally accepts constructs whose semantics are not yet
specified. Declaration recognition follows the block forms in
`project/example-1`, including `state: Name` lines. A chain such as
`layer Infrastructure scope Article {}` is represented as a `scope Article`
symbol. The preceding `layer` is a qualifier.

## Rules still needed

The example does not determine a complete grammar or type system. In particular:

- Are declarations with the same name extensions of one concept, overloads, or
  errors? `ArticlePublication` and `Article` appear in several roles.
- How do `scope`, `layer`, and `application` affect visibility and reference
  lookup across files?
- Which attributes and relations are legal for each declaration, and which are
  mandatory? The example uses both `from: Initial` and `from Initial`.
- Are semicolons optional everywhere, and are multi-line declarations always
  allowed?
- Is `boundry` the intended keyword spelling? The example itself questions
  whether this should be a keyword at all.
- Are names case-sensitive? `UserPortalFrontEnd` in `infrastructure.saml` differs
  from `UserPortalFrontend` in `applications.saml`.

Until these are settled, the server avoids unresolved-name, duplicate-name, and
type errors. Navigation matches exact names and can therefore return several
possible definitions.

Run the tests with `cargo test`.
