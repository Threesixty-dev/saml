# Distributed SAML runtime

Status: design target. The current Rust binaries run locally; they do not yet
connect to a central service or install updates.

Initial decisions: the hub is self-hosted; agents install approved updates
automatically; the first telemetry scope is health and version information.

## Components

```text
LLM host -- file/command or local HTTP --> SAML agent
                                           | analyzer + semantic map
                                           | local portal (127.0.0.1)
                                           | update client
                                           +-- outbound HTTPS --> self-hosted SAML hub
                                                                  | agent registry
                                                                  | health history
                                                                  | release metadata
                                                                  + observability UI
```

The installed SAML agent should work on macOS, Linux, and Windows. It owns
repository registration, semantic analysis, the local portal, and the
LLM-facing API. One agent manages many repositories at once. Each repository
has an independent set of `.spec` and `.saml` files, vocabulary, semantic map,
and diagnostics; references never resolve across repository boundaries. The
local portal lets a person select a repository. The hub is a separate
deployment. It observes running agents and serves release information. The
local analyzer remains usable without hub access.

Repository registration should use a persistent opaque ID per local checkout
and a canonical root path in the agent's local configuration. Paths stay local.
Adding or removing a repository should not require another agent installation.
The current portal accepts multiple roots at startup and uses session-local
numeric selectors; the future agent needs persistent IDs and dynamic registry
updates.

## Connection and observability

An agent enrolls with a server-issued credential and initiates an outbound
HTTPS connection. The first protocol can be periodic heartbeats; no inbound
connection to the user's machine is required. A heartbeat should include a
stable agent ID, app version, OS and architecture, health state, and last
update outcome. It should have a schema version so agent and hub releases can
evolve independently. The hub records last-seen state and a bounded history,
and exposes those through a central dashboard. The hub can export its own
health metrics through OpenTelemetry.

The heartbeat can report an opaque ID and health state for each registered
repository without sending its path or model data. An unhealthy repository
must not make unrelated repositories unavailable. The hub can distinguish
agent health from per-repository analysis health.

The initial heartbeat excludes model source, semantic maps, diagnostics, file
paths, and LLM prompts. Sending any of these later requires an explicit data
policy. The agent should make its current telemetry level visible in the local
portal. The hub should not be a required dependency for local checking or LLM
use.

## Updates

The hub publishes releases by channel and platform. Agents check and install
eligible releases automatically. Update metadata and artifacts must be
authenticated independently of the HTTPS connection; a
compromised hub should not be able to distribute arbitrary executable code.
The installer embeds a trusted update root. The client verifies release
signatures, artifact hash, platform, version, and metadata freshness before
staging an update. Installation needs a separate launcher/updater process so
the running executable can be replaced on every
supported platform. After restart, the launcher checks health and can restore
the previous version on failure. Rollouts should support channels and staged
deployment, with a way to hold or roll back a release.

Release metadata can follow The Update Framework instead of inventing a
signature and rollback protocol. The transport and installation details remain
separate: the hub advertises a release, while the local launcher performs the
platform-specific replacement.

The release publisher signs artifacts and metadata before the hub receives
them. Automatic installation never treats an unsigned hub response as
authorization to execute code. A local rollback to the last known good slot is
separate from accepting an old release as a new update.

## LLM and portal interface

An LLM host can read SAML source files and use `saml-check` and `saml-map`, or
call the local portal's `/api/guide`, `/api/workspaces`, `/api/diagnostics`, and
`/api/map`. The caller selects one repository for each map or diagnostics
request. The local portal remains the visual view of a repository. A future
agent process can own these endpoints and extend `/api/status` with connection
and update state.
The central hub observes agents; it does not need access to the local portal.

## Implementation sequence

1. Keep the analyzer, CLI, and local portal as the offline core. The portal
   accepts multiple roots, isolates their analysis, and provides
   machine-readable service health and version at `/api/status`.
2. Add an outbound agent-to-hub heartbeat with enrollment and a versioned
   schema. Build the hub's registry and health history.
3. Add signed release publication, client-side verification, staging, and a
   platform-specific launcher with rollback.
4. Add deployment channels and a central dashboard. Expand telemetry only
   after its data policy is specified.

## References

- [The Update Framework overview](https://theupdateframework.io/docs/overview/)
- [The Update Framework security model](https://theupdateframework.io/docs/security/)
- [OpenTelemetry signals](https://opentelemetry.io/docs/concepts/signals/)
