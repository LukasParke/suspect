#!/bin/zsh
# Neovim headless smoke runner. Skips cleanly when nvim is absent, so CI
# and developer machines without it stay green.
#
#    ./editors/multiclient/run_nvim.sh [fixture-dir]
#
# With no argument, a fresh fixture is generated (a small OpenAPI doc with
# markdown in its description, a $ref, and a lint finding).

set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
export SUSPECT_BENCH_EXE="$ROOT/target/release/suspect"

if ! command -v nvim >/dev/null 2>&1; then
  echo "SMOKE|nvim not on PATH — skipped (brew install neovim to run)"
  exit 0
fi
if [ ! -x "$SUSPECT_BENCH_EXE" ]; then
  echo "SMOKE|no suspect binary — cargo build --release -p suspect-cli first"
  exit 1
fi

FIXTURE="${1:-}"
if [ -z "$FIXTURE" ]; then
  FIXTURE=$(mktemp -d /tmp/suspect-nvim-smoke.XXXXXX)
  cat > "$FIXTURE/openapi.yaml" <<'EOF'
openapi: 3.1.0
info:
  title: Neovim smoke
  version: '1'
  description: |
    # Title

    Some **bold** prose and a bare https://example.com url.
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
fi

nvim --headless -l "$HERE/nvim_smoke.lua" "$FIXTURE"
