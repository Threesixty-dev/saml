# Validation record

Initial checks on 2026-10-21; Lean checks added on 2026-10-29.
Tool downloads, dependencies, mutated alternatives, and
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

## Lean 4

Checked on 2026-10-29:

- Tool: official Lean 4.35.0, macOS ARM64, commit
  `eb6cafb7290e8bd5f2937522d8d804f58f02bafc`. The temporary toolchain archive's
  SHA-256 matched the digest published in the official GitHub release metadata.
- Ran `lake build` from `competitors`, using the checked-in `lean-toolchain` and
  `lakefile.toml`. Both `ColdStorage` and `ArticlePublication` built successfully
  with compiler warnings treated as errors. There are no external Lean packages.
- Cold storage: proved baseline conformance and its 44 ms bound, rejection of a
  cloud evaluator, rejection of the 164 ms alternative, and conformance with slow
  telemetry under the modeled nonblocking assumption. A general theorem rejects
  any architecture whose evaluator is in the cloud.
- Article publication: proved baseline structural conformance, equivalence of
  the executable check to the declared constraints, and rejection of missing
  provider capability, unrelated data, and a failure-gated publication model.
- Proved by induction that every reachable published state satisfies the
  article-length policy, and separately that successful evaluation was reachable.
  Proved publication executions for 150 and exactly 100 letters. At 99 letters,
  the architecture remains structurally valid, a rejection execution exists,
  and publication is unreachable.

Temporary copies were compiled with `lean -DwarningAsError=true`:

| Mutation | Observed result |
| --- | --- |
| Place the baseline cold-store evaluator in the cloud | Compilation fails at `baseline_valid`; `decide` establishes that conformance is false |
| Remove the baseline backend's offered Articles API | Compilation fails at `baseline_valid`; `decide` establishes that conformance is false |
| Add `Step.bypass : Step m .submitted .published` | Compilation fails because the invariant-preservation and predecessor proofs lack a case for the new transition |

Used `#print axioms` for every named theorem and `verifiedBaseline`. None depends
on `sorryAx` or native evaluation's `Lean.ofReduceBool`. Structural proofs use
Lean's standard axioms (`propext`, and for the article checks also
`Classical.choice` and `Quot.sound`). The transition-invariant, reachability,
publication-safety, and explicit execution proofs have no axiom dependencies.

Scope: proofs about the definitions in these two Lean files. The cold-store
topology, timing inputs, and nonblocking telemetry are assumptions; the article
workflow is immutable and sequential. No SAML translation, Rust implementation,
REST transport, RabbitMQ behavior, or generated application was verified.
