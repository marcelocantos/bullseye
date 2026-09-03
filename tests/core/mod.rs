// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

// tests/core_test.rs is one cargo test binary (see ~/.claude/rust.md on
// integration-test link cost); this module tree keeps it one binary while
// splitting the source by domain so a ledger-behaviour edit searches one
// file, not 7800+ lines. Mechanical split, see 🎯T74.11.
mod support;

mod cache;
mod commit;
mod convergence;
mod core_api;
mod git_guard;
mod github;
mod id_alloc;
mod init_location;
mod ownership_release;
mod portfolio;
mod query;
mod reshape;
mod resolve;
mod startup_summary;
mod store;
mod subdivide;
mod tool_schema;
mod verify_plan;
