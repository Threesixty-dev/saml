# Acme / Armani

## Model

Read [model.acme](model.acme). Acme supplies a structural vocabulary; the family
defines an architecture style and Armani rules constrain its instances.

| Problem concept | Model concept |
| --- | --- |
| Cold-storage architecture style | Family |
| A stage that must remain local | Component type `LocalStage` |
| Probe, evaluator, alarm | Component instances of that type |
| Stage inputs and outputs | Ports |
| Communication channel and its endpoints | Connector and roles |
| Wiring | Attachments between ports and roles |
| Placement and execution bounds | Typed properties |
| Locality and response budget | Named invariant rules |

`staysLocal` requires each `LocalStage` instance to have `site = "edge"`.
`latencyBudget` sums the three stage bounds and the assumed 4 ms link budget.
The baseline arithmetic is `10 + 20 + 10 + 4 = 44 <= 100`.

## Application workflow

1. Define the family, component/connector types, and design rules.
2. Instantiate the types and supply the required property values.
3. Connect the instances through ports and roles.
4. Open the model in an Acme/Armani-capable environment such as AcmeStudio and
   run its type/rule checks.
5. Use violations to reject or revise an architecture candidate.

## Proposed cloud change

Within the evaluator instance, change `Property site = "edge"` to
`Property site = "cloud"`. The `staysLocal` invariant should fail. Separately,
setting its `worstCaseMs` to 140 should make `latencyBudget` fail because the sum
becomes 164 ms.

## What this establishes

The rules express explicit predicates over a structural model. The timing rule
is a custom arithmetic budget, not a supplied scheduling or network analysis.
It hardcodes the participating stages and the link total; adding a stage without
updating the rule would make that budget incomplete. Locality depends on assigning
the intended components to `LocalStage` and modeling their relevant dependencies.

**Validation status:** syntax and idioms were reviewed against Acme documentation
and CMU's Acme models. An Acme parser/rule checker was not run, so the rule outcomes
above are expected outcomes, not recorded tool results.

## Primary sources

- [CMU Acme resources](https://acme.able.cs.cmu.edu/)
- [CMU Rainbow Acme model with typed instances and named invariants](https://github.com/cmu-able/rainbow/blob/master/targets/znews-ss/model/ZNewsSys.acme)
