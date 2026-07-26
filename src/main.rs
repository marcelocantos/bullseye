// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use std::process;

use bullseye::handler::{
    TargetHandler, handle_commit, handle_open, handle_plan_checks, handle_query,
};
use bullseye::tools::{CommitTool, OpenTool, PlanChecksTool, QueryTool};
use rust_mcp_sdk::mcp_server::{McpServerOptions, server_runtime};
use rust_mcp_sdk::schema::{
    CallToolResult, ContentBlock, Implementation, InitializeResult, ProtocolVersion,
    ServerCapabilities, ServerCapabilitiesTools,
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
            "open" => cli_exit(cli_open(&args[2..])),
            "query" => cli_exit(cli_query(&args[2..])),
            "commit" => cli_exit(cli_commit(&args[2..])),
            "plan-checks" => cli_exit(cli_plan_checks(&args[2..])),
            "sync-priorities" => {
                #[cfg(feature = "sqlite")]
                match bullseye::priorities::run_sync(&args[2..]) {
                    Ok(msg) => {
                        println!("{msg}");
                        process::exit(0);
                    }
                    Err(e) => {
                        eprintln!("sync-priorities: {e}");
                        process::exit(1);
                    }
                }
                #[cfg(not(feature = "sqlite"))]
                {
                    eprintln!(
                        "sync-priorities: this binary was built without the `sqlite` feature \
                         (cargo build --no-default-features). Rebuild with default features \
                         or `--features sqlite`."
                    );
                    process::exit(1);
                }
            }
            "github" => match bullseye::github::run(&args[2..]) {
                Ok(msg) => {
                    println!("{msg}");
                    process::exit(0);
                }
                Err(e) => {
                    eprintln!("github: {e}");
                    process::exit(1);
                }
            },
            "issues-poll" => {
                #[cfg(feature = "github-issues")]
                match bullseye::github_issues::http::run(&args[2..]) {
                    Ok(msg) => {
                        println!("{msg}");
                        process::exit(0);
                    }
                    Err(e) => {
                        eprintln!("issues-poll: {e}");
                        process::exit(1);
                    }
                }
                #[cfg(not(feature = "github-issues"))]
                {
                    eprintln!(
                        "issues-poll: rebuild with `--features github-issues` \
                         (event-path consumer; 🎯T33)."
                    );
                    process::exit(1);
                }
            }
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
            title: Some("Bullseye — Intent Ledger MCP Server".to_string()),
            description: Some(
                "Shared intent ledger: desired states, dependencies, and claim lifecycle. \
                 Core tools: bullseye_open, bullseye_query, bullseye_commit, bullseye_plan_checks."
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
            "Bullseye is an intent ledger (git-for-intent), not a task assigner. \
             Core tools: bullseye_open (discover/init/context), bullseye_query \
             (views: context|frontier|target|list|summary|graph|validate), \
             bullseye_commit (ops: track|block|split|achieve|defer|reopen), \
             bullseye_plan_checks (plan only). User intent overrides the frontier. \
             Commit at boundaries for lasting work; do not gate one-shot tasks on the graph. \
             Legacy tools remain as shims; portfolio/github/convergence/import/resolve are extended (L2)."
                .to_string(),
        ),
        protocol_version: ProtocolVersion::V2025_11_25.into(),
    };

    // Event-path background consumer (🎯T35): when BULLSEYE_ISSUEPIPE_* env
    // is set, poll the Master so issues become targets without CLI action.
    #[cfg(feature = "github-issues")]
    bullseye::github_issues::http::maybe_spawn_background_from_env();

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
        "bullseye {} — Intent Ledger MCP Server",
        env!("CARGO_PKG_VERSION")
    );
    println!();
    println!("USAGE:");
    println!("    bullseye                       Start the MCP server (stdio transport)");
    println!("    bullseye open [--cwd DIR] [--location in_repo|external]");
    println!("    bullseye query --view VIEW [--cwd DIR] [--id ID] [--filter F]");
    println!("    bullseye commit --op OP [flags]   # see bullseye commit --help");
    println!("    bullseye plan-checks --id ID [--cwd DIR]");
    println!("    bullseye sync-priorities ...   Extended: portfolio frontier → SQLite");
    println!("    bullseye github sync ...       Extended: GitHub issues ⇄ targets");
    println!(
        "    bullseye issues-poll ...       Extended: issuepipe Master → targets (feature github-issues)"
    );
    println!(
        "        [--interval SECS]          continuous consumer (0=once; env BULLSEYE_ISSUEPIPE_INTERVAL)"
    );
    println!();
    println!("FLAGS:");
    println!("    --version             Print version");
    println!("    --help                Print this help");
    println!("    --help-agent          Print help and agent guide");
    println!();
    println!("Core contract: docs/api-v1-core.md");
}

fn cli_exit(result: Result<String, String>) -> ! {
    match result {
        Ok(msg) => {
            print!("{msg}");
            if !msg.ends_with('\n') {
                println!();
            }
            process::exit(0);
        }
        Err(e) => {
            eprintln!("{e}");
            process::exit(1);
        }
    }
}

