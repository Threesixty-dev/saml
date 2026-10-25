# Architecture modeling examples

Worked examples of the five approaches discussed for comparison with SAML.
The focus is their metamodels, constraints, and workflows for answering an
architecture question. Each directory contains a native model and a walkthrough.
These are small examples written for this repository, not vendor examples.

## Shared problem

A cold store has a temperature probe, a threshold evaluator, a local audible
alarm, and a cloud telemetry history service. A sample flows from the probe to
the evaluator and then to the alarm. Telemetry is also published to the cloud.

The architecture must address two requirements:

- **R1 - Local operation:** the alarm path must work while the WAN is unavailable.
- **R2 - Response time:** from sample acquisition to an audible alarm takes at
  most 100 ms.

The proposed change is to move threshold evaluation to the cloud. Does the model
help identify the consequences, and what evidence supports accepting or rejecting
the change?

Assume telemetry publication does not block local alarming. That assumption must
be checked against an implementation; drawing a separate connection does not
establish it. An edge location alone also does not establish availability: power,
local links, runtime services, and other dependencies can still fail.

For the timing examples, the assumed upper bounds are 10 ms for the probe,
20 ms for evaluation, 10 ms for sounding the alarm, and 2 ms for each of two local
links. Their sum is 44 ms. These are illustrative inputs, not measurements.

## Examples

| Approach | Native model | Workflow demonstrated | Important boundary |
| --- | --- | --- | --- |
| [Structurizr/C4](structurizr/README.md) | People, systems, containers, relationships, properties | Review responsibility and dependency placement | DSL validation does not enforce the example's locality or timing requirements |
| [LikeC4](likec4/README.md) | Custom element and relationship kinds, metadata | Validate a model, then apply a custom graph rule | Custom kinds do not automatically acquire domain semantics |
| [Acme/Armani](acme/README.md) | Families, component types, ports, connectors, roles, properties, invariants | Instantiate an architecture style and check design rules | The timing formula and locality rule are authored explicitly for this example |
| [AADL](aadl/README.md) | Component categories, directed typed ports, connections, flows, timing properties | Instantiate a system and analyze an end-to-end latency budget | Results depend on supplied bounds and the detail of the execution model |
| [ArchiMate](archimate/README.md) | Goals, requirements, constraints, business/application/technology concepts, typed relationships | Trace a proposed change to requirements and operational responsibilities | A realization relationship records a claim; it does not prove fulfillment |

The common scenario makes the questions comparable, but the models have different
scopes. For example, the AADL file models the technical flow, while the ArchiMate
file also models the operator's response and the business goal. This is not a
feature ranking or evidence of adoption.

## How to inspect them

1. Read each directory's metamodel mapping and native source.
2. Follow its application workflow and cloud-change exercise.
3. Separate what the tool checks from assumptions and human review.
4. Evaluate whether the modeled information helps make the decision.

Each walkthrough includes primary sources and a validation status. Tool versions
and checks are recorded in [VALIDATION.md](VALIDATION.md).

For the evolving SAML article-publication sketch, see the [parallel native examples](article-publication/README.md) in ArchiMate, EMF/Epsilon, Refinery, Acme/Armani, AADL, LikeC4, and Structurizr.
