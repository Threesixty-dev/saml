# Semantic model prototype

The Rust library in `src/semantic.rs` is independent of the editor protocol.
It consumes all `.spec` and `.saml` sources in a workspace and produces a
semantic map. `src/main.rs` exposes findings as LSP diagnostics; the standalone
`saml-map` binary prints the same map as JSON:

```sh
cargo run --bin saml-map -- project/example-1
```

## Map

- **Concepts** come from spec declarations. `meta Entity` supplies the `meta`
  role and `boundry Entity` supplies the `boundry` role to one `entity` word.
- **Objects** are named uses of vocabulary words, such as `entity User`, or
  currently undeclared words found in model files. Their identity includes
  kind, name, and lexical scope. Repeated declarations of that identity
  contribute additional occurrences, each with its own source location and
  boundary memberships.
- **Scopes** follow containing declarations and boundary qualifiers. A `scope`
  or `layer` qualifier can introduce an implicit boundary object; `layer`
  membership is recorded without adding a lexical scope level.
- **Links** record boundary membership, typed fields, and `extends` separately.
  Reference links outside the source object's scope or descendants are marked
  `crosses_scope`; boundary membership links are not crossings.
- **Findings** have source spans, codes, and error or warning severity.

For unqualified references, the analyzer prefers the nearest containing scope.
If there is no local or ancestor match, it accepts one unique workspace match
and records whether the resolved link crosses scopes. Multiple workspace
matches are ambiguous. This is the current prototype resolution rule; access
policy between boundaries is still unspecified.

A field rule such as `meta Action { initiator { type: Entity } }` checks that an
`initiator: Name` value on an `action` object resolves to an `entity` object.
`extends Base` resolves to an object of the same kind and creates a separate
`extends` link. The analyzer checks unknown vocabulary, unresolved or ambiguous
references, wrong target kinds, and conflicting spec field rules. Unknown
vocabulary is a warning while the manifest is being completed.

The current example has eight unknown-vocabulary warnings because its manifest
does not yet declare `interface`, `technology`, `environment`,
`applicationType`, or `applicationCapability`. The analyzer does not yet parse
all prose relations or enforce Public/Private boundary access.
