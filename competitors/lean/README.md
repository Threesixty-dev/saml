# Lean 4

## Model

Read [ColdStorage.lean](ColdStorage.lean). This example defines the architecture,
its requirements, an executable check, and proofs in Lean itself. It uses only
the standard library bundled with the pinned Lean 4.35.0 toolchain.

| Problem concept | Lean representation |
| --- | --- |
| Probe, evaluator, alarm, history | A finite `Component` type |
| Component placement and timing | Total functions in `Architecture` |
| Local alarm path | `LocalAlarm`, a proposition about the three participating components |
| Sample-to-alarm timing | `alarmLatencyMs`, the sum of component and link bounds |
| R1 and R2 together | `Conforms` |
| Executable decision | `check`, with the theorem `check_correct` |

`check_correct` proves, for any architecture in this model, that the checker
returns `true` exactly when `Conforms` holds. Here the check directly uses Lean's
decision procedure for that proposition; this is not a separately implemented
graph algorithm or a proof about SAML's Rust analyzer.

The baseline has cloud history and an entirely local alarm path. Its assumed
latency is `10 + 2 + 20 + 2 + 10 = 44 ms`. The file proves both the arithmetic
and conformance.

## Application workflow

Install Lean using the [official setup instructions](https://lean-lang.org/install/).
From the repository root:

```sh
cd competitors
lake build
```

The shared Lake project builds both this example and the
[article-publication example](../article-publication/lean/README.md).
`lean-toolchain` pins the compiler; there are no external package dependencies.
To check only this module, run `lake build ColdStorage` from `competitors`.
Compiler warnings are errors, so unfinished proofs using `sorry` fail the build.

1. Define the architecture vocabulary and the intended constraints.
2. Supply a concrete architecture and bounds.
3. State and prove its conformance.
4. Change the architecture and rebuild; Lean must check the proof again.

## Proposed cloud change

The source keeps alternatives separate from the baseline:

- `cloudEvaluator` changes only evaluator placement. Its check returns `false`.
- `cloud_evaluator_violates_locality` proves that any cloud evaluator violates
  the modeled locality requirement, regardless of its timing bounds.
- `slowEvaluator` raises the evaluator bound to 140 ms. Its total is 164 ms and
  its check returns `false`.
- `slowHistory` increases telemetry history time to 1000 ms. The alarm
  requirements still hold under the stated nonblocking assumption.

These rejection results are themselves checked theorems, so the example builds
successfully while demonstrating invalid candidates. To see a failing proof,
change the baseline evaluator placement to `.cloud` and run `lake build`;
`baseline_valid` can no longer be proved by `decide`.

## What this establishes

Lean checks the authored formal requirements. The topology is fixed to three
alarm components and two links; changing that topology requires updating the
definitions. The example assumes all alarm dependencies are represented and
telemetry is nonblocking. Local placement does not prove outage availability,
resource isolation, or correctness of implementation. The timing inputs are
assumed upper bounds, not measurements or a scheduling analysis.

The definitions are an example domain library, not built-in architectural
semantics supplied by Lean. They illustrate a possible foundation for SAML;
there is no `.saml` translation or integration with the runtime here.

**Validation status:** compiled with Lean 4.35.0 on 2026-10-29. See the
[validation record](../VALIDATION.md) for positive and negative checks.

## Primary sources

- [Theorem Proving in Lean: propositions and proofs](https://lean-lang.org/theorem_proving_in_lean4/Propositions-and-Proofs/)
- [Theorem Proving in Lean: inductive types](https://lean-lang.org/theorem_proving_in_lean4/Inductive-Types/)
- [Lean reference: validating proofs](https://lean-lang.org/doc/reference/latest/ValidatingProofs/)
- [Lake build tool](https://lean-lang.org/doc/reference/latest/Build-Tools-and-Distribution/Lake/)
- [Pinned Lean release](https://github.com/leanprover/lean4/releases/tag/v4.35.0)
