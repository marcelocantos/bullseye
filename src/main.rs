// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use std::process;

use bullseye::handler::TargetHandler;
use bullseye::priorities;
use rust_mcp_sdk::mcp_server::{McpServerOptions, server_runtime};
use rust_mcp_sdk::schema::{
    Implementation, InitializeResult, ProtocolVersion, ServerCapabilities, ServerCapabilitiesTools,
};
use rust_mcp_sdk::{
    McpServer, StdioTransport, ToMcpServerHandler, TransportOptions, error::SdkResult,
};

const AGENT_GUIDE: &str = include_str!("../docs/agents-guide.md");

#[tokio::main]
async fn main() -> SdkResult<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        match args[1].as_str() {
            "--version" => {
                println!("bullseye {}", env!("CARGO_PKG_VERSION"));
                process::exit(0);
            }
            "--help" => {
                print_help();
                process::exit(0);
            }
            "--help-agent" => {
                print_help();
                println!();
                print!("{AGENT_GUIDE}");
                process::exit(0);
            }
            "sync-priorities" => match priorities::run_sync(&args[2..]) {
                Ok(msg) => {
                    println!("{msg}");
                    process::exit(0);
                }
                Err(e) => {
                    eprintln!("sync-priorities: {e}");
                    process::exit(1);
                }
            },
            other => {
                eprintln!("unknown flag: {other}");
                eprintln!();
                print_help();
                process::exit(1);
            }
        }
    }

    let server_details = InitializeResult {
        server_info: Implementation {
            name: "bullseye".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            title: Some("Bullseye — Target Management MCP Server".to_string()),
            description: Some(
                "Manage targets — desired states expressed as testable properties. \
                 Provides frontier computation, validation, and dependency graph analysis."
                    .to_string(),
            ),
            icons: vec![],
            website_url: None,
        },
        capabilities: ServerCapabilities {
            tools: Some(ServerCapabilitiesTools { list_changed: None }),
            ..Default::default()
        },
        meta: None,
        instructions: Some(
            "Target management. Targets are desired states expressed as testable \
             properties. Use bullseye_list to see active targets, bullseye_frontier \
             for unblocked targets ready for work, bullseye_put to create or \
             patch targets, and bullseye_retire to mark them achieved."
                .to_string(),
        ),
        protocol_version: ProtocolVersion::V2025_11_25.into(),
    };

    let transport = StdioTransport::new(TransportOptions::default())?;
    let handler = TargetHandler.to_mcp_server_handler();
    let server = server_runtime::create_server(McpServerOptions {
        transport,
        handler,
        server_details,
        task_store: None,
        client_task_store: None,
        message_observer: None,
    });

    server.start().await
}

fn print_help() {
    println!(
        "bullseye {} — Target Management MCP Server",
        env!("CARGO_PKG_VERSION")
    );
    println!();
    println!("USAGE:");
    println!("    bullseye                       Start the MCP server (stdio transport)");
    println!("    bullseye sync-priorities ...   Sync portfolio frontier into a SQLite table");
    println!();
    println!("FLAGS:");
    println!("    --version             Print version");
    println!("    --help                Print this help");
    println!("    --help-agent          Print help and agent guide");
    println!();
    println!("Run `bullseye sync-priorities --help` for subcommand details.");
}
