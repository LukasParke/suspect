#!/usr/bin/env bash
# Provisions the tool directories the opt-in native SDK suites expect
# (`cargo test -p suspect-codegen --test <backend>_* -- --ignored`).
#
# Usage: tools/native-setup.sh <backend>...   (typescript python go ruby php cpp rust | all)
#
# Toolchains themselves (node, python3.11/3.14, ruby, java, dotnet, ...) are
# found by `suspect_codegen::toolchain`; this script only installs the
# per-suite packages that live under the repository's `target/` directory.
# It is idempotent and is run by .github/workflows/native-evidence.yml.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tools="$root/crates/suspect-codegen/tools"

typescript() {
  # Pinned TypeScript compilers, bundlers and doc tools, one package-lock
  # per tool directory.
  for dir in "$tools"/*/; do
    if [[ -f "$dir/package-lock.json" ]]; then
      echo "npm ci: ${dir#"$root"/}"
      (cd "$dir" && npm ci --no-audit --no-fund --loglevel=error)
    fi
  done
}

rust() {
  # The Rust suites build every generated SDK with `cargo --offline`, so its
  # pinned dependency set must already be in the registry cache with a
  # current index (a stale one picks yanked releases or none). Resolve it
  # once, online, in a throwaway crate mirroring rust_codecs::cargo_manifest.
  local scratch
  scratch="$(mktemp -d)"
  mkdir -p "$scratch/src"
  : > "$scratch/src/lib.rs"
  cat > "$scratch/Cargo.toml" <<'TOML'
[package]
name = "suspect-generated-rust-deps"
version = "0.0.0"
edition = "2024"
publish = false

[dependencies]
serde = { version = "=1.0.229", features = ["derive"] }
serde_json = { version = "=1.0.151", features = ["raw_value"] }
url = "=2.5.8"
reqwest = { version = "=0.12.28", default-features = false, features = ["rustls-tls"] }
# The consumer crates the Rust suites build pin their runtime exactly.
tokio = { version = "=1.53.1", features = ["macros", "rt", "rt-multi-thread", "time"] }
TOML
  echo "cargo fetch: generated Rust SDK dependencies"
  cargo fetch --quiet --manifest-path "$scratch/Cargo.toml"
  rm -rf "$scratch"
}

python() {
  # One venv holding the build, type-check and docs tools the Python suites
  # call as `target/sdk-native-python-tools/bin/python -m <tool>`. The
  # generated SDK pins httpx and Sphinx to these versions.
  local interpreter="${SUSPECT_PYTHON_CURRENT_BIN:-}"
  if [[ -z "$interpreter" ]]; then
    # A version-manager shim can be on PATH without a usable version, so
    # take the first interpreter that actually runs.
    for candidate in 3.14 3.13 3.12 3.11; do
      interpreter="$(python_minor "$candidate")"
      if [[ -n "$interpreter" ]]; then break; fi
    done
  fi
  if [[ -z "$interpreter" ]]; then
    echo "python: no python3.11+ found; set SUSPECT_PYTHON_CURRENT_BIN" >&2
    exit 1
  fi
  local venv="$root/target/sdk-native-python-tools"
  echo "python venv: ${venv#"$root"/} ($("$interpreter" --version))"
  "$interpreter" -m venv "$venv"
  "$venv/bin/python" -m pip install --quiet --upgrade pip
  # hatchling is the generated pyproject's build backend (`build
  # --no-isolation` imports it from this venv); mypy is pinned so new
  # releases' stricter narrowing cannot turn the strict gates red unannounced.
  "$venv/bin/python" -m pip install --quiet build wheel "hatchling==1.29.0" "mypy==2.4.1" \
    "Sphinx==8.2.3" "httpx==0.28.1"

  # The installed-wheel suites create one uv venv per admitted interpreter
  # (`suspect_codegen::toolchain::pinned_python`) and install the SDK wheel
  # plus Sphinx with `uv pip install --offline`; warm uv's cache here so
  # those installs resolve without the network.
  if ! command -v uv >/dev/null; then
    echo "python: uv not found; the installed-wheel suites need it (https://docs.astral.sh/uv/)" >&2
    exit 1
  fi
  local scratch minor found
  scratch="$(mktemp -d)"
  for minor in 3.11 3.14; do
    found="$(python_minor "$minor")"
    if [[ -z "$found" ]]; then
      echo "python: installing Python $minor with uv"
      uv python install --quiet "$minor"
      found="$(python_minor "$minor")"
    fi
    if [[ -z "$found" ]]; then
      echo "python: Python $minor still not found after uv python install" >&2
      exit 1
    fi
    echo "python $minor: $found (warming uv cache)"
    uv venv --quiet --python "$found" "$scratch/venv-$minor"
    uv pip install --quiet --python "$scratch/venv-$minor/bin/python" "Sphinx==8.2.3" "httpx==0.28.1"
  done
  rm -rf "$scratch"
}

# Prints a Python <minor> interpreter that reports that exact version, in
# `toolchain::pinned_python`'s order (PATH, mise, uv), or nothing.
python_minor() {
  local minor="$1" candidate
  local mise="${MISE_DATA_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/mise}"
  local uv_root="${UV_PYTHON_INSTALL_DIR:-$HOME/.local/share/uv/python}"
  for candidate in "$(command -v "python$minor" || true)" \
    "$mise/installs/python/$minor/bin/python$minor" \
    "$uv_root"/cpython-"$minor".*/bin/python"$minor"; do
    if [[ -x "$candidate" ]] &&
      [[ "$("$candidate" -c 'import sys; print("%d.%d" % sys.version_info[:2])' 2>/dev/null)" == "$minor" ]]; then
      echo "$candidate"
      return
    fi
  done
}

