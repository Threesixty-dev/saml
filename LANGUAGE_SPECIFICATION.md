# SAML language guide

SAML describes an architecture using words defined by that architecture's
model author. This guide covers the language and the checks implemented today.
It is a prototype: even a report with zero errors and warnings establishes
only the properties checked by the analyzer, not correctness of the application.

## Your first model

The [working quickstart](examples/quickstart/) has two files:

```text
examples/quickstart/
  spec.saml
  architecture.saml
```

`spec.saml` defines the vocabulary and one typed property:

```saml
boundary Scope {}

concept Data {}

concept Service {
  reads { type: [Data] }
}
```

`architecture.saml` uses the words introduced by the spec:

```saml
scope Content {
  Public data Article {}
  Public data Author {}
}

scope Delivery {
  service ArticlesApi { reads: [Article, Author] }
}
```

`Service` introduces the model word `service`; `Data` introduces `data`; and
`Scope` introduces `scope`. The `reads` property requires a list of references
to `data` materializations. `Article` and `Author` are `Public` so a service in
another scope can reference them.

With Rust and Cargo installed, check the example from the SAML repository root
(CI uses Rust 1.99.0):

```sh
cargo run --locked --bin saml-check -- examples/quickstart
```

The JSON report should contain `"files": 2`, `"errors": 0`, and
`"warnings": 0`. To inspect the resolved concepts, materializations, links,
bindings, and findings, run:

```sh
cargo run --locked --bin saml-map -- examples/quickstart
```

## Files and vocabulary

A model workspace is the directory passed to `saml-check` or `saml-map`. The
tools read `.spec` and `.saml` files recursively under that directory. Files
ending in `.spec`, and a `.saml` file named exactly `spec.saml`, contribute
vocabulary to the whole workspace. Other `.saml` files contain model
materializations. Separate workspace roots are analyzed independently; one
workspace cannot reference another's materializations.

The built-in declaration words are `concept`, `boundary`, and `rule`.
`concept Data {}` defines a kind of model element and introduces `data` as a
model word. `boundary Scope {}` introduces `scope` for grouping elements and
controlling visibility. The model word is formed by lowercasing the first
character of the declaration name; `ApplicationType` becomes
`applicationType`. A name may have both concept and boundary roles:
`concept Entity {}` and `boundary Entity {}` contribute to the same `entity`
word.

`rule` is recognized as a built-in word, but the current analyzer does not
execute general rule bodies. Put checks you rely on in supported typed
properties and reference relationships; keep other requirements explicit in
prose until they have defined semantics.

## Materializations and typed properties

A model declaration materializes a vocabulary word, for example
`data Article {}`. Its name identifies that element within its kind and
lexical scope. Properties written directly inside a `concept` define required
fields and their reference types. In `spec.saml`:

```saml
concept Data {}
concept Service {
  primary { type: Data }
  dependencies { type: [Data] }
}
```

A `service` must provide `primary` with one `data` reference and
`dependencies` with a list of `data` references. An empty list `[]` is valid;
omitting a required property is not. The analyzer reports unresolved or
ambiguous names, references to the wrong kind, and values with the wrong
single/list shape.

In a model `.saml` file:

```saml
data Article {}
data Author {}
service ArticlesApi {
  primary: Article
  dependencies: [Article, Author]
}
```

A model can also contain fields without a declared type. They are retained in
the semantic map, but their contents do not receive the typed-reference checks
shown above.

## Boundaries, visibility, and lookup

A boundary declaration in the spec enables a model grouping such as
`scope Content { ... }`. Elements inside boundaries are `Private` by default:
they can be referenced locally, but not from another boundary. Write `Public`
immediately before an element's vocabulary word to expose it:

```saml
scope Content {
  Private data Draft {}
  Public data Article {}
}

scope Delivery {
  service ArticlesApi {
    primary: Article
    dependencies: []
  }
}
```