fn tool_result_text(
    result: Result<CallToolResult, rust_mcp_sdk::schema::schema_utils::CallToolError>,
) -> Result<String, String> {
    match result {
        Ok(r) => Ok(text_from_result(r)),
        Err(e) => Err(e.to_string()),
    }
}

fn text_from_result(result: CallToolResult) -> String {
    result
        .content
        .into_iter()
        .map(|block| match block {
            ContentBlock::TextContent(t) => t.text,
            other => format!("{other:?}"),
        })
        .collect::<Vec<_>>()
        .join("")
}

fn flag_value(args: &[String], name: &str) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        if args[i] == name {
            return args.get(i + 1).cloned();
        }
        if let Some(rest) = args[i].strip_prefix(&format!("{name}=")) {
            return Some(rest.to_string());
        }
        i += 1;
    }
    None
}

fn has_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn default_cwd(args: &[String]) -> String {
    flag_value(args, "--cwd").unwrap_or_else(|| {
        std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| ".".to_string())
    })
}

fn cli_open(args: &[String]) -> Result<String, String> {
    if has_flag(args, "--help") {
        return Ok(
            "bullseye open [--cwd DIR] [--location in_repo|external] [--project-name NAME]\n"
                .to_string(),
        );
    }
    tool_result_text(handle_open(OpenTool {
        cwd: default_cwd(args),
        location: flag_value(args, "--location"),
        project_name: flag_value(args, "--project-name"),
        recent_days: flag_value(args, "--recent-days").and_then(|s| s.parse().ok()),
    }))
}

fn cli_query(args: &[String]) -> Result<String, String> {
    if has_flag(args, "--help") {
        return Ok(
            "bullseye query --view VIEW [--cwd DIR] [--id ID] [--filter active|achieved|set_aside|all]\n\
             views: context|frontier|target|list|summary|graph|validate\n"
                .to_string(),
        );
    }
    tool_result_text(handle_query(QueryTool {
        cwd: default_cwd(args),
        view: flag_value(args, "--view"),
        id: flag_value(args, "--id"),
        filter: flag_value(args, "--filter"),
        recent_days: flag_value(args, "--recent-days").and_then(|s| s.parse().ok()),
        momentum: None,
        frontier_details: None,
    }))
}

fn cli_commit(args: &[String]) -> Result<String, String> {
    if has_flag(args, "--help") || args.is_empty() {
        return Ok("bullseye commit --op OP [--cwd DIR] [fields]\n\
             ops: track|block|split|achieve|defer|reopen\n\
             track:  --name NAME --acceptance A [--acceptance A2] [--id ID] [--child-of P]\n\
             block:  --id ID --blocks T1[,T2]\n\
             achieve: --id ID [--actual-cost N]\n\
             defer/reopen: --id ID --reason TEXT\n\
             split: use MCP (children are structured); CLI split not fully wired\n"
            .to_string());
    }
    let op = flag_value(args, "--op").ok_or_else(|| "commit requires --op".to_string())?;
    let mut acceptance = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--acceptance"
            && let Some(v) = args.get(i + 1)
        {
            acceptance.push(v.clone());
            i += 2;
            continue;
        }
        i += 1;
    }
    let blocks = flag_value(args, "--blocks").map(|s| {
        s.split(',')
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
    });
    tool_result_text(handle_commit(CommitTool {
        cwd: default_cwd(args),
        op,
        id: flag_value(args, "--id"),
        child_of: flag_value(args, "--child-of"),
        name: flag_value(args, "--name"),
        value: flag_value(args, "--value").and_then(|s| s.parse().ok()),
        cost: flag_value(args, "--cost").and_then(|s| s.parse().ok()),
        acceptance: if acceptance.is_empty() {
            None
        } else {
            Some(acceptance)
        },
        context: flag_value(args, "--context"),
        status: flag_value(args, "--status"),
        depends_on: flag_value(args, "--depends-on").map(|s| {
            s.split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect()
        }),
        blocks,
        origin: flag_value(args, "--origin"),
        tags: flag_value(args, "--tags").map(|s| {
            s.split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect()
        }),
        actual_cost: flag_value(args, "--actual-cost").and_then(|s| s.parse().ok()),
        reason: flag_value(args, "--reason"),
        parent: flag_value(args, "--parent"),
        mode: flag_value(args, "--mode"),
        children: None,
        retire_reason: flag_value(args, "--retire-reason"),
        tail: None,
        owner: flag_value(args, "--owner"),
    }))
}

fn cli_plan_checks(args: &[String]) -> Result<String, String> {
    if has_flag(args, "--help") {
        return Ok("bullseye plan-checks --id ID [--cwd DIR]\n".to_string());
    }
    let id = flag_value(args, "--id").ok_or_else(|| "plan-checks requires --id".to_string())?;
    tool_result_text(handle_plan_checks(PlanChecksTool {
        cwd: default_cwd(args),
        id,
    }))
}
