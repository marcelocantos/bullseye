// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Version string with build provenance (🎯T69).
//!
//! The crate version alone cannot distinguish a binary built from a fix
//! from a release carrying the same unbumped version. Every surface that
//! prints a version prints [`VERSION`], which appends the source commit
//! derived by `build.rs`.

/// Crate version from Cargo — the release identity.
pub const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Build identity: short commit SHA, `-dirty` when the working tree
/// differed from that commit, or `unknown` when the build saw no git
/// metadata. Set by `build.rs`.
pub const PROVENANCE: &str = env!("BULLSEYE_BUILD_PROVENANCE");

/// `0.44.0 (a1b2c3d4e5f6)` — release identity plus build identity. Two
/// binaries built from different commits differ here even when their
/// crate versions match.
pub const VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("BULLSEYE_BUILD_PROVENANCE"),
    ")"
);
