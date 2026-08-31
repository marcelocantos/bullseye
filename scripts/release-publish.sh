#!/usr/bin/env bash
# Create the GitHub release from dist/, update the Homebrew tap, and
# verify the published binary by upgrading this machine to it.
#
# Run after the gate: `make release` runs `make bullseye` first.
set -euo pipefail
source "$(dirname "$0")/release-common.sh"

TAG="$(release_tag)"
VERSION="$(release_version)"
NOTES_FILE=""
SKIP_TAP=false
SKIP_BREW=false

usage() {
	echo "usage: $0 [--notes FILE] [--skip-tap] [--skip-brew]" >&2
	exit 2
}

while [[ $# -gt 0 ]]; do
	case "$1" in
	--notes)
		NOTES_FILE=$2
		shift 2
		;;
	--skip-tap)
		SKIP_TAP=true
		shift
		;;
	--skip-brew)
		SKIP_BREW=true
		shift
		;;
	-h | --help) usage ;;
	*)
		echo "unknown argument: $1" >&2
		usage
		;;
	esac
done

require_gh

for triple in darwin-arm64 linux-amd64 linux-arm64; do
	f="dist/bullseye-${VERSION}-${triple}.tar.gz"
	[[ -f "$f" ]] || {
		echo "missing $f — run 'make release-dist' first" >&2
		exit 1
	}
done

if gh release view "$TAG" >/dev/null 2>&1; then
	echo "release-publish: ${TAG} already exists on GitHub" >&2
	exit 1
fi

notes_tmp=""
if [[ -z "$NOTES_FILE" ]]; then
	notes_tmp="$(mktemp)"
	NOTES_FILE="$notes_tmp"
	if prev="$(git describe --tags --abbrev=0 2>/dev/null)"; then
		git log "${prev}..HEAD" --pretty=format:'- %s' >"$NOTES_FILE" || true
	fi
	[[ -s "$NOTES_FILE" ]] || echo "Release ${TAG}." >"$NOTES_FILE"
fi
trap 'rm -f "$notes_tmp"' EXIT

echo "release-publish: creating ${TAG} …" >&2
gh release create "$TAG" --title "$TAG" --notes-file "$NOTES_FILE" \
	dist/*.tar.gz "dist/bullseye-${VERSION}-SHA256SUMS"
git fetch --tags

if [[ "$SKIP_TAP" == false ]]; then
	"$(dirname "$0")/release-tap.sh" "$TAG"
fi

if [[ "$SKIP_BREW" == false ]]; then
	echo "release-publish: brew upgrade …" >&2
	brew update
	brew upgrade marcelocantos/tap/bullseye 2>/dev/null ||
		brew install marcelocantos/tap/bullseye
	got="$(bullseye --version 2>/dev/null | awk '{print $2}')"
	if [[ "$got" != "$VERSION" ]]; then
		echo "release-publish: expected bullseye ${VERSION}, got ${got:-<missing>}" >&2
		exit 1
	fi
fi

# The reachability probe (🎯T70) used to run in ci.yml. With CI gone it
# runs here, and this is the better position anyway: post-publish, the
# "newest published release" it probes is the one just shipped, so it
# answers whether agents can actually reach the repair paths rather
# than whether the previous release could.
if [[ "$SKIP_BREW" == false ]]; then
	echo "release-publish: probing published release for reachability …" >&2
	"$ROOT/scripts/probe-published-release.sh"
fi

echo "release-publish: ${TAG} → https://github.com/marcelocantos/bullseye/releases/tag/${TAG}"
