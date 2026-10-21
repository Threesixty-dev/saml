# LikeC4

## Model

Read [model.c4](model.c4). Its specification declares `probe`, `evaluator`,
`alarm`, and `archive` element kinds, plus `signal` and `telemetry` relationship
kinds. Instances carry `site` metadata. These names are a small vocabulary chosen
for this example; LikeC4 does not supply their cold-storage meaning.

[check.mjs](check.mjs) adds executable meaning to part of that vocabulary. Starting
at the alarm, it traverses incoming `signal` relationships and requires every
visited element to have `site = edge`. It also requires the probe and evaluator
to appear on that path.

## Application workflow

From this directory, with Node.js compatible with LikeC4 1.60.0:

```sh
npm install
npm run validate
npm run check
```

The first check validates the native language. The second runs the example's
custom architectural rule through LikeC4's model API. Its expected output is:

```text
R1 passes for the modeled signal path. R2 timing is not checked.
```

## Proposed cloud change

Change only the evaluator's `site` metadata from `edge` to `cloud`. Native model
validation still passes. `npm run check` rejects the change with:

```text
R1: evaluator is outside the edge site
```

This is the difference between supplying a vocabulary and supplying a check over
that vocabulary. The check lives in JavaScript alongside the native model.

## What this establishes

The check establishes locality for the explicitly modeled signal predecessors.
It assumes all dependencies needed for alarming are represented by those
relationships. It cannot detect a hidden cloud license check, blocking telemetry
publication, or a misleading `site` value. It does not analyze timing or certify
operation during an outage.

## Primary sources

- [Specification and custom kinds](https://likec4.dev/dsl/specification/)
- [Model syntax and metadata](https://likec4.dev/dsl/model/)
- [Custom validation workflow](https://likec4.dev/guides/validate-your-model/)
- [Official implementation and model API source](https://github.com/likec4/likec4/tree/main/packages/likec4)
