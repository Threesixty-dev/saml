

Purpose:

SAML is an architectural DLS focused on expressing solutions high level concepts in form of detailed specifications, which can be then interpreted by any LLM for further implementation process. It ensures semantic correctness across boundaries, coupling scattered meaning under particular symbol.


Core components:
- SAML syntax
- spec manifest
- interpreter
- compiler
- middleware
-




1. SAML syntax

Restricted keywords:

boundary
concept
rule



2. SPEC manifest

Spec declarations together define the metamodel.

`concept` introduces a named kind and can define its properties. `boundary` gives a
kind boundary behavior and can introduce the kind on its own. These declarations
compose by name: `concept Entity` and `boundary Entity` describe one `entity`
vocabulary word with both roles.

A block directly inside a concept declares a property. For example, in
`concept ApplicationType { environment { type: Environment } }`, `environment`
is the property name and `Environment` is the required value's concept. A
materialization of `ApplicationType` must provide `environment` with an
`Environment` materialization. A resolved `extends` base may provide it instead.
`type: [Data]` declares a list of `Data` references instead of one reference.
Property names belong to their concept; they do not introduce new vocabulary.

Using a vocabulary word with a name materializes that concept in the model.
For example, `entity User` is a materialization of `concept Entity`. `extends`
describes a separate relationship between materializations.

A materialization may declare required inputs. In
`applicationCapability RestApi { expose: { type: [Data] } }`, `expose` is an input
whose value must be a list of `Data` materializations. A reference with a block,
such as `capability: RestApi { expose: [Article, Author] }`, binds those inputs
for this use of `RestApi`. The binding does not change `RestApi` or create a
second capability materialization. Every declared input must be supplied, and
binding arguments must match their declared names, value shapes, and kinds.
