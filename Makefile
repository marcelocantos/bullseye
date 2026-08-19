# Makefile for bullseye — development helpers.
#
# The canonical build system is `cargo`; this Makefile is thin
# orchestration over cargo commands. The primary rule is `bullseye`,
# which is invoked by `bullseye_convergence` (yes, bullseye runs its
# own convergence hook — classic dogfooding) to check standing
# invariants before recommending next work.

.PHONY: bullseye test check fmt lint build

# Standing-invariants check. Exit 0 = all green, non-zero = at least
# one violation. Stdout is relayed verbatim to the agent in the
# bullseye_convergence response, so the ✓ / ✗ bullets below are
# human-readable status, not machine-parsed.
#
# Ordered cheapest-first so fast feedback arrives first:
#   1. fmt check (sub-second)
#   2. clippy     (a few seconds warm, via rust-cache in CI)
#   3. tests      (a few seconds warm)
#   4. clean tree (trivial git call)
bullseye:
	@cargo fmt --check >/dev/null && echo "✓ fmt"
	@cargo clippy --quiet --all-targets -- -D warnings >/dev/null 2>&1 && echo "✓ clippy"
	@cargo test --quiet >/dev/null 2>&1 && echo "✓ tests"
	@test -z "$$(git status --porcelain | grep -vE 'bullseye\.yaml$$')" && echo "✓ clean tree" || \
	 (echo "✗ dirty tree:"; git status --short | grep -vE 'bullseye\.yaml$$'; exit 1)

# Convenience aliases for common cargo commands.
test:
	cargo test

check: fmt lint test

fmt:
	cargo fmt

lint:
	cargo clippy --all-targets -- -D warnings

build:
	cargo build
