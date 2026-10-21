# AADL

## Model

Read [cold_storage.aadl](cold_storage.aadl). The model uses devices for the probe
and alarm, systems for evaluation and history, directed event-data ports,
connections, flow specifications, and an end-to-end flow.

The `Monitor.local` implementation instantiates the architecture. Each source,
path, sink, and local connection contributes a latency interval. The end-to-end
flow carries the required upper bound of 100 ms.

The assumed component/link intervals imply a simple additive budget of 22..44 ms.
Event-data ports avoid introducing periodic sampling into this small example.
There are no threads, processors, dispatch protocols, bus bindings, or measured
execution times, so this is a flow-budget model rather than a schedulability study.

## Application workflow

1. Import the AADL file into an OSATE AADL project.
2. Validate the declarative model.
3. Instantiate `Cold_Storage::Monitor.local` to produce a system instance.
4. Run end-to-end flow latency analysis on that instance.
5. Compare the analysis with R2 and inspect diagnostics and assumptions.
6. Refine the model with execution/platform details before relying on a bound.

The included [.project](.project) allows importing this directory as an existing
OSATE project. With OSATE CLI 0.3.0 installed, the equivalent commands from this
directory are:

```sh
port=$(osate-cli cold-storage init "$PWD")
osate-cli cold-storage -p "$port" check "$PWD/cold_storage.aadl"
osate-cli cold-storage -p "$port" instantiate "$PWD/cold_storage.aadl" Cold_Storage::Monitor.local
osate-cli cold-storage -p "$port" analyze-latency "$PWD/instances/cold_storage_Monitor_local_Instance.aaxl2"
osate-cli cold-storage -p "$port" exit
```

The checked baseline reports 22..44 ms and satisfies the 100 ms upper budget.

## Proposed cloud change

As an initial timing exercise, change the evaluator's flow `Latency` from
`10 ms .. 20 ms` to `120 ms .. 140 ms`, instantiate again, and rerun the analysis.
The simple additive upper budget becomes 164 ms, exceeding R2. This change only
represents a larger latency budget; a cloud alternative also needs WAN components,
connections, and failure/dependency assumptions to address R1.

This alternative was also run: OSATE reports 132..164 ms and errors for exceeding
the 100 ms deadline. See [../VALIDATION.md](../VALIDATION.md).

Keeping the telemetry connection outside the end-to-end alarm flow makes the
intended flow explicit. It does not prove resource isolation or nonblocking code.

## Primary sources

- [SEI introduction to AADL](https://www.sei.cmu.edu/projects/architecture-analysis-and-design-language-aadl/)
- [OSATE tooling and CLI](https://github.com/osate/aadl-tooling)
