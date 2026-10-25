# Semantic model prototype

The Rust library in `src/semantic.rs` is independent of the editor protocol.
It consumes all `.spec` and `.saml` sources in a workspace and produces a
semantic map. `src/main.rs` exposes findings as LSP diagnostics; the standalone
`saml-map` binary prints the same map as JSON:

```sh
cargo run --bin saml-map -- project/example-1
```

For a machine-readable validation gate, run:

```sh
cargo run --bin saml-check -- project/example-1
```

It prints syntax and semantic diagnostics with file URIs and editor ranges,
and exits with status 1 when errors are present. The viewer serves the same
report at `/api/diagnostics` and the agent guide at `/api/guide`.

`saml-map` and `saml-check` accept multiple repository paths. Each path is
analyzed separately, and output is wrapped in a `workspaces` array with a
`root` and `map` or `report` for each repository. A single path keeps the
original output shape. The viewer can serve multiple roots in one process;
`/api/workspaces` lists their session-local IDs, and `?workspace=<id>` selects
the root for `/api/map` and `/api/diagnostics`.

To explore the same map as a 3D graph in a browser, run:

```sh
cargo run --bin saml-view -- project/example-1
```

Open `http://127.0.0.1:8787`. The viewer binds only to localhost. Its Refresh
button rereads saved `.spec` and `.saml` files, so save editor changes first.
Set `SAML_VIEW_PORT` to use another port. Drag to orbit, scroll to zoom, click a
node to inspect it, or search by name. Concept-to-materialization lines show
which spec word each materialization comes from. The JSON map retains the
original boundary, property, and `extends` links. Selecting a materialization
also shows its merged fields and the files that contribute them.

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
  materialization that may fill it. The map records the first declaration,
  every repeated declaration, every model assignment, and the value kind's
  source span. Property references and rename use the declaration and assignment
  spans, scoped to the owning concept.
- **Materializations** are named realizations of concepts in model files, such
  as `entity User`. The map also records provisional materializations for
  undeclared vocabulary so findings can point to them. Their identity includes
  kind, name, and lexical scope. Repeated declarations of that identity add
  occurrences with separate source locations and boundary memberships. A
  materialization can declare typed input parameters, including list inputs.
  Each occurrence records `public` or `private` visibility. The merged `fields`
  group assignment keys, and each field occurrence points to its declaration
  fragment. Repeated compatible input schemas retain all definition locations.
- **Scopes** follow containing declarations and boundary qualifiers. A `scope`
  or `layer` qualifier can introduce an implicit boundary materialization.
  `layer` membership is recorded without adding a lexical scope level.
- **Links** record boundary membership, typed properties, and `extends` separately.
  Binding arguments add links to the materializations supplied as input values.
  Reference links outside the source materialization's scope or descendants
  are marked `crosses_scope`. Links whose target lies outside the source's
  boundary memberships are marked `crosses_boundary`; boundary membership
  links are not crossings. Boundary qualifiers retain the exact source span of
  each name, including list qualifiers such as `layer [Business, Application]`.
- **Bindings** record a typed property reference with a local argument block,
  such as `capability: RestApi { expose: [Article, Author] }`. They retain the
  target materialization and the resolved argument values without modifying the
  target.
- **Findings** have source spans, codes, and error or warning severity.

For unqualified references, the analyzer considers accessible materializations
and prefers the nearest containing scope. If there is no local or ancestor
match, it accepts one unique accessible workspace match. Multiple accessible
matches are ambiguous. A materialization inside a boundary is private by
default. `Public` immediately before its kind exports it through all containing
boundaries, for example `layer Infrastructure Public data Article {}`. An
explicit `Private` has the same behavior as the default. A reference to a
matching materialization that exists only in another boundary produces an
`inaccessible-reference` finding. Declarations outside boundaries are visible
across the workspace. Structural boundary membership links do not require an
export. Fragments of one materialization must agree on visibility; a target
crossing a boundary is exported only when every fragment is `Public`.

The editor uses resolved link targets as symbol identities for materialization
navigation, reference search, and rename. It can distinguish equal names in
different boundaries and can include repeated declarations of one identity.
Unresolved references have no target to navigate to. Rename checks for other
untracked uses of the old and new spelling before returning edits.

A property rule such as `concept Action { initiator { type: Entity } }` checks that an
`initiator: Name` value on an `action` materialization resolves to an `entity`
materialization. `extends Base` resolves to a materialization of the same kind
and creates a separate `extends` link. Typed properties are required on each
materialization, either directly or through resolved `extends` bases. The
analyzer checks unknown vocabulary, unresolved or ambiguous references, wrong
value kinds, missing required properties, and conflicting spec property rules.
`type: [Entity]` accepts a list of `entity` references. Materialization input
schemas also check missing, unknown, or duplicate binding arguments and value
shape mismatches. Distinct declared properties on fragments of the same
materialization compose. A property assigned in multiple fragments produces a
`duplicate-fragment-property` error. Incompatible input types produce
`conflicting-parameter-rule`, and mixed visibility produces
`conflicting-fragment-visibility`. Untyped prose fields appear in the merged
field list but are not yet checked for duplicates.
Unknown vocabulary is a warning while the manifest is being completed.

The current example has six missing required property findings. For example,
`UserPortalFrontend` needs a `capability`, and several `workflowStage`
materializations need a `state`. The analyzer does not yet parse all prose
relations.
