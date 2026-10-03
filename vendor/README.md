# Vendored dependency patches

These trees are copies of crates.io packages with **minimal local patches**
so bullseye can ship without waiting on upstream feature trims. Licenses
remain as shipped by the upstream crates (MIT for the rust-mcp-* packages —
see each `LICENSE`).

## `rust-mcp-sdk` / `rust-mcp-transport` (🎯T47, 🎯T74.13)

- **Upstream**: crates.io `rust-mcp-sdk` 0.9.0, `rust-mcp-transport` 0.9.0.
- **Patch**: `tokio` features changed from `["full"]` to the required runtime,
  I/O, sync, time and process features in the vendored manifests. Bullseye
  also enables `signal` and `net` for the HTTP listener/shutdown path.
- **Why**: Cargo feature unification would otherwise force the full Tokio
  stack into every build. The shipped MCP transport is **Streamable HTTP**
  (`server`, `macros`, `hyper-server`); do not remove HTTP or the Tokio trim.
  Bullseye binds loopback without authentication. The SDK's `auth` feature
  is excluded by `default-features = false`; its shared `AuthInfo`/`Audience`
  types remain because server traits and runtimes reference them even with
  auth disabled. Auth-only helpers and disabled SSE paths have narrow feature
  guards to avoid misleading warnings in Bullseye's feature set.
- **Wired via** `[patch.crates-io]` in the root `Cargo.toml`.

## Manual refresh policy (accepted)

The product owner explicitly accepts manual vendor refresh maintenance for
these two patched crates (🎯T74.13). No automated refresh is required.
On each upstream version bump, the maintainer must re-copy and review the
upstream diff, re-apply the Tokio trim and feature guards, then verify both
Bullseye's HTTP transport and the relevant SDK feature combinations. Do not
assume the patches are still needed or that upstream has preserved the same
feature structure; remove the patches only after verifying upstream no
longer enables `tokio/full` for Bullseye's HTTP feature set.

```bash
# After bumping the version in Cargo.toml, re-copy from the registry:
cp -R ~/.cargo/registry/src/*/rust-mcp-sdk-<ver> vendor/rust-mcp-sdk
cp -R ~/.cargo/registry/src/*/rust-mcp-transport-<ver> vendor/rust-mcp-transport
# Re-apply the Tokio feature edit (search for features = ["full"]) and
# compare the local feature guards against upstream before restoring them.
# Check server+hyper-server (no auth), server+hyper-server+auth+sse,
# transport sse, and the root HTTP transport integration tests.
```
