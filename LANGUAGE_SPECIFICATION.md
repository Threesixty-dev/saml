

Purpose:

SAML is an architectural DLS focused on expressing solutions high level concepts in form of detailed specifications, which can be then interpreted by any LLM for further implementation process. It ensures semantic correctness across boundries, coupling scattered meaning under particular symbol.


Core components:
- SAML syntax
- spec manifest
- interpreter
- compiler
- middleware
-




1. SAML syntax

Restricted keywords:

boundry
concept
rule



2. SPEC manifest

Spec declarations together define the metamodel.

`concept` introduces a named kind and can define its properties. `boundry` gives a
kind boundary behavior and can introduce the kind on its own. These declarations
compose by name: `concept Entity` and `boundry Entity` describe one `entity`
vocabulary word with both roles.

A block directly inside a concept declares a property. For example, in
`concept ApplicationType { environment { type: Environment } }`, `environment`
is the property name and `Environment` is the required value's concept. A
materialization of `ApplicationType` must provide `environment` with an
`Environment` materialization. A resolved `extends` base may provide it instead.
Property names belong to their concept; they do not introduce new vocabulary.

Using a vocabulary word with a name materializes that concept in the model.
For example, `entity User` is a materialization of `concept Entity`. `extends`
describes a separate relationship between materializations.