ruby() {
  # RBS signatures, Steep type checks and YARD docs for the Ruby suites
  # (they read SUSPECT_RUBY_GEMS, defaulting to this directory). Generated
  # gems' .yardopts select `--markup-provider redcarpet`, so YARD needs it.
  local gems="$root/target/sdk-ruby-tools/gems"
  local gem_bin="gem"
  if [[ -n "${SUSPECT_RUBY_BIN:-}" ]]; then gem_bin="$(dirname "$SUSPECT_RUBY_BIN")/gem"; fi
  echo "ruby gems: ${gems#"$root"/}"
  mkdir -p "$gems"
  "$gem_bin" install --no-document --install-dir "$gems" --bindir "$gems/bin" rbs steep yard redcarpet:3.6.1
}

php() {
  # Composer and PHPStan at the versions the PHP suites pin
  # (target/sdk-php-tools/<tool>-<version>.phar). PHP itself comes from
  # PATH or SUSPECT_PHP_BIN.
  local dir="$root/target/sdk-php-tools"
  mkdir -p "$dir"
  fetch "$dir/composer-2.10.3.phar" "https://getcomposer.org/download/2.10.3/composer.phar"
  fetch "$dir/phpstan-2.2.13.phar" \
    "https://github.com/phpstan/phpstan/releases/download/2.2.13/phpstan.phar"
}

cpp() {
  # The C++ suites configure generated packages with CMake + a C++20
  # compiler, link libcurl, and build the Doxygen docs target with
  # WARN_AS_ERROR. Doxygen is pinned to 1.9.8 (Ubuntu 24.04's package, the
  # generated CMakeLists' minimum): Homebrew's 1.18.0 intermittently dies
  # with SIGBUS in its comment scanner under parallel gates. CMake and the
  # compiler come from the host.
  if command -v apt-get >/dev/null; then
    if ! dpkg -s doxygen libcurl4-openssl-dev >/dev/null 2>&1; then
      echo "apt: doxygen libcurl4-openssl-dev"
      sudo apt-get update -qq
      sudo apt-get install -y -qq doxygen libcurl4-openssl-dev
    fi
    echo "present: doxygen $(doxygen --version)"
    return
  fi
  if [[ "$(uname -s)" != Darwin ]]; then
    echo "cpp: install Doxygen 1.9.8 and the libcurl headers, or set SUSPECT_CPP_DOXYGEN" >&2
    exit 1
  fi
  # macOS: the official 1.9.8 app bundle's binary (x86_64; Rosetta on arm64)
  # at target/sdk-cpp-tools/doxygen-1.9.8/doxygen, which toolchain::resolve_path
  # ("doxygen") prefers over PATH.
  local dir="$root/target/sdk-cpp-tools/doxygen-1.9.8"
  if [[ -x "$dir/doxygen" ]]; then
    echo "present: ${dir#"$root"/}/doxygen"
    return
  fi
  local scratch
  scratch="$(mktemp -d)"
  fetch "$scratch/Doxygen-1.9.8.dmg" \
    "https://github.com/doxygen/doxygen/releases/download/Release_1_9_8/Doxygen-1.9.8.dmg"
  hdiutil attach -quiet -nobrowse -readonly -mountpoint "$scratch/mnt" "$scratch/Doxygen-1.9.8.dmg"
  mkdir -p "$dir"
  cp "$scratch/mnt/Doxygen.app/Contents/Resources/doxygen" "$dir/doxygen.part"
  hdiutil detach -quiet "$scratch/mnt"
  mv "$dir/doxygen.part" "$dir/doxygen"
  rm -rf "$scratch"
  echo "installed: ${dir#"$root"/}/doxygen ($("$dir/doxygen" --version))"
}

fetch() {
  local dest="$1" url="$2"
  if [[ -s "$dest" ]]; then
    echo "present: ${dest#"$root"/}"
    return
  fi
  echo "download: ${dest#"$root"/}"
  curl --fail --location --silent --show-error --output "$dest.part" "$url"
  mv "$dest.part" "$dest"
}

if [[ $# -eq 0 ]]; then
  sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'
  exit 2
fi
for backend in "$@"; do
  case "$backend" in
    typescript | python | ruby | php | cpp | rust) "$backend" ;;
    # Go and Python SDK docs build with Sphinx from the Python tools venv.
    go) python ;;
    all) typescript; python; ruby; php; cpp; rust ;;
    *) echo "$backend: nothing to provision" ;;
  esac
done
