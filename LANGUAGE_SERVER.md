# SAML language server (prototype)

Build and test from the repository root:

```sh
cargo build
cargo test
```

The server binary is `target/debug/saml-lsp`. It uses standard LSP messages on
stdin and stdout. The `vscode-saml` extension starts it automatically for
`*.saml` and `*.spec` files when this repository is open in VS Code. Set
`saml.serverPath` in VS Code if the binary lives elsewhere.

Only `boundry`, `meta`, and `rule` are built-in keywords. A `.spec` file can
introduce vocabulary with declarations such as `meta Entity {}` and
`boundry Scope {}`. These become `entity` and `scope` in model files. Vocabulary
is workspace-wide: every `.spec` in the open workspace contributes words to all
model files in that workspace, regardless of directory. For the existing example,
`spec.saml` is also read as a manifest. VS Code gives built-in keywords and
spec-defined vocabulary distinct semantic token categories. The static grammar
colors comments, strings, properties, names, and punctuation; it does not
maintain a list of user-defined words. The `samlVocabulary` token falls back to
the theme's `support.function` color, keeping it visually separate from model
names that use `entity.name.type`.

The server supports full-document synchronization, diagnostics, document
symbols, go to definition, hover, and completion. Definitions are found across
all `.saml` and `.spec` files in the workspace, including open unsaved buffers.
If a name has several declarations, go to definition returns all of them.

The current diagnostics cover unmatched `{}`, `[]`, `()`, and unterminated quoted
strings. This intentionally accepts constructs whose semantics are not yet
specified. Declaration recognition uses the workspace vocabulary, including
`state: Name` lines when `State` is introduced by a spec. A chain such as
`layer Infrastructure scope Article {}` is represented as a `scope Article`
symbol when `Scope` is defined. The preceding `layer` is a qualifier.

## Rules still needed

The example does not determine a complete grammar or type system. In particular:

- Are declarations with the same name extensions of one concept, overloads, or
  errors? `ArticlePublication` and `Article` appear in several roles.
- How do `scope`, `layer`, and `application` affect visibility and reference
  lookup within the workspace?
- Which attributes and relations are legal for each declaration, and which are
  mandatory? The example uses both `from: Initial` and `from Initial`.
- Are semicolons optional everywhere, and are multi-line declarations always
  allowed?
- Is `boundry` the final spelling of the restricted keyword?
- Are names case-sensitive? `UserPortalFrontEnd` in `infrastructure.saml` differs
  from `UserPortalFrontend` in `applications.saml`.

Until these are settled, the server avoids unresolved-name, duplicate-name, and
type errors. Navigation matches exact names and can therefore return several
possible definitions.

Run the tests with `cargo test`.
