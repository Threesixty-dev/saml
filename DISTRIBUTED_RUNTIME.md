# Distributed SAML runtime

Status: `saml-agent` manages multiple repositories, serves the local portal
and LLM API, and sends health/version heartbeats to the self-hosted
`saml-hub`. The hub serves a fleet dashboard. Signed automatic updates remain
planned.

Initial decisions: the hub is self-hosted; agents install approved updates
automatically; the first telemetry scope is health and version information.

## Components

```text
LLM host -- file/command or local HTTP --> SAML agent
                                           | repository registry
                                           | analyzer + local portal
                                           | independent health checks
                                           +-- outbound HTTPS --> self-hosted SAML hub
                                                                  | agent registry
                                                                  | bounded history
                                                                  | authenticated API
                                                                  + fleet dashboard
```

The agent is a portable Rust executable. It owns repository registration,
semantic health checks, the local portal, and the LLM-facing API. The
`saml-view` executable remains available for ad hoc use. One agent manages many
repositories at once. Each repository has an independent set of `.spec` and
`.saml` files, vocabulary, semantic map, and diagnostics; references never
resolve across repository boundaries. The
local portal lets a person select a repository. The hub is a separate
deployment. It observes running agents. The local analyzer remains usable
without hub access.

Repository registration uses a persistent opaque ID per local checkout
and a canonical root path in the agent's local configuration. Paths stay local.
Adding or removing a repository should not require another agent installation.
The agent portal uses persistent repository IDs and reloads its registry on
every API request. The running agent also reloads it before each heartbeat.
Adding or removing a repository does not require an agent restart. The
standalone `saml-view` still accepts multiple roots at startup and uses
session-local numeric selectors.

## Connection and observability

An agent receives a server-provisioned credential and initiates outbound
HTTPS requests. Loopback HTTP is allowed for local development. The protocol
uses periodic heartbeats; no inbound connection to the user's machine is
required. A heartbeat includes a stable agent ID, app version, OS,
architecture, agent health, and one opaque ID and health state per repository.
It has a schema version so agent and hub releases can evolve independently.
The hub records last-seen state and the latest 100 samples per agent in an
atomically replaced JSON file, exposed by an authenticated API. A central
dashboard shows recent presence, versions, repository health, and history.
Update outcomes and OpenTelemetry export remain future work.

The heartbeat reports an opaque ID and health state for each registered
repository without sending its path or model data. An unhealthy repository
must not make unrelated repositories unavailable. The hub can distinguish
agent health from per-repository analysis health.

The initial heartbeat excludes model source, semantic maps, diagnostics, file
paths, and LLM prompts. Sending any of these later requires an explicit data
policy. The agent should make its current telemetry level visible in the local
portal. The hub should not be a required dependency for local checking or LLM
use.

## Run the connected slice

Install the binaries with `cargo install --path .`. On each agent machine,
create a local registry and add repositories:

```sh
saml-agent init --config ./agent.json
saml-agent add --config ./agent.json --repo /path/to/repo-a
saml-agent add --config ./agent.json --repo /path/to/repo-b
saml-agent run --config ./agent.json
```

The agent portal listens on `127.0.0.1:8787` by default. Set
`SAML_AGENT_PORT` to another port. The portal and local LLM endpoints remain
available without a hub or token.

`init` prints the agent ID. To connect to a hub, configure its HTTPS URL on
the agent:

```sh
saml-agent set-hub --config ./agent.json --hub https://saml.example.org
```

On the server, provision that ID and keep the
printed token private:

```sh
saml-hub provision AGENT_ID /var/lib/saml/credentials.json
export SAML_HUB_CREDENTIALS=/var/lib/saml/credentials.json
export SAML_HUB_STATE=/var/lib/saml/state.json
export SAML_HUB_ADMIN_TOKEN='<at-least-32-random-characters>'
saml-hub
```

