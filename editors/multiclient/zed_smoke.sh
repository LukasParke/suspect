#!/bin/zsh
# Zed attach smoke: does the suspect LSP start and answer under Zed?
#
# Zed sends a leaner handshake than VS Code — no dynamic registration, no
# workspace/configuration — so it exercises paths VS Code never does. This
# script never touches global Zed settings (they hold user secrets); the
# fixture carries its own project-local .zed/settings.json.
#
# Requires the dev extension in editors/zed to be installed (it registers the
# `suspect-lsp` language server); the fixture points that server at the
# binary below via `lsp.suspect-lsp.binary.path`.
#
#    SUSPECT_BIN=/path/to/suspect ./editors/multiclient/zed_smoke.sh
#
# SUSPECT_BIN defaults to <repo>/target/release/suspect.
#
#    ZED_SMOKE_MANAGED=1 ./editors/multiclient/zed_smoke.sh
#
# The no-PATH run: no `binary.path`, and `suspect` must not be on PATH, so
# the extension can only start the release build it downloads and checks
# (editors/zed/src/download.rs) into its work directory. This is a manual
# check: it needs the GitHub release v<version of editors/zed/extension.toml>
# to exist, and is skipped (exit 0) while it does not.
#
# Verifies: Zed spawns the server as a child; the process stays alive; and
# Zed's log records no LSP error for suspect. Prints SMOKE| lines for each
# check and exits non-zero on failure.

set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
MANAGED="${ZED_SMOKE_MANAGED:-}"
SUSPECT="${SUSPECT_BIN:-$ROOT/target/release/suspect}"
FIXTURE=$(mktemp -d /tmp/suspect-zed-smoke.XXXXXX)
ZED_LOG="$HOME/Library/Application Support/Zed/log/Zed.log"

if ! command -v zed >/dev/null 2>&1; then
  echo "SMOKE|zed not on PATH — skipped"
  exit 0
fi
if [ -n "$MANAGED" ]; then
  VERSION=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$ROOT/editors/zed/extension.toml" | head -1)
  ASSET="suspect-cli-$(uname -m | sed 's/arm64/aarch64/')-apple-darwin.tar.gz"
  URL="https://github.com/LukasParke/suspect/releases/download/v$VERSION/$ASSET.sha256"
  if ! curl -fsIL "$URL" >/dev/null 2>&1; then
    echo "SMOKE|SKIP — managed mode needs release v$VERSION ($URL); manual check until it is published"
    exit 0
  fi
  if command -v suspect >/dev/null 2>&1; then
    echo "SMOKE|FAIL — suspect is on PATH ($(command -v suspect)); managed mode must not find it there"
    exit 1
  fi
  # What the extension unpacks into its work directory.
  SUSPECT="$HOME/Library/Application Support/Zed/extensions/work/suspect/suspect-$VERSION/suspect"
  echo "SMOKE|managed mode: expecting $SUSPECT"
elif [ ! -x "$SUSPECT" ]; then
  echo "SMOKE|no suspect binary at $SUSPECT — cargo build --release -p suspect-cli or set SUSPECT_BIN"
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
if [ -n "$MANAGED" ]; then
  LSP='{}'
else
  LSP="{ \"binary\": { \"path\": \"$SUSPECT\", \"arguments\": [\"lsp\"] } }"
fi
cat > "$FIXTURE/.zed/settings.json" <<EOF
{
  "lsp": {
    "suspect-lsp": $LSP
  },
  "languages": {
    "YAML": {
      "language_servers": ["suspect-lsp", "..."]
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
