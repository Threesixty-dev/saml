

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
meta
rule



2. SPEC manifest

Spec serves as pieces declaration

`meta` describes a thing. `boundry` marks a boundary. These roles compose for
the same name: `meta Entity` and `boundry Entity` describe one `entity`
vocabulary word that is both a thing and a boundary.

Using a vocabulary word with a name creates an object of that kind, for
example `entity User` creates a `User` object from `meta Entity`. `extends`
describes a separate relationship between objects.
