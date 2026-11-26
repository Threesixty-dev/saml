# Article publication in Lean

Read [ArticlePublication.lean](ArticlePublication.lean). The file supplies a small
domain metamodel, architecture instances, an executable structural check, and
a transition system with a safety proof. All of these are written in Lean;
SAML syntax is not needed to express this example.

## Metamodel and relations

Separate ID types distinguish actors, articles, data objects, capabilities, and
applications. A `Consumption` must refer to applications and a capability;
`DataObject.realizes` must refer to an article. Each architecture supplies total
lookup functions, so every ID in these finite domains resolves. The domains are
fixed for this example rather than loaded from an extensible manifest.

`Conforms` requires:

- The API consumer runs in a browser.
- The provider offers the capability used by the consumer.
- The data exposed through that capability realizes the workflow's article.
- Publication follows evaluation with a successful outcome.
- The queue producer is the API provider and its consumer stage is evaluation.

`check_correct` proves that `check model = true` is equivalent to those
constraints. The checker uses `decide` on the proposition itself, keeping one
definition of structural validity. `VerifiedArchitecture` pairs a model with
evidence of conformance; `verifiedBaseline` constructs one.

The baseline names the frontend/backend roles, an Articles API, a RabbitMQ queue,
and an author submitting an article with 150 letters. REST transport and RabbitMQ
execution are not modeled: their names identify architectural intent.

## Behavior and proof

The example assumes **at least 100 letters**, matching the explicit policy in
the Epsilon and Refinery examples. The SAML sketch does not supply that threshold.

`Step` defines the allowed transitions:

```text
Submitted -- at least 100 letters --> Successful -- configured gate --> Published
Submitted -- fewer than 100 letters --> Failed
```

`Reachable` starts at `Submitted` and permits only those steps. The proof proceeds
by induction over any reachable execution, using `StateInvariant` and
`step_preserves_invariant`. The resulting `publication_requires_eligible` theorem
states that a reachable `Published` state implies the article has at least 100
letters, for every architecture described by these definitions.
`publication_requires_successful_evaluation` separately proves that successful
evaluation was reachable before publication.

This goes beyond checking the baseline trace. `baseline_can_publish` also proves
that the model has a successful execution, avoiding a safety result that holds
only because publication is unreachable.
`threshold_article_can_publish` checks the inclusive boundary of exactly 100
letters as well.

The article is immutable during an execution. This is a single-article workflow;
there is no concurrency, retry, persistence, authentication, or queue-delivery
semantics. Extending the transition system requires revisiting its proof.

## Invalid alternatives

Three structurally invalid models have proofs that their checks return `false`:

| Alternative | Violated constraint |
| --- | --- |
| `missingProvider` | The selected audit capability is not offered by the backend |
| `unrelatedData` | Exposed data realizes a different article from the workflow subject |
| `ungatedPublication` | Publication requires failure rather than success |

`shortArticle` has 99 letters. Its architecture still conforms, but
`short_article_can_fail` proves a rejection execution exists and
`short_article_cannot_publish` proves publication is unreachable. Architectural
validity and acceptance of submitted content are separate predicates.

## Run and experiment

With Lean installed, run these commands from the repository root:

```sh
cd competitors
lake build ArticlePublication
```

The shared project pins Lean 4.35.0, uses only its bundled standard library, and
treats warnings as errors. `lake build` checks both Lean comparison examples.
An editor with Lean support can show each theorem's goal and proof state.

To observe proof failures, remove `.articlesApi` from the baseline backend's
`provides` list, or add a direct `Submitted -> Published` constructor to `Step`.
The former breaks `baseline_valid`; the latter leaves the invariant-preservation
proof without a valid case. The checked-in alternatives instead prove rejection
and therefore do not deliberately break the build.

**Validation status:** compiled with Lean 4.35.0 on 2026-10-29, including the
negative alternatives and the general reachability proof. The
[shared validation record](../../VALIDATION.md) records mutation checks as well.

## Relationship to SAML

This demonstrates the Lean-only approach: definitions, instances, and proofs in
one language. SAML could later provide an authoring syntax that translates to
such definitions. This example does not formalize SAML's scope resolution,
fragment composition, inheritance, or configurable metamodels, and it proves
nothing about the current Rust parser or generated application code.

## Primary sources

- [Theorem Proving in Lean: structures](https://lean-lang.org/theorem_proving_in_lean4/Structures-and-Records/)
- [Theorem Proving in Lean: inductive propositions](https://lean-lang.org/theorem_proving_in_lean4/Inductive-Types/)
- [Lean reference: validating proofs](https://lean-lang.org/doc/reference/latest/ValidatingProofs/)
- [Setup and toolchain](../../lean/README.md#application-workflow)
