#!/usr/bin/env bash
# Shared helpers for the local release path (package, publish, tap).
#
# The gate and the publish both run on the dev Mac. There is no CI to
# ask; `make check` is the whole definition of green, and it is
# reproducible on the machine in front of you.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# Version lives in Cargo.toml's [package] table. Scoped to that table so
# a dependency's `version = ` can never be picked up instead.
release_version() {
	awk '
		/^\[package\]/ { in_pkg = 1; next }
		/^\[/          { in_pkg = 0 }
		in_pkg && /^version *= *"/ {
			match($0, /"[^"]*"/)
			print substr($0, RSTART + 1, RLENGTH - 2)
			exit
		}
	' Cargo.toml
}

release_tag() {
	printf 'v%s' "$(release_version)"
}

require_gh() {
	command -v gh >/dev/null || {
		echo "gh CLI required" >&2
		exit 1
	}
	gh auth status >/dev/null 2>&1 || {
		echo "gh auth login required" >&2
		exit 1
	}
}

# Cross-compiling to Linux needs rustup's toolchain, which carries the
# linux-gnu std. Homebrew's rust ships only the host target and sits
# earlier on PATH, so `cargo` alone resolves to a toolchain that cannot
# see `core` for these targets — and fails with the misleading
# "target may not be installed", even though `rustup target list` shows
# it installed (rustup added it to a different toolchain).
rustup_cargo() {
	local candidate="$HOME/.cargo/bin/cargo"
	[[ -x "$candidate" ]] || {
		echo "rustup cargo not found at $candidate — cross builds need rustup, not Homebrew rust" >&2
		exit 1
	}
	printf '%s' "$candidate"
}
