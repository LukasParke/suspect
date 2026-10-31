#!/usr/bin/env bash
# Opens the zed-industries/extensions PR that moves the suspect submodule to
# the release commit and sets its version (docs/proposals/P5-release.md,
# step 8). Called by publish-editors.yml from a checkout of the release tag.
#
#   zed-registry-pr.sh <version>
#
# env:
#   ZED_REGISTRY_FORK  fork of zed-industries/extensions the token can push to
#                      (owner/name)
#   GH_TOKEN           token with push access to the fork and permission to
#                      open PRs (ZED_EXTENSIONS_TOKEN)
#   DRY_RUN=1          prepare the commit and print it; no push, no PR
#
# The first submission (the `[suspect]` entry and the submodule) is manual;
# this script only updates an existing entry.
set -euo pipefail

version="${1:?usage: zed-registry-pr.sh <version>}"
version="${version#v}"
upstream="zed-industries/extensions"
fork="${ZED_REGISTRY_FORK:?set ZED_REGISTRY_FORK to the owner/name of the registry fork}"
branch="suspect-v$version"
repo_root=$(git rev-parse --show-toplevel)
sha=$(git -C "$repo_root" rev-parse HEAD)

# Zed builds the submodule commit and requires it to be on a branch.
git -C "$repo_root" fetch --quiet origin main
if ! git -C "$repo_root" merge-base --is-ancestor "$sha" origin/main; then
  echo "error: $sha is not on main; Zed requires the submodule commit to be on a branch" >&2
  exit 1
fi
want=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$repo_root/editors/zed/extension.toml" | head -n1)
if [ "$want" != "$version" ]; then
  echo "error: editors/zed/extension.toml has version $want, expected $version" >&2
  exit 1
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
# ZED_REGISTRY_URL overrides the clone URL (tests).
git clone --quiet --filter=blob:none --no-checkout --depth 1 \
  "${ZED_REGISTRY_URL:-https://github.com/$upstream.git}" "$work/extensions"
cd "$work/extensions"
git checkout --quiet -B "$branch" origin/main

# The [suspect] table: its submodule path and the version line to replace.
if ! grep -qx '\[suspect\]' extensions.toml; then
  echo "error: extensions.toml has no [suspect] entry; make the first submission by hand" >&2
  exit 1
fi
submodule=$(awk '/^\[/{t=$0} t=="[suspect]" && /^submodule = /{gsub(/"/,"",$3); print $3; exit}' extensions.toml)
[ -n "$submodule" ] || { echo "error: [suspect] has no submodule path" >&2; exit 1; }
awk -v v="$version" '
  /^\[/ { t = $0 }
  t == "[suspect]" && /^version = / { $0 = "version = \"" v "\"" }
  { print }
' extensions.toml > extensions.toml.new
mv extensions.toml.new extensions.toml

# Move the gitlink without fetching the submodule.
git update-index --cacheinfo "160000,$sha,$submodule"
git add extensions.toml
if git diff --cached --quiet; then
  echo "the registry already lists suspect $version at $sha"
  exit 0
fi
git -c user.name="suspect release" -c user.email="noreply@github.com" \
  commit --quiet -m "Update suspect to v$version"
git show --stat HEAD

if [ "${DRY_RUN:-}" = 1 ]; then
  echo "DRY_RUN=1: not pushing to $fork or opening a PR"
  exit 0
fi
: "${GH_TOKEN:?GH_TOKEN is required}"
# The clone is shallow, so the fork must already have upstream's main.
gh repo sync "$fork" --branch main
git push --quiet --force "https://x-access-token:$GH_TOKEN@github.com/$fork.git" "HEAD:refs/heads/$branch"
gh pr create --repo "$upstream" --base main --head "${fork%%/*}:$branch" \
  --title "Update suspect to v$version" \
  --body "Moves the suspect extension to v$version (https://github.com/LukasParke/suspect/releases/tag/v$version). Opened by the suspect release workflow."
