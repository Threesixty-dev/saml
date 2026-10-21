# ArchiMate

## Model

Read [model.xml](model.xml). It uses The Open Group's ArchiMate 3 exchange format,
with a model and relationships but no diagram coordinates.

This slice groups evaluation and alarm triggering into the edge monitor. The
probe and physical alarm internals are outside its scope.

| Problem concept | Model concept |
| --- | --- |
| Protect stored stock | Goal |
| Offline alarming and 100 ms response | Requirements |
| Keep critical processing at the edge | Constraint |
| Operator and response procedure | Business role and business process |
| Edge monitor and cloud history | Application components |
| Evaluation behavior and exposed alarming capability | Application function and application service |
| Executable and gateway | Artifact and device |
| Responsibility, implementation, consumption, data transfer | Assignment, realization, serving, and flow relationships |

The distinction between a component, the function assigned to it, and the service
that function realizes is meaningful: it permits a deployment or responsibility
change to be discussed without treating all three as the same thing.

## Application workflow

1. Import `model.xml` into a tool supporting the ArchiMate exchange format, such
   as Archi's Open Exchange import.
2. Start at the stock-protection goal and inspect its requirements.
3. Follow realization relationships to the alarming service and function.
4. Inspect the assigned component and the artifact/device deployment chain.
5. Follow the service's consumers to identify operational impact.
6. Review a proposed change against the linked requirements and seek evidence
   for any fulfillment claims.

## Proposed cloud change

Reassign the evaluation function to a cloud component and revise its deployment
chain. Review the locality constraint, R1, R2, and the operator response process.
This reveals the concerns that need reevaluation. Simply retaining the service's
realization links to R1 and R2 does not establish that either requirement holds.

## What this establishes

The model supports traceability across motivation, business, application, and
technology concepts. A `Constraint` has a defined modeling role, but the text
inside it is not automatically an executable predicate. An XML schema check
validates interchange structure and references; it does not enforce all allowed
ArchiMate relationship combinations or prove requirement fulfillment.

## Primary sources

- [ArchiMate specification](https://pubs.opengroup.org/architecture/archimate32-doc/)
- [The Open Group exchange format](https://www.opengroup.org/open-group-archimate-model-exchange-file-format)
- [Official model schema](https://www.opengroup.org/xsd/archimate/3.0/archimate3_Model.xsd)
- [Archi user guide, including exchange import/export](https://www.archimatetool.com/downloads/archi/Archi%20User%20Guide.pdf)
