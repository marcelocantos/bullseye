# Vendored dependency patches

These trees are copies of crates.io packages with **minimal local
patches** so bullseye can ship without waiting on upstream feature
trims. Licenses remain as shipped by the upstream crates (MIT for the
rust-mcp-* packages — see each `LICENSE`).

## `rust-mcp-sdk` / `rust-mcp-transport` (🎯T47)

- **Upstream**: crates.io `rust-mcp-sdk` 0.9.0, `rust-mcp-transport` 0.9.0
- **Patch**: `tokio` features changed from `["full"]` to
  `macros`, `rt-multi-thread`, `io-util`, `io-std`, `sync`, `time`
- **Why**: cargo feature-unification forced the full tokio stack into
  every bullseye build even though the stdio MCP path only needs a
  multi-thread runtime and I/O helpers.
- **Wired via** `[patch.crates-io]` in the root `Cargo.toml`.

When upstream publishes a release that no longer enables `tokio/full`
for the stdio path, drop the patch and delete these directories.

## Refresh procedure

```bash
# After bumping the version in Cargo.toml, re-copy from the registry:
cp -R ~/.cargo/registry/src/*/rust-mcp-sdk-<ver> vendor/rust-mcp-sdk
cp -R ~/.cargo/registry/src/*/rust-mcp-transport-<ver> vendor/rust-mcp-transport
# Re-apply the tokio feature edit (search for features = ["full"]).
```
