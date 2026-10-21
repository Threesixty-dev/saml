# Structurizr DSL / C4

## Model

Read [workspace.dsl](workspace.dsl). C4 supplies the abstraction levels;
Structurizr DSL expresses the model and its views.

| Problem concept | Model concept |
| --- | --- |
| Operator | Person |
| Monitoring capability | Software system |
| Probe, evaluator, alarm firmware, history service | Containers: separate executing applications/firmware in this example |
| Sample, trigger, telemetry | Directed relationships |
| Edge/cloud placement and requirement text | Custom properties |
| Critical alarm path | Custom relationship tag |

Here `site` describes the intended deployment. It is an annotation, not a native
deployment model. A fuller example could add deployment nodes and container
instances. The container view selects information from the model.

## Application workflow

1. Identify the responsibilities that execute independently.
2. Record the sample/trigger path and the separate telemetry relationship.
3. Review which relationships cross the edge/cloud boundary.
4. Validate the DSL to catch model and reference errors.
5. Record a decision about R1 and obtain timing evidence for R2 elsewhere.

With the standalone Structurizr CLI, from the repository root:

```sh
structurizr.sh validate -workspace competitors/structurizr/workspace.dsl
```

This command was checked with the pinned CLI version in [../VALIDATION.md](../VALIDATION.md).

## Proposed cloud change

Set the evaluator's `site` to `cloud` and describe the probe/evaluator and
evaluator/alarm relationships as WAN links. The DSL can still be valid. A reviewer
can identify the newly remote critical path, but rejecting it for R1 requires a
review rule or additional tooling. R2 is only requirement text in this model.

## What this establishes

The model makes responsibilities and dependency placement inspectable. The example
does not establish nonblocking telemetry, outage behavior, or a response bound.
Those are limits of this model and workflow; they do not imply that the wider
Structurizr ecosystem cannot be extended.

## Primary sources

- [C4 abstraction levels](https://c4model.com/abstractions)
- [Structurizr DSL language reference](https://docs.structurizr.com/dsl/language)
- [Structurizr CLI validation](https://docs.structurizr.com/cli/validate)
