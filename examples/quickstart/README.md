# Quickstart model

This example is a small, valid SAML workspace. `spec.saml` defines the `scope`,
`data`, and `service` words and requires every service to provide a list of
data it reads. `architecture.saml` puts data and a service in separate scopes;
the data is `Public` so the service can reference it.

From the SAML repository root, run:

```sh
cargo run --locked --bin saml-check -- examples/quickstart
cargo run --locked --bin saml-map -- examples/quickstart
```

The first command should report two files, zero errors, and zero warnings.
Read the [language guide](../../LANGUAGE_SPECIFICATION.md) for an explanation
of the syntax and instructions for using SAML in another repository.
