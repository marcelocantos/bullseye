#!/usr/bin/env bash
# Run the gate checks given as arguments and, if they pass on an unchanged
# tree, attest that tree in .git/gate-attestation. scripts/hooks/pre-push
# checks the attestation instead of re-running the gate: git opens the SSH
# connection before the hook runs, and GitHub closes an idle connection long
# before a slow gate finishes (claudia, 2026-09-22).
#
# The attestation is per-clone state in the git dir. It is never committed.
#
# macOS /bin/bash is 3.2: no associative arrays, no ${var,,}.
set -euo pipefail

[[ $# -gt 0 ]] || { echo "usage: $0 <gate command...>" >&2; exit 2; }

snapshot() {
	tree="$(git rev-parse 'HEAD^{tree}')"
	dirty="$(git status --porcelain | wc -l | tr -d ' ')"
	# Content of uncommitted changes, so an edit during the run is caught
	# even when the count of changed files stays the same.
	diff_sum="$( { git diff HEAD; git ls-files --others --exclude-standard; } | shasum -a 256)"
}

snapshot
tree0="$tree" dirty0="$dirty" diff0="$diff_sum"

"$@"

snapshot
if [[ "$tree" != "$tree0" || "$dirty" != "$dirty0" || "$diff_sum" != "$diff0" ]]; then
	echo "gate-attest: the tree changed while the gate ran — not attested" >&2
	exit 1
fi

att="$(git rev-parse --absolute-git-dir)/gate-attestation"
tmp="$(mktemp "$att.XXXXXX")"
printf 'tree=%s\ndirty=%s\nat=%s\n' "$tree" "$dirty" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" > "$tmp"
mv "$tmp" "$att"
if [[ "$dirty" == 0 ]]; then
	echo "✓ attested tree ${tree:0:12}"
else
	echo "⚠ attested tree ${tree:0:12} with $dirty uncommitted change(s) — pre-push will refuse it"
fi
