# Article publication across modeling approaches

These native examples translate the intent of [SAML example 1](../../project/example-1/) into seven other approaches. They supplement the [cold-store comparison](../README.md); they do not claim that the languages have equivalent semantics.

## Shared case and assumptions

An author submits an article. Evaluation decides whether it can proceed to publication. A browser frontend consumes an Articles REST API provided by a backend. The backend exposes article data; that data should correspond to the domain article. A publication queue separates submission from evaluation.

The SAML sketch leaves `ArticleLettersNumberIsHigherThan` without a predicate. The Epsilon and Refinery examples use **at least 100 letters** so that their rules can be evaluated; this threshold is an example assumption, not a SAML requirement. The examples use the intended `UserPortalFrontend` spelling and treat RabbitMQ as the intended queue technology even though its declaration is currently absent from the SAML files. Authentication is outside the comparison because the SAML relationship does not yet specify its target or conditions.

## What each example shows

| Approach | Native files | Expressed in the example | What is actually checked |
| --- | --- | --- | --- |
| ArchiMate | [model.xml](archimate/model.xml) | Business processes and event, application service and consumer, business/data object realization, deployment artifacts | The exchange XML passes schema validation. The letter-count constraint and success condition are named claims, not executable tests. |
| EMF + Epsilon | [metamodel](emf-epsilon/article.emf), [instance](emf-epsilon/article.flexmi), [EVL rules](emf-epsilon/check.evl) | Typed metamodel, article and data counterpart, applications, capability consumption, workflow stages | EVL specifies checks for letter count, data realization, provider capability, and publication gating. The EVL model was not executed here. |
| Refinery | [model.refinery](refinery/model.refinery) | Classes and typed references, a partial article-publication model, derived eligibility and errors | The source defines an error for a short article and for consuming a capability the provider does not offer. A Refinery solver was not run here. |
| Acme/Armani | [model.acme](acme/model.acme) | Reusable application/queue types, REST and asynchronous connectors, ports, attachments, and invariants | Invariants specify browser placement and backend article-data provision. No Acme parser or Armani evaluator was run. |
| AADL | [article_publication.aadl](aadl/article_publication.aadl) | Frontend, backend, queue, evaluation, and publication as systems connected by event-data ports | The file describes technical paths, not REST semantics or the business rule. OSATE validation could not run in this sandbox. |
| LikeC4 | [model.c4](likec4/model.c4), [check.mjs](likec4/check.mjs) | Custom kinds for the domain, stages, applications, capability, queue, and typed relationships | Native validation passes. A separate JavaScript check verifies provider existence, stage ordering, and data realization in the modeled graph. |
| Structurizr/C4 | [workspace.dsl](structurizr/workspace.dsl) | Person, system, frontend/backend/queue/store containers, backend API and workflow components | Native DSL validation passes. The letter-count and success rules remain descriptions, not enforced predicates. |

The important comparison is **where a rule lives**. ArchiMate and Structurizr make relationships visible; Acme/Armani, EMF/Epsilon, and Refinery can express explicit constraints; AADL checks technical structure and can support specialized analyses. LikeC4's example check runs in JavaScript alongside the model. SAML's proposed distinction would be a stable core that enforces rules defined in a curated metamodel while preserving meaning across domain, application, and technical refinements. That behavior is not yet implemented in this repository.

## Validation record

Checked on 2026-10-22:

- ArchiMate: `xmllint` validated `model.xml` against The Open Group exchange schema using a local XML catalog for `xml.xsd`. This does not check ArchiMate relationship conformance or whether the requirements hold.
- Structurizr: the standalone CLI `validate -workspace` command succeeded.
- LikeC4: `likec4 validate --no-layout` succeeded, and `check.mjs` passed using LikeC4 1.60.0.
- EMF + Epsilon: the Flexmi file is well-formed XML; Emfatic compilation, model loading, and EVL execution were not run.
- Refinery and Acme/Armani: sources were written against their documented syntax, but were not parsed by their native tools.
- AADL: OSATE CLI initialization was blocked by the local sandbox's `launchd` bootstrap restriction, before parsing this file. Syntax and model checks remain unverified.

## Primary references

- [ArchiMate overview and modeling concepts](https://archimate-community.pages.opengroup.org/workgroups/archimate-101/)
- [EMFatic syntax](https://eclipse.dev/emfatic/), [Flexmi](https://eclipse.dev/epsilon/doc/flexmi/), and [Epsilon Validation Language](https://eclipse.dev/epsilon/doc/evl/)
- [Refinery classes](https://refinery.tools/learn/language/classes/) and [predicates and errors](https://refinery.tools/learn/language/predicates/)
- [CMU Acme resources](https://acme.able.cs.cmu.edu/)
- [SEI AADL overview](https://www.sei.cmu.edu/projects/architecture-analysis-and-design-language-aadl/)
- [LikeC4 specification](https://likec4.dev/dsl/specification/) and [validation guide](https://likec4.dev/guides/validate-your-model/)
- [Structurizr DSL reference](https://docs.structurizr.com/dsl/language) and [CLI validation](https://docs.structurizr.com/cli/validate)
