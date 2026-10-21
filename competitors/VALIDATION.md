# Validation record

Checked on 2026-10-21. Tool downloads, dependencies, mutated alternatives, and
generated analysis reports were kept outside the repository in temporary
directories. The supplied model files describe the baseline architecture.

## Structurizr

- Tool: standalone Structurizr CLI 2025.11.09, structurizr-java 5.0.2, Java 17.
- Ran `validate -workspace competitors/structurizr/workspace.dsl`.
- Result: success, exit code 0.
- Scope: native DSL/model validation. No custom locality or timing check was run.

## LikeC4

- Tool: LikeC4 1.60.0, Node.js 24.21.0.
- Ran `likec4 validate --no-layout competitors/likec4`.
- Result: valid, one source file, exit code 0.
- Ran `check.mjs` against the baseline: the locality rule passed, exit code 0.
- In a temporary copy, changed only the evaluator's `site` from `edge` to `cloud`.
  The API accepted the native model; the custom check exited 1 with
  `R1: evaluator is outside the edge site`.
- Scope: an example rule over the modeled signal graph. Timing is not checked.

## Acme / Armani

- Reviewed the model against Acme resources and CMU's published Acme models,
  including typed instantiation and named invariant syntax.
- No Acme parser, typechecker, or Armani evaluator was run.
- The documented rule outcomes are predictions from the expressions, not
  independently verified AcmeStudio output.

## AADL

- Tool: OSATE CLI 0.3.0 for macOS ARM64, with its bundled Java 21 runtime.
- Ran native validation and instantiated `Cold_Storage::Monitor.local`.
- Ran `analyze-latency` on the resulting system instance.
- Baseline: reported actual latency 22..44 ms; reported maximum 44 ms <= 100 ms.
- Temporary alternative: changed evaluator `Latency` to `120 ms .. 140 ms`,
  then validated, instantiated, and analyzed again. The alternative remained a
  valid AADL model but reported actual latency 132..164 ms and deadline errors:

```text
Maximum actual latency total 164.0ms exceeds expected maximum end to end latency 100.0ms
```

The tool used its default asynchronous-system latency analysis settings. The
reported intervals derive from the supplied flow/connection budgets; they are
not measured execution times or a proof of schedulability on a target platform.

## ArchiMate

- Tool: `xmllint`, with The Open Group's official `archimate3_Model.xsd` and its
  W3C `xml.xsd` dependency resolved locally.
- Ran XML Schema validation of `competitors/archimate/model.xml`.
- Result: validates, exit code 0.
- Scope: exchange-file schema validation. A desktop import and a complete
  ArchiMate relationship-conformance check were not run. Requirement fulfillment
  and timing were not analyzed.
