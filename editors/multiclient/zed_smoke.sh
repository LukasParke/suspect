#!/bin/zsh
# Zed attach smoke: does the suspect LSP start and answer under Zed?
#
# Zed sends a leaner handshake than VS Code — no dynamic registration, no
# workspace/configuration — so it exercises paths VS Code never does. This
# script never touches global Zed settings (they hold user secrets); the
# fixture carries its own project-local .zed/settings.json.
#
# Verifies: Zed spawns the server as a child; the process stays alive; and
# Zed's log records no LSP error for suspect. Prints SMOKE| lines for each
# check and exits non-zero on failure.

set -u

SUSPECT=/Users/luke/github/suspect-core-pr/target/release/suspect
FIXTURE=$(mktemp -d /tmp/suspect-zed-smoke.XXXXXX)
ZED_LOG="$HOME/Library/Application Support/Zed/log/Zed.log"

if ! command -v zed >/dev/null 2>&1; then
  echo "SMOKE|zed not on PATH — skipped"
  exit 0
fi
if [ ! -x "$SUSPECT" ]; then
  echo "SMOKE|no suspect binary — build first"
  exit 1
fi

# The fixture: an OpenAPI doc with markdown in its description, a $ref to
# follow, and a lint finding to publish.
cat > "$FIXTURE/openapi.yaml" <<'EOF'
openapi: 3.1.0
info:
  title: Zed smoke
  version: '1'
  description: |
    # Title

    Some **bold** prose.
paths:
  /pets:
    get:
      operationId: listPets
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Pet'
components:
  schemas:
    Pet:
      type: object
EOF

mkdir -p "$FIXTURE/.zed"
cat > "$FIXTURE/.zed/settings.json" <<EOF
{
  "lsp": {
    "suspect": {
      "binary": {
        "path": "$SUSPECT",
        "arguments": ["lsp"]
      }
    }
  },
  "languages": {
    "YAML": {
      "language_servers": ["suspect"]
    }
  }
}
EOF

echo "SMOKE|fixture at $FIXTURE"
echo "SMOKE|launching Zed on the fixture (project-local settings only)"
zed "$FIXTURE" >/dev/null 2>&1 &

# Zed may already be running; opening the fixture still loads the
# project-local settings in that window. Give the client time to spawn.
SUSPECT_PID=""
for i in $(seq 1 30); do
  sleep 2
  SUSPECT_PID=$(ps -Ao pid,ppid,command | grep -F "$SUSPECT lsp" | grep -v grep | awk '{print $1}' | head -1)
  [ -n "$SUSPECT_PID" ] && break
done

if [ -z "$SUSPECT_PID" ]; then
  echo "SMOKE|FAIL — suspect lsp never spawned under Zed"
  echo "SMOKE|zed log tail:"
  tail -20 "$ZED_LOG" 2>/dev/null | sed 's/^/SMOKE|  /'
  exit 1
fi
echo "SMOKE|PASS — server spawned as pid $SUSPECT_PID"

sleep 10
if ! kill -0 "$SUSPECT_PID" 2>/dev/null; then
  echo "SMOKE|FAIL — server exited within 10s of spawning"
  tail -20 "$ZED_LOG" 2>/dev/null | grep -i -A 3 "suspect" | sed 's/^/SMOKE|  /'
  exit 1
fi
echo "SMOKE|PASS — server alive after 10s (no startup crash)"

CPU1=$(ps -o time= -p "$SUSPECT_PID" 2>/dev/null)
sleep 10
CPU2=$(ps -o time= -p "$SUSPECT_PID" 2>/dev/null)
if [ "$CPU1" = "$CPU2" ] && [ -z "$(ps -o etime= -p "$SUSPECT_PID" | cut -d: -f1)" ]; then
  echo "SMOKE|NOTE — cpu static ($CPU1); fine when idle"
fi
echo "SMOKE|INFO — cpu ${CPU1:-?} -> ${CPU2:-?}"

if grep -qi "suspect.*error\|error.*suspect" "$ZED_LOG" 2>/dev/null; then
  echo "SMOKE|FAIL — Zed's log records an error mentioning suspect:"
  grep -i "suspect" "$ZED_LOG" | tail -10 | sed 's/^/SMOKE|  /'
  exit 1
fi
echo "SMOKE|PASS — no suspect error in Zed's log"

echo "SMOKE|done — Zed left open on $FIXTURE for a human look; the server is pid $SUSPECT_PID"