This example assumes `boundary Scope {}` and the `Service` and `Data`
declarations above.
`ArticlesApi` can reference `Article` across boundaries; a reference to
`Draft` from `Delivery` would produce an `inaccessible-reference` finding.
Unqualified references prefer the nearest accessible scope. If no local or
ancestor match exists, one unique accessible workspace match can resolve; more
than one is ambiguous.

A boundary can also appear as a qualifier, for example
`scope Content Public data Article {}`. The larger example uses `layer` as
another spec-defined boundary word. Neither `scope` nor `layer` is built in:
declare each boundary kind your model uses.

## Inheritance and fragments

`extends` links a materialization to a base of the same kind. Required typed
properties can be supplied by a resolved base. In `spec.saml`:

```saml
concept Environment {}
concept ApplicationType {
  environment { type: Environment }
}
```

In a model `.saml` file:

```saml
environment Browser {}
applicationType Base { environment: Browser }
applicationType Derived extends Base {}
```

Definitions with the same kind, name, and lexical scope in different files are
fragments of one materialization. Distinct properties and compatible input
declarations compose. A typed property assigned in multiple fragments, mixed
`Public`/`Private` visibility, or conflicting input types produces a finding.
Fragments in different scopes remain separate elements.

## Inputs bound at a use site

A materialization can declare an input schema. A reference to it supplies
arguments for that particular use. In `spec.saml`:

```saml
concept Data {}
concept ApplicationCapability {}
concept Application {
  capability { type: ApplicationCapability }
}
```

In a model `.saml` file:

```saml
data Article {}
applicationCapability RestApi { expose: { type: [Data] } }
application Portal {
  capability: RestApi { expose: [Article] }
}
```

`RestApi` requires an `expose` list when referenced. The argument is checked
against the declared shape and `Data` kind; it does not modify the `RestApi`
materialization. The analyzer reports missing, extra, duplicate, or mistyped
binding arguments.

## Using SAML in another repository

1. Create an `architecture/saml/` directory in the repository that owns the
   architecture. Start with `spec.saml` and one small `.saml` model file.
2. Define only the concepts, boundaries, and typed properties needed to answer
   a concrete architecture question. Use the quickstart as a syntax example;
   adapt the vocabulary to your domain.
3. Install `saml-check` from a
   [language release](LANGUAGE_SERVER.md#get-and-install-the-server) when one is
   available. Otherwise, clone the public SAML repository and run
   `cargo install --locked --path /absolute/path/to/saml` from the checkout.
   Run `saml-check architecture/saml` from the target repository. Resolve errors
   and review warnings before relying on the model.
4. Commit the model beside the code. Add `saml-check architecture/saml` to CI,
   using a pinned SAML revision so local and CI diagnostics agree. Update the
   model when the architectural decision changes.
5. Use `saml-map architecture/saml` when you need the resolved JSON graph or
   want to understand why a reference did or did not resolve.

`saml-check` prints JSON with file, error, warning, and diagnostic counts. It
exits with status 1 on errors; warnings do not fail the command. With a single
root containing no `.spec` or `.saml` files, it returns an error instead of an
empty successful report.

## Current validation limits

The analyzer checks delimiter and string termination errors, declared
vocabulary, required typed properties, reference kinds and cardinality,
`extends`, visibility, fragments, and input bindings. Unknown vocabulary is a
warning while a manifest is being written. The grammar and type system remain
incomplete: unrestricted relations and prose fields are not fully checked,
repeated assignments within one fragment are not yet constrained, and general
`rule` bodies are not executed. A clean `saml-check` result does not establish
that source code, APIs, message delivery, or runtime behavior conform to the
model.

For more detail, see the [semantic model](SEMANTIC_MODEL.md). The
[language server setup guide](LANGUAGE_SERVER.md#get-and-install-the-server)
provides installation steps plus editor diagnostics, navigation, and rename
support. The older [article-publication sketch](project/example-1/)
is exploratory and has known diagnostics; the quickstart is the validated
example to copy first.
