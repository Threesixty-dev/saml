# Semantic model prototype

The Rust library in `src/semantic.rs` is independent of the editor protocol.
It consumes all `.spec` and `.saml` sources in a workspace and produces a
semantic map. `src/main.rs` exposes findings as LSP diagnostics; the standalone
`saml-map` binary prints the same map as JSON:

```sh
cargo run --bin saml-map -- project/example-1
```

To explore the same map as a 3D graph in a browser, run:

```sh
cargo run --bin saml-view -- project/example-1
```

Open `http://127.0.0.1:8787`. The viewer binds only to localhost. Its Refresh
button rereads saved `.spec` and `.saml` files, so save editor changes first.
Set `SAML_VIEW_PORT` to use another port. Drag to orbit, scroll to zoom, click a
node to inspect it, or search by name. Concept-to-materialization lines show
which spec word each materialization comes from. The JSON map retains the
original boundary, property, and `extends` links.

## Map

The JSON map exposes `concepts`, `materializations`, `links`, `bindings`, and
`findings`.

- **Concepts** come from spec declarations. `concept Entity` defines the kind
  and its properties; `boundary Entity` adds a boundary role to the same `entity`
  word. Each concept also records exact source locations where its vocabulary
  word is used, including model declarations, boundary qualifiers, and typed
  property references. The editor uses these locations for workspace renames
  and reference search.
- **Properties** belong to concepts. In `concept ApplicationType`, the name
  `environment` declares a property; `Environment` names the kind of
  materialization that may fill it. The map records both source spans.
- **Materializations** are named realizations of concepts in model files, such
  as `entity User`. The map also records provisional materializations for
  undeclared vocabulary so findings can point to them. Their identity includes
  kind, name, and lexical scope. Repeated declarations of that identity add
  occurrences with separate source locations and boundary memberships. A
  materialization can declare typed input parameters, including list inputs.
- **Scopes** follow containing declarations and boundary qualifiers. A `scope`
  or `layer` qualifier can introduce an implicit boundary materialization.
  `layer` membership is recorded without adding a lexical scope level.
- **Links** record boundary membership, typed properties, and `extends` separately.
  Binding arguments add links to the materializations supplied as input values.
  Reference links outside the source materialization's scope or descendants
  are marked `crosses_scope`; boundary membership links are not crossings.
- **Bindings** record a typed property reference with a local argument block,
  such as `capability: RestApi { expose: [Article, Author] }`. They retain the
  target materialization and the resolved argument values without modifying the
  target.
- **Findings** have source spans, codes, and error or warning severity.

For unqualified references, the analyzer prefers the nearest containing scope.
If there is no local or ancestor match, it accepts one unique workspace match
and records whether the resolved link crosses scopes. Multiple workspace
matches are ambiguous. This is the current prototype resolution rule; access
policy between boundaries is still unspecified.

A property rule such as `concept Action { initiator { type: Entity } }` checks that an
`initiator: Name` value on an `action` materialization resolves to an `entity`
materialization. `extends Base` resolves to a materialization of the same kind
and creates a separate `extends` link. Typed properties are required on each
materialization, either directly or through resolved `extends` bases. The
analyzer checks unknown vocabulary, unresolved or ambiguous references, wrong
value kinds, missing required properties, and conflicting spec property rules.
`type: [Entity]` accepts a list of `entity` references. Materialization input
schemas also check missing, unknown, or duplicate binding arguments and value
shape mismatches.
Unknown vocabulary is a warning while the manifest is being completed.

The current example has six missing required property findings. For example,
`UserPortalFrontend` needs a `capability`, and several `workflowStage`
materializations need a `state`. The analyzer does not yet parse all prose
relations or enforce Public/Private boundary access.
