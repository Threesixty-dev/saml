#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 4 ]]; then
  echo "usage: $0 BOOTSTRAP_AGENT BOOTSTRAP_LAUNCHER EXPECTED_VERSION HUB_URL" >&2
  exit 2
fi

bootstrap_agent=$(realpath "$1")
bootstrap_launcher=$(realpath "$2")
expected_version=$3
hub_url=$4
test_dir=$(mktemp -d "${TMPDIR:-/tmp}/saml-linux-update.XXXXXX")
container="saml-update-smoke-$$"

cleanup() {
  docker rm -f "$container" >/dev/null 2>&1 || true
  rm -rf "$test_dir"
}
trap cleanup EXIT

cp "$bootstrap_agent" "$test_dir/saml-agent-bootstrap"
cp "$bootstrap_launcher" "$test_dir/saml-launcher"
chmod 700 "$test_dir/saml-agent-bootstrap" "$test_dir/saml-launcher"
HUB_URL="$hub_url" python3 - "$test_dir/agent.json" <<'PY'
import json
import os
import sys

with open(sys.argv[1], "w", encoding="utf-8") as config:
    json.dump({
        "schema_version": 1,
        "agent_id": "00000000-0000-4000-8000-000000000001",
        "hub_url": os.environ["HUB_URL"],
        "repositories": [],
    }, config)
PY

docker run -d --name "$container" --user "$(id -u):$(id -g)" \
  -e SAML_UPDATE_POLL_SECS=5 \
  -e SAML_CF_ACCESS_CLIENT_ID -e SAML_CF_ACCESS_CLIENT_SECRET \
  -v "$test_dir:/work" -w /work rust:1.99-bookworm \
  /work/saml-launcher --config /work/agent.json \
  --agent /work/saml-agent-bootstrap --state-dir /work/state >/dev/null

for _ in {1..30}; do
  if [[ -f "$test_dir/state/active.json" ]] && \
    python3 - "$test_dir/state/active.json" "$expected_version" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as state:
    sys.exit(json.load(state)["version"] != sys.argv[2])
PY
  then
    status=$(docker exec "$container" curl --fail --silent \
      http://127.0.0.1:8787/api/status)
    STATUS="$status" EXPECTED="$expected_version" python3 - <<'PY'
import json
import os

status = json.loads(os.environ["STATUS"])
assert status["version"] == os.environ["EXPECTED"], status
assert status["service"] == "saml-agent", status
print(f"Linux agent updated to {status['version']} and passed its portal health check")
PY
    docker logs --tail 10 "$container"
    exit 0
  fi
  if [[ $(docker inspect -f '{{.State.Running}}' "$container") != true ]]; then
    docker logs "$container" >&2
    echo "Linux launcher exited before updating" >&2
    exit 1
  fi
  sleep 5
done

docker logs --tail 30 "$container" >&2
echo "Timed out waiting for Linux agent $expected_version" >&2
exit 1
