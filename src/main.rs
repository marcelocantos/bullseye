// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use std::process;

use bullseye::handler::{
    TargetHandler, handle_commit, handle_convergence, handle_import, handle_open,
    handle_plan_checks, handle_portfolio, handle_query, handle_resolve,
};
use bullseye::tools::{
    CommitTool, ConvergenceTool, ImportTool, MomentumEntry, OpenTool, PlanChecksTool,
    PortfolioTool, QueryTool, ResolveTool,
};
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
    // Server create default (🎯T61) may appear as a global flag before any
    // subcommand, or alone when starting MCP: `bullseye --default-location external`.
    let (server_flags, rest) = split_server_flags(&args[1..]);
    apply_server_flags(&server_flags);

    if !rest.is_empty() {
        match rest[0].as_str() {
            "--version" => {
                println!("bullseye {}", bullseye::version::VERSION);
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
            "open" => cli_exit(cli_open(&rest[1..])),
            "query" => cli_exit(cli_query(&rest[1..])),
            "commit" => cli_exit(cli_commit(&rest[1..])),
            "apply" => cli_exit(cli_apply(&rest[1..])),
            "plan-checks" => cli_exit(cli_plan_checks(&rest[1..])),
            "convergence" => cli_exit(cli_convergence(&rest[1..])),
            "portfolio" => cli_exit(cli_portfolio(&rest[1..])),
            "import" => cli_exit(cli_import(&rest[1..])),
            "resolve" => cli_exit(cli_resolve(&rest[1..])),
            "sync-priorities" => {
                #[cfg(feature = "sqlite")]
                match bullseye::priorities::run_sync(&rest[1..]) {
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
            "github" => match bullseye::github::run(&rest[1..]) {
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
                match bullseye::github_issues::http::run(&rest[1..]) {
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
            // A leading `-` is the only thing that distinguishes a mistyped
            // flag from a mistyped subcommand, and the two send the reader
            // looking in different places (🎯T67).
            other if other.starts_with('-') => {
                eprintln!("unknown flag: {other}");
                eprintln!();
                print_help();
                process::exit(1);
            }
            other => {
                eprintln!("unknown subcommand: {other}");
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

/// Peel global server flags from argv. Recognises
/// `--default-location VALUE` and `--default-location=VALUE` (🎯T61).
fn split_server_flags(args: &[String]) -> (Vec<String>, Vec<String>) {
    let mut flags = Vec::new();
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--default-location" {
            flags.push(a.clone());
            if let Some(v) = args.get(i + 1) {
                flags.push(v.clone());
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }
        if let Some(rest_eq) = a.strip_prefix("--default-location=") {
            flags.push("--default-location".to_string());
            flags.push(rest_eq.to_string());
            i += 1;
            continue;
        }
        // First non-server-flag token starts the subcommand / remaining args.
        rest.extend_from_slice(&args[i..]);
        break;
    }
    (flags, rest)
}

fn apply_server_flags(flags: &[String]) {
    if let Some(val) = flag_value(flags, "--default-location")
        && let Err(e) = bullseye::config::set_process_default_location(&val)
    {
        eprintln!("--default-location: {e}");
        process::exit(1);
    }
}

fn print_help() {
    println!(
        "bullseye {} — Intent Ledger MCP Server",
        bullseye::version::VERSION
    );
    println!();
    println!("USAGE:");
    println!("    bullseye [--default-location in_repo|external]");
    println!("                               Start the MCP server (stdio transport)");
    println!("    bullseye open [--cwd DIR] [--location in_repo|external]");
    println!("    bullseye query --view VIEW [--cwd DIR] [--id ID] [--filter F]");
    println!("    bullseye apply [--cwd DIR] (-f FILE | - | --id ID --set k=v ...)");
    println!("                               The single write verb — see bullseye apply --help");
    println!("    bullseye commit --op OP [flags]   # sugar over apply; see commit --help");
    println!("    bullseye plan-checks --id ID [--cwd DIR]");
    println!(
        "    bullseye convergence [--cwd DIR] [--skip-invariants] [--momentum ID=MULT,...]\n\
         \x20                              Extended: invariants + frontier + recommendation"
    );
    println!(
        "    bullseye portfolio [--root DIR] [--max-depth N] [--momentum ID=MULT,...]\n\
         \x20                              Extended: cross-repo WSJF ranking"
    );
    println!(
        "    bullseye import --path FILE.md [--cwd DIR] [--location L] [--force]\n\
         \x20                              Extended: markdown targets → bullseye.yaml"
    );
    println!(
        "    bullseye resolve --reference REF [--workspace-root DIR]\n\
         \x20                              Extended: repo reference → absolute path"
    );
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
    println!(
        "    --default-location L  Create default when location omitted (in_repo|external; 🎯T61)"
    );
    println!(
        "                           Also: env BULLSEYE_DEFAULT_LOCATION. Discovery unchanged."
    );
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
             views: context|frontier|target|list|summary|graph|validate\n\
             graph (🎯T57): [--scope active|all|achieved|set_aside] [--nodes ID,ID]\n\
                           [--seeds ID,ID] [--expand ancestors,descendants,children,parents,frontier]\n\
               default: whole active graph (depends_on edges). nodes= explicit set;\n\
               seeds+expand= intelligent neighborhood. Disjoint components OK.\n"
                .to_string(),
        );
    }
    let nodes = flag_value(args, "--nodes").map(|s| {
        s.split(',')
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
    });
    let seeds = flag_value(args, "--seeds").map(|s| {
        s.split(',')
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
    });
    let expand = flag_value(args, "--expand").map(|s| {
        s.split(|c: char| c == ',' || c.is_whitespace())
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
    });
    tool_result_text(handle_query(QueryTool {
        cwd: default_cwd(args),
        view: flag_value(args, "--view"),
        id: flag_value(args, "--id"),
        filter: flag_value(args, "--filter"),
        recent_days: flag_value(args, "--recent-days").and_then(|s| s.parse().ok()),
        momentum: None,
        frontier_details: None,
        scope: flag_value(args, "--scope"),
        nodes,
        seeds,
        expand,
    }))
}

/// Flags `bullseye commit` accepts, and whether each takes a value.
/// Declared for the same reason as [`APPLY_FLAGS`]: an unrecognised
/// flag must fail rather than return `ok: true` (🎯T76).
const COMMIT_FLAGS: &[(&str, bool)] = &[
    ("--cwd", true),
    ("--op", true),
    ("--id", true),
    ("--child-of", true),
    ("--name", true),
    ("--value", true),
    ("--cost", true),
    ("--acceptance", true),
    ("--context", true),
    ("--status", true),
    ("--depends-on", true),
    ("--blocks", true),
    ("--origin", true),
    ("--tags", true),
    ("--actual-cost", true),
    ("--attestation", true),
    ("--reason", true),
    ("--postponed-until", true),
    ("--postpone-predicate", true),
    ("--parent", true),
    ("--mode", true),
    ("--children-file", true),
    ("--retire-reason", true),
    ("--tail", true),
    ("--help", false),
];

fn cli_commit(args: &[String]) -> Result<String, String> {
    if has_flag(args, "--help") || args.is_empty() {
        return Ok("bullseye commit --op OP [--cwd DIR] [fields]\n\
             ops: track|block|split|achieve|defer|reopen|assign|unassign|postpone|wake|rehash\n\
             track:  --name NAME --acceptance A [--acceptance A2] [--id ID] [--child-of P]\n\
             block:  --id ID --blocks T1[,T2]\n\
             achieve: --id ID --attestation TEXT [--actual-cost N]\n\
             defer/reopen/rehash: --id ID --reason TEXT (rehash: reason only)\n\
             postpone: --id ID [--postponed-until YYYY-MM-DD] [--postpone-predicate TEXT]\n\
             wake: --id ID\n\
             assign: --id ID --owner HANDLE --reason TEXT\n\
             split: --parent P --mode add|aggregate|retire --children-file F|- \n\
             \x20      (children: YAML list of {name, acceptance[, id, context, tags, depends_on]})\n\
             \x20      [--retire-reason TEXT] [--tail T1,T2]\n\
             \n\
             Every op above is sugar over `bullseye apply`, which reaches the same\n\
             fields directly and in bulk. See `bullseye apply --help`.\n"
            .to_string());
    }
    reject_unknown_flags(args, COMMIT_FLAGS, "commit")?;
    let op = flag_value(args, "--op").ok_or_else(|| "commit requires --op".to_string())?;

    // 🎯T76: `split` children are structured, which is why the CLI
    // route used to send callers to MCP. They are read here as a YAML
    // list from a file or stdin, so split is reachable from both
    // surfaces like every other capability.
    let children = match flag_value(args, "--children-file") {
        Some(src) => {
            let text = if src == "-" {
                let mut buf = String::new();
                std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)
                    .map_err(|e| format!("commit: cannot read stdin: {e}"))?;
                buf
            } else {
                std::fs::read_to_string(&src)
                    .map_err(|e| format!("commit: cannot read {src}: {e}"))?
            };
            let parsed: Vec<bullseye::tools::SubdivisionChild> = serde_yaml_ng::from_str(&text)
                .map_err(|e| {
                    format!(
                        "commit: --children-file must be a YAML list of \
                         {{name, acceptance[, id, context, tags, depends_on]}}: {e}"
                    )
                })?;
            Some(parsed)
        }
        None => None,
    };
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
        attestation: flag_value(args, "--attestation"),
        reason: flag_value(args, "--reason"),
        postponed_until: flag_value(args, "--postponed-until"),
        postpone_predicate: flag_value(args, "--postpone-predicate"),
        parent: flag_value(args, "--parent"),
        mode: flag_value(args, "--mode"),
        children,
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

/// `--momentum T1=1.5,T2=0.5` → the wire shape the tools take.
fn momentum_entries(args: &[String]) -> Result<Option<Vec<MomentumEntry>>, String> {
    let Some(raw) = flag_value(args, "--momentum") else {
        return Ok(None);
    };
    let mut out = Vec::new();
    for part in raw.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (id, mult) = part
            .split_once('=')
            .ok_or_else(|| format!("--momentum: expected ID=MULTIPLIER, got {part:?}"))?;
        let multiplier: f64 = mult
            .trim()
            .parse()
            .map_err(|_| format!("--momentum: {mult:?} is not a number"))?;
        out.push(MomentumEntry {
            id: id.trim().to_string(),
            multiplier,
        });
    }
    Ok(if out.is_empty() { None } else { Some(out) })
}

/// A `--cwd` that does not exist is an operator error, not an empty
/// ledger — the shared handlers report the latter as an ordinary result,
/// so scripts driving these subcommands need the distinction up front.
fn require_dir(cwd: &str) -> Result<(), String> {
    if std::path::Path::new(cwd).is_dir() {
        return Ok(());
    }
    Err(format!("--cwd {cwd}: not a directory"))
}

fn cli_convergence(args: &[String]) -> Result<String, String> {
    if has_flag(args, "--help") {
        return Ok("bullseye convergence [--cwd DIR] [--skip-invariants] \
             [--momentum ID=MULT,...]\n\
             Runs the project's `make bullseye` invariants (unless --skip-invariants),\n\
             scans for unreleased fixes, and prints the target summary plus a\n\
             next-action recommendation. Exits non-zero on error.\n"
            .to_string());
    }
    let cwd = default_cwd(args);
    require_dir(&cwd)?;
    tool_result_text(handle_convergence(ConvergenceTool {
        cwd,
        momentum: momentum_entries(args)?,
        skip_invariants: Some(has_flag(args, "--skip-invariants")),
    }))
}

fn cli_portfolio(args: &[String]) -> Result<String, String> {
    if has_flag(args, "--help") {
        return Ok(
            "bullseye portfolio [--root DIR] [--max-depth N] [--momentum ID=MULT,...]\n\
             Scans the workspace root (default ~/work) for repos with targets and\n\
             ranks them by aggregate WSJF score.\n"
                .to_string(),
        );
    }
    tool_result_text(handle_portfolio(PortfolioTool {
        root: flag_value(args, "--root"),
        max_depth: flag_value(args, "--max-depth").and_then(|s| s.parse().ok()),
        momentum: momentum_entries(args)?,
    }))
}

fn cli_import(args: &[String]) -> Result<String, String> {
    if has_flag(args, "--help") {
        return Ok(
            "bullseye import --path FILE.md [--cwd DIR] [--location in_repo|external] [--force]\n\
             Imports targets from a /cv-style markdown file into bullseye.yaml.\n\
             Refuses to overwrite an existing bullseye.yaml without --force.\n"
                .to_string(),
        );
    }
    let cwd = default_cwd(args);
    require_dir(&cwd)?;
    tool_result_text(handle_import(ImportTool {
        cwd,
        path: flag_value(args, "--path"),
        location: flag_value(args, "--location"),
        force: has_flag(args, "--force"),
    }))
}

fn cli_resolve(args: &[String]) -> Result<String, String> {
    if has_flag(args, "--help") {
        return Ok("bullseye resolve --reference REF [--workspace-root DIR]\n\
             Resolves a leaf name, org/repo fragment, or absolute path to a repo root.\n\
             Ambiguous or unmatched references exit non-zero.\n"
            .to_string());
    }
    let reference = flag_value(args, "--reference")
        .or_else(|| args.first().filter(|a| !a.starts_with('-')).cloned())
        .ok_or_else(|| "resolve requires --reference REF".to_string())?;
    tool_result_text(handle_resolve(ResolveTool {
        reference,
        workspace_root: flag_value(args, "--workspace-root"),
    }))
}

/// Flags `bullseye apply` accepts, and whether each takes a value.
///
/// Declared rather than discovered so an unrecognised flag can be an
/// error. The old hand-rolled parsing accepted anything and returned
/// `ok: true`, which meant a typo'd flag looked like a successful
/// write — a documented reason agents gave up on the CLI and edited
/// the YAML by hand (🎯T76).
const APPLY_FLAGS: &[(&str, bool)] = &[
    ("--cwd", true),
    ("--file", true),
    ("-f", true),
    ("--id", true),
    ("--set", true),
    ("--base", true),
    ("--remove", true),
    ("--reason", true),
    ("--help", false),
];

/// Reject any flag the subcommand does not declare.
fn reject_unknown_flags(
    args: &[String],
    spec: &[(&str, bool)],
    subcommand: &str,
) -> Result<(), String> {
    let mut i = 0;
    while i < args.len() {
        let tok = &args[i];
        // A bare "-" means stdin; anything else starting with "-" is a
        // flag we must recognise. Values are skipped explicitly below,
        // so a value that happens to look like a flag is not misread.
        if tok == "-" || !tok.starts_with('-') {
            i += 1;
            continue;
        }
        let (name, inline_value) = match tok.split_once('=') {
            Some((n, _)) => (n, true),
            None => (tok.as_str(), false),
        };
        match spec.iter().find(|(f, _)| *f == name) {
            Some((_, takes_value)) => {
                i += if *takes_value && !inline_value { 2 } else { 1 };
            }
            None => {
                let accepted = spec.iter().map(|(f, _)| *f).collect::<Vec<_>>().join(", ");
                return Err(format!(
                    "{subcommand}: unrecognised flag `{name}`.\nAccepted flags: {accepted}\n\
                     Run `bullseye {subcommand} --help` for the full field list."
                ));
            }
        }
    }
    Ok(())
}

/// All values given for a repeatable flag, in order.
fn flag_values(args: &[String], name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == name {
            if let Some(v) = args.get(i + 1) {
                out.push(v.clone());
            }
            i += 2;
            continue;
        }
        if let Some(rest) = args[i].strip_prefix(&format!("{name}=")) {
            out.push(rest.to_string());
        }
        i += 1;
    }
    out
}

/// `bullseye apply` help, with the field list generated from
/// [`bullseye::apply::FIELD_HELP`] so the documented surface cannot
/// drift from the schema (🎯T76).
fn apply_help() -> String {
    let mut s = String::from(
        "bullseye apply [--cwd DIR] (-f FILE | - | --id ID --set k=v ...)\n\
         \n\
         Applies a partial desired-state fragment. Fields you do not mention are\n\
         left alone, and targets you do not mention are never removed — removal is\n\
         explicit via `remove:` (fragment) or --remove ID[,ID] (flags).\n\
         \n\
         Fragment form (YAML on stdin or -f FILE):\n\
         \x20 base: sha256:…            # optional CAS token; mismatch = code=conflict\n\
         \x20 targets:\n\
         \x20   T55: {value: 8}         # patch an existing target\n\
         \x20   _new: {name: …, acceptance: [ … ]}   # `_` prefix allocates an ID\n\
         \x20 remove: [T99]\n\
         \n\
         Flag form:\n\
         \x20 bullseye apply --id T55 --set value=8 --set cost=5\n\
         \n\
         Fields:\n",
    );
    for f in bullseye::apply::FIELD_HELP {
        s.push_str(&format!("\x20 {:<19} {}\n", f.name, f.blurb));
    }
    s.push_str("\nEvidence required by transition:\n");
    for ob in bullseye::apply::POLICY {
        s.push_str(&format!(
            "\x20 {:<28} requires `{}`\n",
            ob.transition, ob.requires
        ));
    }
    s
}

/// Build a one-target request from `--id` plus repeated `--set k=v`.
///
/// Values are typed by field rather than by parsing each as YAML:
/// target prose routinely contains colons and commas, and a
/// YAML-parsed `--set name="T5: do the thing"` would silently become a
/// map. Unknown keys still fail, because the assembled mapping is
/// deserialized into `Fragment`, which denies unknown fields.
fn fragment_from_sets(sets: &[String]) -> Result<bullseye::apply::Fragment, String> {
    use serde_yaml_ng::Value;
    let mut map = serde_yaml_ng::Mapping::new();
    let mut acceptance: Vec<Value> = Vec::new();
    for pair in sets {
        let (key, value) = pair
            .split_once('=')
            .ok_or_else(|| format!("--set expects key=value, got `{pair}`"))?;
        match key {
            "value" | "cost" | "actual_cost" => {
                let n: f64 = value
                    .parse()
                    .map_err(|_| format!("--set {key}={value}: expected a number"))?;
                map.insert(Value::from(key), Value::from(n));
            }
            // Repeatable: each --set acceptance=… appends one criterion,
            // because criteria contain commas and must not be split.
            "acceptance" => acceptance.push(Value::from(value)),
            "tags" | "depends_on" | "blocks" => {
                let items: Vec<Value> = value
                    .split(',')
                    .map(|p| p.trim())
                    .filter(|p| !p.is_empty())
                    .map(Value::from)
                    .collect();
                map.insert(Value::from(key), Value::Sequence(items));
            }
            _ => {
                map.insert(Value::from(key), Value::from(value));
            }
        }
    }
    if !acceptance.is_empty() {
        map.insert(Value::from("acceptance"), Value::Sequence(acceptance));
    }
    // serde's own error already enumerates the legal fields.
    serde_yaml_ng::from_value(Value::Mapping(map)).map_err(|e| e.to_string())
}

fn cli_apply(args: &[String]) -> Result<String, String> {
    if has_flag(args, "--help") || args.is_empty() {
        return Ok(apply_help());
    }
    reject_unknown_flags(args, APPLY_FLAGS, "apply")?;

    let from_file = flag_value(args, "-f").or_else(|| flag_value(args, "--file"));
    let from_stdin = args.iter().any(|a| a == "-");

    let mut req: bullseye::apply::ApplyRequest = if let Some(path) = from_file {
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("apply: cannot read {path}: {e}"))?;
        serde_yaml_ng::from_str(&text).map_err(|e| format!("apply: {path}: {e}"))?
    } else if from_stdin {
        let mut text = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)
            .map_err(|e| format!("apply: cannot read stdin: {e}"))?;
        serde_yaml_ng::from_str(&text).map_err(|e| format!("apply: stdin: {e}"))?
    } else {
        let id = flag_value(args, "--id").ok_or_else(|| {
            "apply: give either a fragment (-f FILE or -) or --id ID with --set k=v".to_string()
        })?;
        let frag = fragment_from_sets(&flag_values(args, "--set"))?;
        let mut targets = std::collections::BTreeMap::new();
        targets.insert(id, frag);
        bullseye::apply::ApplyRequest {
            targets,
            ..Default::default()
        }
    };

    // Flags layer over a fragment so `-f frag.yaml --base <hash>` works.
    if let Some(base) = flag_value(args, "--base") {
        req.base = Some(base);
    }
    if let Some(reason) = flag_value(args, "--reason") {
        req.reason = Some(reason);
    }
    if let Some(remove) = flag_value(args, "--remove") {
        req.remove.extend(
            remove
                .split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty()),
        );
    }

    tool_result_text(bullseye::handler::apply_request(&default_cwd(args), req))
}
