# Distributed SAML runtime

Status: `saml-agent` manages multiple repositories, serves the local portal
and LLM API, and sends repository-detail heartbeats to the self-hosted
`saml-hub`. The hub serves a fleet dashboard and signed agent releases.
`saml-launcher` verifies and installs agent updates.

The hub is self-hosted; agents install approved updates automatically. Its
telemetry now includes bounded repository details alongside health and version.

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
and a canonical root path in the agent's local configuration. The configured
source and model paths are included in the authenticated hub snapshot.
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
Each repository also reports its name and repository-relative model paths, model inventory and diagnostics
summary, Git working-tree changes, branches, and recent commit subjects.
It has a schema version so agent and hub releases can evolve independently.
The hub records last-seen state and the latest 100 samples per agent in an
atomically replaced JSON file, exposed by an authenticated API. A central
dashboard shows recent presence, versions, repository details, and health history.
Each repository opens on its own page with a searchable semantic relationship
graph. Files in an external model workspace are labeled and shown relative to
that model root.
The launcher also reports verified release attempts to the hub. OpenTelemetry
export remains future work.

Inventory and Git lists are bounded, with full counts and truncation flags.
When a fleet exceeds the heartbeat size limit, repository health is retained
and some detail snapshots are omitted. Health history stores only IDs and
health states, keeping it compact. The agent uploads a separate graph report
after each successful heartbeat, capped at 500 items and 2,000 resolved links.
The heartbeat contains metadata and diagnostic messages but no model source text, LLM
prompts, credentials, or Git remote URLs. The agent reports its telemetry level
at `/api/status`. Local checking and MCP use remain available without the hub.
An unhealthy repository does not make unrelated repositories unavailable.

## Run the connected slice

Install the binaries with `cargo install --path .`. On each agent machine,
create a local registry and add repositories:

```sh
saml-agent init --config ./agent.json
saml-agent add --config ./agent.json --repo /path/to/repo-a
saml-agent add --config ./agent.json --repo /path/to/repo-b
saml-agent run --config ./agent.json
```

Models may also be stored outside their source repository. Add
`--model-root /path/to/model` when registering a repository; the agent keeps
the canonical source path and stable ID, while the portal and health check
analyze the model root. A repository with no model sources is degraded.

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
for remote agents. A container must set `SAML_HUB_ALLOW_NON_LOOPBACK=1` to
bind to its private container interface; never publish its raw HTTP port to
the internet. On a fresh data directory, the hub creates an empty credentials
file with owner-only permissions. The server
reloads the credentials file for each heartbeat, so provisioning another
agent does not require a restart. Open the hub's root page to use the fleet
dashboard. The default `SAML_HUB_AUTH_MODE=token` asks for
`SAML_HUB_ADMIN_TOKEN` and keeps it in browser memory for that tab.

### Coolify deployment

`Dockerfile.hub` builds only the hub. In Coolify, deploy it from a Git
repository using that Dockerfile, or publish the image to a registry and use
the Docker Image resource. On ThreeSixty, configure the Coolify domain as
`http://saml.coolify.threesixty.dev` to route to container port `8765`.
Cloudflare forces public HTTP requests to HTTPS; enabling Coolify's HTTPS
redirect on this internal route creates a redirect loop. Do not configure a
public host port. Mount persistent storage at `/data` and set
`SAML_HUB_AUTH_MODE=external` when Cloudflare Access protects the entire
hostname. The dashboard then loads without an application admin token. Keep
the Coolify origin reachable only through trusted private networking and the
authenticated Cloudflare route. Other deployments can keep the default token
mode and set `SAML_HUB_ADMIN_TOKEN` to at least 32 random characters.
The image already sets `SAML_HUB_BIND`, `SAML_HUB_ALLOW_NON_LOOPBACK`,
`SAML_HUB_STATE`, and `SAML_HUB_CREDENTIALS`. It runs as UID 10001, so a bind
mount must be writable by that UID. Set the HTTP health check path to
`/healthz` (the image also includes a Docker health check).

If the hostname is behind Cloudflare Access, give SAML its own self-hosted
Access application for `saml.coolify.threesixty.dev`. Keep a human login
policy for the dashboard and a Service Auth policy for agents. Each agent
then needs a Cloudflare Access service token as well as its own SAML agent
token. Set `SAML_CF_ACCESS_CLIENT_ID` and `SAML_CF_ACCESS_CLIENT_SECRET` in
the agent's environment; the agent sends them as `CF-Access-Client-Id` and
`CF-Access-Client-Secret` headers on heartbeats. Both variables must be set
together. Keep them out of `agent.json` and the repository. In external auth
mode, Cloudflare Access also protects the fleet API; the hub continues to
require each agent's separate `SAML_AGENT_TOKEN` for heartbeats.

From the deployed container's Coolify terminal, provision each agent's ID:

```sh
saml-hub provision AGENT_ID /data/credentials.json
```

Keep the printed token private and set it as `SAML_AGENT_TOKEN` on that agent
machine. Connect the agent with `saml-agent set-hub --config ./agent.json
--hub https://saml.coolify.threesixty.dev`. Check `/healthz`, verify
Cloudflare Access denies an unauthenticated `/api/v1/agents` request, then
send a heartbeat with `saml-agent run --config ./agent.json --once` and confirm that the dashboard
shows the agent and its repositories.

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

