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

Only `boundary`, `concept`, and `rule` are built-in keywords. A `.spec` file can
introduce vocabulary with declarations such as `concept Entity {}` and
`boundary Scope {}`. These become `entity` and `scope` in model files. The
former `boundry` spelling produces a rename diagnostic in spec files.
Vocabulary is workspace-wide: every `.spec` in the open workspace contributes
words to all model files in that workspace, regardless of directory. For the
existing example, `spec.saml` is also read as a manifest. VS Code gives built-in
keywords, spec-defined vocabulary, and property names distinct semantic token categories.
The static grammar colors comments, strings, names, and punctuation; it does not
maintain a list of user-defined words. The `samlVocabulary` token falls back to
the theme's `support.function` color, keeping it visually separate from model
names that use `entity.name.type`. `Public` and `Private` are highlighted as
keywords when used as access modifiers before a materialization kind.

`concept Entity` and `boundary Entity` give the same `entity` word two compatible
roles: concept and boundary. Hover shows both roles, and go to definition
includes both declarations.

Property names in a concept have their own identity. Hovering `environment` in
`concept ApplicationType` describes that property; go to definition on an
`environment:` assignment returns the property declaration. Hovering or
navigating from its `type: Environment` value still resolves the `Environment`
concept. Typed properties are required on materializations, directly or through
resolved `extends` bases.

`type: [Data]` in a property or materialization input schema requires a list
of `Data` references. A reference with a block, such as
`capability: RestApi { expose: [Article, Author] }`, binds inputs declared on
the `RestApi` materialization for that use. The server checks argument names,
value shapes, and resolved target kinds. The semantic map records bindings and
their target links.

Model materializations in a boundary are private by default. Put `Public`
immediately before the materialization kind to allow references from other
boundaries, as in `scope Identity Public entity User {}`. `Private` explicitly
states the default. An inaccessible typed reference produces an
`inaccessible-reference` diagnostic. The semantic map records visibility on
each occurrence and marks cross-boundary links.

The server supports full-document synchronization, diagnostics, document
symbols, go to definition, hover, completion, Find All References, and
workspace-wide Rename Symbol. Press Shift+F12 to find uses of `Interface` from
either its concept declaration or an `interface` model declaration. Press F2 to
rename both forms; typed property references and boundary qualifiers update too.

Materialization names are separate symbols. Go to Definition, hover, references,
and rename follow the resolved materialization identity, including typed property
values, binding arguments, `extends`, and explicit boundary qualifiers. Repeated
declarations of one identity are included together; same-named materializations
in other scopes are excluded. An unresolved, ambiguous, or inaccessible name
does not navigate to an arbitrary same-named declaration. Rename uses open
unsaved buffers and rejects invalid names, workspace collisions of the same
kind, and names with unresolved or untyped uses. Implicit boundary
materializations and `state` materializations cannot yet be renamed safely.
Property names remain separate symbols and do not support rename yet.

The reusable Rust semantic analyzer is in `src/semantic.rs`. It composes spec
roles, maps workspace materializations, resolves typed references and `extends`
links, and returns semantic findings to the LSP. Run
`cargo run --bin saml-map -- project/example-1` to inspect its JSON output.
The custom LSP request `saml/semanticMap` returns the same map for open buffers.
Named model elements appear under its `materializations` key.
See `SEMANTIC_MODEL.md` for the current model and resolution rules.

Diagnostics cover unmatched `{}`, `[]`, `()`, unterminated quoted strings, and
the semantic findings described above. Other constructs whose semantics are not
yet specified are accepted. Declaration recognition uses the workspace
vocabulary, including `state: Name` lines when `State` is introduced by a spec.
A chain such as `layer Infrastructure scope Article {}` is represented as a `scope Article`
symbol when `Scope` is defined. The preceding `layer` is a qualifier.

## Rules still needed

The example does not determine a complete grammar or type system. In particular:

- When model declarations repeat a name and role, are they extensions of one
  concept, overloads, or errors? `ArticlePublication` and `Article` recur in
  the example.
- Should boundary kinds beyond `scope` and `layer` participate in lexical lookup
  differently from their current containment behavior?
- Which attributes and relations are legal for each declaration, and which are
  mandatory? The example uses both `from: Initial` and `from Initial`.
- Are semicolons optional everywhere, and are multi-line declarations always
  allowed?
- Are names case-sensitive? `UserPortalFrontEnd` in `infrastructure.saml` differs
  from `UserPortalFrontend` in `applications.saml`.

The semantic analyzer checks references whose target kind is declared in a
spec property rule or an `extends` clause. It does not yet validate unrestricted
prose relations or duplicate model declarations. Renaming stops if an untracked
identifier with the same spelling could be left behind.

Run the tests with `cargo test`.
