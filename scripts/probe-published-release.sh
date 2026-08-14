#!/usr/bin/env bash
# Copyright 2026 Marcelo Cantos
# SPDX-License-Identifier: Apache-2.0
#
# Probe the newest PUBLISHED release for reachability (🎯T70).
#
# 🎯T68 built the oracle: tests/reachability_test.rs feeds a ledger
# carrying status-scoped residue to an installed binary and reads what
# it says. But it can only ask that question on a machine that has an
# installed release, and GitHub runners have none — so ci.yml opted the
# probe out and CI enforced nothing. This script closes that: it
# installs the newest published release into a directory outside the
# workspace and points the oracle at it, so the delivery question is
# answered without depending on any developer's local toolchain state.
#
# NO VERSION COMPARISON, DELIBERATELY. The 🎯T64 incident is the whole
# reason: the fixed master build and the broken published build both
# reported `bullseye 0.44.0`, so a version gate would have been green
# while no agent in the fleet could reach the repair path. This script
# RECORDS the version it probed — acceptance requires naming it — and
# never gates on it. The verdict is purely behavioural.
#
# Runs on GitHub Actions and locally. Locally, override the platform:
#
#     BULLSEYE_RELEASE_ASSET=darwin-arm64 scripts/probe-published-release.sh
#
# Exits non-zero when the published release cannot repair the ledger,
# which is the point: source-present, binary-absent is a red build.

set -euo pipefail

# Platform slug in the release asset name (see .github/workflows/release.yml).
ASSET_PLATFORM="${BULLSEYE_RELEASE_ASSET:-linux-amd64}"
# Pin a tag to probe an older release; default is the newest published.
RELEASE_TAG="${BULLSEYE_RELEASE_TAG:-}"
REPO="${GITHUB_REPOSITORY:-marcelocantos/bullseye}"

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

for tool in gh cargo tar; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "probe-published-release: required tool '$tool' not found" >&2
    exit 1
  }
done

# A probe that opts itself out proves nothing. Refuse rather than pass
# vacuously — the exact failure mode 🎯T68 exists to prevent.
if [[ "${BULLSEYE_REACHABILITY_CHECK:-}" == "skip" ]]; then
  echo "probe-published-release: BULLSEYE_REACHABILITY_CHECK=skip is set." >&2
  echo "This job IS the reachability enforcement point; it may not skip." >&2
  exit 1
fi

INSTALL_DIR="$(mktemp -d)"
trap 'rm -rf "$INSTALL_DIR"' EXIT

# The oracle rejects a binary inside the workspace as a trivially-passing
# cargo artifact. Fail here instead, where the message can say why.
case "$INSTALL_DIR" in
  "$REPO_ROOT"/*)
    echo "probe-published-release: install dir $INSTALL_DIR is inside the" >&2
    echo "workspace $REPO_ROOT; the probe requires an out-of-tree install." >&2
    exit 1
    ;;
esac

if [[ -z "$RELEASE_TAG" ]]; then
  RELEASE_TAG="$(gh release view --repo "$REPO" --json tagName -q .tagName)"
fi
if [[ -z "$RELEASE_TAG" ]]; then
  echo "probe-published-release: could not resolve a published release for $REPO" >&2
  exit 1
fi

echo "==> probing published release $RELEASE_TAG ($ASSET_PLATFORM) from $REPO"

gh release download "$RELEASE_TAG" \
  --repo "$REPO" \
  --pattern "bullseye-*-${ASSET_PLATFORM}.tar.gz" \
  --dir "$INSTALL_DIR"

# Glob rather than `find | head`: under `pipefail` a short-reading head
# can take the whole script down on a SIGPIPE it did nothing wrong to get.
TARBALL=""
for candidate in "$INSTALL_DIR"/bullseye-*-"${ASSET_PLATFORM}".tar.gz; do
  [[ -f "$candidate" ]] || continue
  TARBALL="$candidate"
  break
done
if [[ -z "$TARBALL" ]]; then
  echo "probe-published-release: release $RELEASE_TAG has no" >&2
  echo "bullseye-*-${ASSET_PLATFORM}.tar.gz asset to install." >&2
  exit 1
fi

tar xzf "$TARBALL" -C "$INSTALL_DIR"
INSTALLED_BIN="$INSTALL_DIR/bullseye"
if [[ ! -f "$INSTALLED_BIN" ]]; then
  echo "probe-published-release: $TARBALL did not contain a 'bullseye' binary" >&2
  exit 1
fi
chmod +x "$INSTALLED_BIN"

# Recorded for the record, never gated on — see the header.
PROBED_VERSION="$("$INSTALLED_BIN" --version 2>&1 || true)"
echo "==> installed $INSTALLED_BIN"
echo "==> published release $RELEASE_TAG reports: $PROBED_VERSION"

# --no-default-features skips the bundled-SQLite compile. Safe here: the
# oracle only shells out to the already-built binary above, so no feature
# of this workspace's own build affects the verdict.
LOG="$INSTALL_DIR/probe.log"
rc=0
BULLSEYE_INSTALLED_BIN="$INSTALLED_BIN" \
  cargo test --test reachability_test --no-default-features -- --nocapture \
  >"$LOG" 2>&1 || rc=$?
cat "$LOG"

if [[ "$rc" -ne 0 ]]; then
  echo
  echo "REACHABILITY FAILURE: published release $RELEASE_TAG ($PROBED_VERSION)" >&2
  echo "cannot repair a ledger that master's source can. The fix is in the" >&2
  echo "repo and NOT in the binary agents invoke — cut a release." >&2
  exit "$rc"
fi

# The oracle returns success when it SKIPS, too. Demand the markers that
# only a probe which actually ran can print.
for marker in "🎯T68 reachability OK" "🎯T68 rehash repair OK"; do
  if ! grep -qF "$marker" "$LOG"; then
    echo "probe-published-release: probe reported success without printing" >&2
    echo "'$marker' — it did not actually run against the installed binary." >&2
    exit 1
  fi
done

echo
echo "==> REACHABLE: published release $RELEASE_TAG ($PROBED_VERSION) carries"
echo "    every repair path tests/reachability_test.rs probes for."
