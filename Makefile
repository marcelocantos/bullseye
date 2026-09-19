# Makefile for bullseye — development helpers.
#
# The canonical build system is `cargo`; this Makefile is thin
# orchestration over cargo commands. The primary rule is `bullseye`,
# which is invoked by `bullseye_convergence` (yes, bullseye runs its
# own convergence hook — classic dogfooding) to check standing
# invariants before recommending next work.


# Standing-invariants check. Exit 0 = all green, non-zero = at least
# one violation. Stdout is relayed verbatim to the agent in the
# bullseye_convergence response, so the ✓ / ✗ bullets below are
# human-readable status, not machine-parsed.
#
# Ordered cheapest-first so fast feedback arrives first:
#   1. fmt check (sub-second)
#   2. clippy     (a few seconds warm)
#   3. tests
#   4. declared checks (🎯T86)
#   5. TLA+       (~13–22 min: mutants first, then the faithful config)
#   6. dirty tree (warning only — leftover WIP is the normal /cv state)
#
# Steps 1–4 are `gate` — the oracle scripts/hooks/pre-push runs before
# every push. TLC stays on `bullseye` / `tla`, not on the push hook.
gate:
	@log=$$(mktemp); \
	  if cargo fmt --check >"$$log" 2>&1; then echo "✓ fmt"; \
	  else echo "✗ fmt"; cat "$$log"; rm -f "$$log"; exit 1; fi; rm -f "$$log"
	@log=$$(mktemp); \
	  if cargo clippy --quiet --all-targets -- -D warnings >"$$log" 2>&1; then echo "✓ clippy"; \
	  else echo "✗ clippy"; grep -v '^ *--> vendor/' "$$log"; rm -f "$$log"; exit 1; fi; rm -f "$$log"
	@log=$$(mktemp); \
	  if cargo test --quiet >"$$log" 2>&1; then echo "✓ tests"; \
	  else echo "✗ tests"; cat "$$log"; rm -f "$$log"; exit 1; fi; rm -f "$$log"
# Bullseye eats its own cooking (🎯T86): the checks targets declare in this
# ledger are run by this gate, so a declared check that goes red reddens the
# gate. Without this step `checks:` is documentation. Uses the just-built
# debug binary, not an installed one — the gate must adjudicate this tree.
	@log=$$(mktemp); \
	  if cargo run --quiet -- run-checks --all --cwd . >"$$log" 2>&1; then \
	    echo "✓ declared checks"; grep -E '^(Gate passed|No target)' "$$log" || true; \
	  else echo "✗ declared checks"; cat "$$log"; rm -f "$$log"; exit 1; fi; rm -f "$$log"

bullseye: gate
	@./formal/check
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

# TLC on the in-repo Convergence instance (faithful green + three mutants).
# Wired into `bullseye` (the standing invariant), not into `gate` / `check`.
tla:
	./formal/check

# Once per clone. A relative core.hooksPath resolves against the worktree
# the push runs from, so one setting covers every `git worktree`.
hooks:
	git config core.hooksPath scripts/hooks
	@echo "core.hooksPath=$$(git config core.hooksPath)"

check: fmt lint test

fmt:
	cargo fmt

lint:
	cargo clippy --all-targets -- -D warnings

build:
	cargo build

# --- release (local; the gate above is the preflight oracle) ----------
#
#   make release-dist  — build dist/bullseye-<ver>-{darwin-arm64,linux-*}.tar.gz
#   make release-tap   — update marcelocantos/homebrew-tap (release must exist)
#   make release       — gate + dist + gh release create + tap + brew upgrade
#
# There is no CI. The gate runs here, publish runs here, and both are
# reproducible on the machine in front of you.

release-dist:
	./scripts/release-package.sh

release-tap:
	./scripts/release-tap.sh

# Is the newest PUBLISHED release able to reach every repair path
# (🎯T70)? Ran in ci.yml until the gate moved local; keep it runnable
# on its own, not only as part of a release.
reachability:
	./scripts/probe-published-release.sh

release: bullseye release-dist
	./scripts/release-publish.sh

clean:
	cargo clean
	rm -rf dist

.PHONY: gate hooks tla bullseye test check fmt lint build release release-dist release-tap reachability clean
