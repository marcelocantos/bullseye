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
#   4. dirty tree (warning only — leftover WIP is the normal /cv state)
bullseye:
	@log=$$(mktemp); \
	  if cargo fmt --check >"$$log" 2>&1; then echo "✓ fmt"; \
	  else echo "✗ fmt"; cat "$$log"; rm -f "$$log"; exit 1; fi; rm -f "$$log"
	@log=$$(mktemp); \
	  if cargo clippy --quiet --all-targets -- -D warnings >"$$log" 2>&1; then echo "✓ clippy"; \
	  else echo "✗ clippy"; grep -v '^ *--> vendor/' "$$log"; rm -f "$$log"; exit 1; fi; rm -f "$$log"
	@log=$$(mktemp); \
	  if cargo test --quiet >"$$log" 2>&1; then echo "✓ tests"; \
	  else echo "✗ tests"; cat "$$log"; rm -f "$$log"; exit 1; fi; rm -f "$$log"
	@dirty=$$(git status --porcelain | grep -vE 'bullseye\.yaml$$' || true); \
	if [ -z "$$dirty" ]; then echo "✓ working tree clean"; \
	else \
	  echo ""; \
	  echo "================================================================"; \
	  echo "⚠  DIRTY WORKING TREE"; \
	  echo ""; \
	  echo "Warning only — invariants still pass (exit 0)."; \
	  echo "Look at the files below before starting a new target."; \
	  echo "Leftover work from a different objective → park it in a commit first."; \
	  echo "This session's WIP on the recommended target → continue."; \
	  echo "================================================================"; \
	  echo "$$dirty"; \
	  echo "================================================================"; \
	  echo ""; \
	fi

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