The hub listens on `127.0.0.1:8765` by default. Set `SAML_HUB_BIND` to
another loopback address or port. Put an HTTPS reverse proxy in front of it
for remote agents; the hub refuses to bind to a public interface. The server
reloads the credentials file for each heartbeat, so provisioning another
agent does not require a restart. Open the hub's root page to use the fleet
dashboard; it asks for `SAML_HUB_ADMIN_TOKEN` and keeps it in browser memory
for that tab.

On the agent machine, set `SAML_AGENT_TOKEN` to the provisioned token and
start or restart the agent:

```sh
saml-agent run --config ./agent.json
```

With a token, the agent sends a heartbeat every 30 seconds and reloads its repository
config before each send. Set `SAML_HEARTBEAT_SECS` to change the interval
(minimum 5 seconds), or use `--once` for a single send. Use `saml-agent
remove --config ./agent.json --id REPOSITORY_ID` to unregister a repository.
Each `add` prints a stable repository ID; adding the same canonical root
again returns the existing ID.

The hub exposes `GET /healthz` and `GET /api/v1/agents`. The latter requires
`Authorization: Bearer <SAML_HUB_ADMIN_TOKEN>` and returns the last heartbeat
and bounded history per agent. The agent sends to `POST /api/v1/heartbeat`
using its own provisioned credential. The local analyzer, LLM commands, and
portal continue to work if the hub is unreachable.

## Updates

The hub can serve a read-only update repository when `SAML_HUB_UPDATE_DIR`
points at a directory with `metadata/` and `targets/` children. They are
available at `/updates/metadata/` and `/updates/targets/`. The operator
publishes signed TUF metadata and artifacts into that directory; the hub is
only a transport and does not sign or authorize executable code. Path traversal
and symlink escapes are rejected. No agent update client or installer is
enabled yet.

The signed repository will contain releases by channel and platform. Agents
will check and install eligible releases automatically. Update metadata and
artifacts must be authenticated independently of the HTTPS connection; a
compromised hub should not be able to distribute arbitrary executable code.
The installer embeds a trusted update root. The client verifies release
signatures, artifact hash, platform, version, and metadata freshness before
staging an update. Installation needs a separate launcher/updater process so
the running executable can be replaced on every
supported platform. After restart, the launcher checks health and can restore
the previous version on failure. Rollouts should support channels and staged
deployment, with a way to hold or roll back a release.

Release metadata will follow The Update Framework for signature, freshness,
and rollback checks. Transport and installation remain separate: the hub
serves repository files, while a local launcher will perform platform-specific
replacement after verification.

The release publisher signs artifacts and metadata before the hub receives
them. Automatic installation never treats an unsigned hub response as
authorization to execute code. A local rollback to the last known good slot is
separate from accepting an old release as a new update.

## LLM and portal interface

An LLM host can read SAML source files and use `saml-check` and `saml-map`, or
call the agent portal's `/api/guide`, `/api/workspaces`, `/api/diagnostics`, and
`/api/map`. The caller selects a persistent repository ID for each map or
diagnostics request. The same local portal provides the visual view. Its
`/api/status` reports service and version information.
The central hub observes agents; it does not need access to the local portal.

## Implementation sequence

1. Done: keep the analyzer, CLI, and local portal as the offline core. The portal
   accepts multiple roots, isolates their analysis, and provides
   machine-readable service health and version at `/api/status`.
2. Done: add an outbound agent-to-hub heartbeat with provisioning and a versioned
   schema. Build the hub's registry and health history.
3. Done: have the agent host the existing portal and LLM API from its
   persistent repository registry. Packaging it as an OS service remains
   planned.
4. In progress: the hub serves update metadata and targets. Add signed release
   publication, client-side TUF verification, staging, and a platform-specific
   launcher with rollback.
5. Done: add a central dashboard for health and versions. Deployment channels
   remain planned. Expand telemetry only after its data policy is specified.

## References

- [The Update Framework overview](https://theupdateframework.io/docs/overview/)
- [The Update Framework security model](https://theupdateframework.io/docs/security/)
- [OpenTelemetry signals](https://opentelemetry.io/docs/concepts/signals/)
