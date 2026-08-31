#!/usr/bin/env bash
# Update the Homebrew tap declared in tapper.yaml for an existing release.
set -euo pipefail
source "$(dirname "$0")/release-common.sh"

TAG="${1:-$(release_tag)}"
command -v tapper >/dev/null || {
	echo "tapper not on PATH — see https://github.com/marcelocantos/tapper" >&2
	exit 1
}
exec tapper push --version "$TAG"
