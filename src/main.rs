// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use targets::handler::TargetHandler;
use rust_mcp_sdk::mcp_server::{server_runtime, McpServerOptions};
use rust_mcp_sdk::schema::{
    Implementation, InitializeResult, ProtocolVersion, ServerCapabilities,
    ServerCapabilitiesTools,
};
use rust_mcp_sdk::{error::SdkResult, McpServer, StdioTransport, ToMcpServerHandler, TransportOptions};

#[tokio::main]
async fn main() -> SdkResult<()> {
    let server_details = InitializeResult {
        server_info: Implementation {
            name: "targets".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            title: Some("Convergence Targets MCP Server".to_string()),
            description: Some(
                "Manage convergence targets — desired states expressed as testable \
                 properties. Provides WSJF ranking, validation, and dependency graph \
                 analysis."
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
            "Convergence target management. Targets are desired states expressed as \
             testable properties. Use targets_list to see active targets, targets_rank \
             for WSJF-ordered recommendations, targets_add/update/retire to manage them."
                .to_string(),
        ),
        protocol_version: ProtocolVersion::V2025_11_25.into(),
    };

    let transport = StdioTransport::new(TransportOptions::default())?;
    let handler = TargetHandler::default().to_mcp_server_handler();
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