The hub exposes `GET /healthz`, `GET /api/v1/config`, and
`GET /api/v1/agents`. The config endpoint reports `token` or `external` auth
mode. In token mode, the fleet endpoint requires
`Authorization: Bearer <SAML_HUB_ADMIN_TOKEN>`; in external mode, the proxy
controls access. The fleet endpoint returns the last heartbeat and bounded
history per agent. The agent sends to `POST /api/v1/heartbeat` and then
`POST /api/v1/repository-graphs` using its own provisioned credential in either
mode. `GET /api/v1/repository-graphs/{agent_id}/{repository_id}` uses the same
dashboard authorization as `GET /api/v1/agents`. Graphs are persisted with hub
state and removed when the agent unregisters a repository. The local analyzer, LLM commands, and
portal continue to work if the hub is unreachable.

## Signed agent updates

The hub serves a read-only update repository from `SAML_HUB_UPDATE_DIR`, with
`metadata/release.json`, `metadata/release.sig`, and versioned binaries under
`targets/saml-agent/<os>/<arch>/<version>`. The Coolify image copies the
repository's `updates/` directory into `/opt/saml/updates`. Cloudflare Access
protects the transport; the launcher independently authenticates the exact
manifest bytes using the pinned Ed25519 public key in `updates/root.pub`.

The publisher is `scripts/publish-agent-release.py`. It accepts one or more
`--target OS/ARCH=BINARY` arguments, a stable `--version`, and an Ed25519
private key. It rejects duplicate targets and binaries whose executable format
or CPU architecture does not match the declared platform. Keep the signing key
outside Git with owner-only permissions.

The `Agent build` GitHub Actions workflow builds and tests the agent and launcher
on macOS Apple Silicon, macOS Intel, Linux ARM64, Linux x86-64, and Windows
x86-64. Linux builds run inside Debian Bookworm so they work on that runtime
baseline; newer Ubuntu-hosted glibc is not allowed to raise the requirement.
Each artifact contains both executables and names its source commit.
The workflow has read-only repository access and never receives the signing
key. Download artifacts from the same passing commit, then sign the desired
platform binaries on the trusted signing machine. For example:

```sh
python3 scripts/publish-agent-release.py --version 0.3.7 \
  --key /private/path/update-signing-key.pem --output updates \
  --target macos/aarch64=/path/to/macos-aarch64/saml-agent \
  --target linux/aarch64=/path/to/linux-aarch64/saml-agent
```

Add the other platform artifacts to the same command when they are ready to
ship. Publish the resulting `updates/` files and deploy the hub. Do not publish
a manifest until its referenced targets are present in the hub image. The launcher reads
the hub URL from the local agent config and Cloudflare service token from its
environment. Run it as the OS-supervised process, with paths to the initial
agent binary and a private state directory:

```sh
saml-launcher --config ./agent.json --agent ./saml-agent \
  --state-dir ./agent-state
```

The launcher checks every 60 seconds by default (`SAML_UPDATE_POLL_SECS`,
minimum 5). It verifies the signature, metadata expiry, increasing semantic
version, platform-specific target path, size, and SHA-256 before staging the
binary. It stops the old child, starts the new one, and confirms `/api/status`
reports the expected version. If the new process fails startup, it restores
the previous binary. State and previous binaries remain local. The launcher
itself is a stable bootstrap executable; deploying it initially is a manual
step. On Windows, it stores versioned agents with the `.exe` suffix. Platform
binaries must be published for each target platform.

The launcher keeps a private `update-events.json` beside `active.json` and
sends update outcomes to `POST /api/v1/update-events` with the existing agent
credential and Cloudflare service token. It retries undelivered events after
network failures. The hub deduplicates event IDs and retains the latest 100
events per agent. The dashboard shows the attempted version, installed or
failed result, time, and a fixed reason code translated to a short description.
No file paths, network error bodies, credentials, or model data are sent.
Unsigned or unreachable manifest checks remain in local launcher logs because
there is no authenticated target version to report. Updating an existing
launcher to gain this reporting requires a manual launcher replacement.

To smoke-test a Linux ARM64 update against the deployed hub, provide a Linux
bootstrap agent and launcher from the previous version, the new version, the
hub URL, and any Cloudflare Access service token in the environment:

```sh
scripts/test-linux-update.sh /path/to/old/saml-agent \
  /path/to/old/saml-launcher 0.3.7 https://saml.coolify.threesixty.dev
```

The script starts an isolated Linux container, waits for the signed update,
checks the new agent's local portal, then removes the test container. It sends
no fleet heartbeat and requires no test agent credential.

This is a single-key signed manifest, not a complete TUF repository. Key
rotation, delegated roles, and staged rollouts remain future work. Keep the
private key backed up securely; losing it means
existing launchers cannot trust later releases without a manual trust-root
replacement. The hub only transports release bytes and never holds the key.

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
4. Done for the initial channel: the hub serves signed release metadata and
   targets; the launcher verifies and stages updates with startup rollback.
   Full TUF metadata, key rotation, and staged deployment remain planned.
5. Done: add a central dashboard for health, versions, and repository details. Deployment channels
   remain planned. Expand telemetry only after its data policy is specified.

## References

- [The Update Framework overview](https://theupdateframework.io/docs/overview/)
- [The Update Framework security model](https://theupdateframework.io/docs/security/)
- [OpenTelemetry signals](https://opentelemetry.io/docs/concepts/signals/)
