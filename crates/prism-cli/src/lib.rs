//! The `prism` CLI library: a hand-rolled argument parser, command handlers
//! wired to [`prism_api`], and a stable [`PrismError`] → exit-code mapping
//! (architecture doc, *CLI Design*).
//!
//! The binary in `main.rs` is a thin shell over [`run`], which parses `argv`,
//! dispatches to a handler, prints a friendly message on failure, and returns
//! the process exit code. Everything is testable without spawning a subprocess:
//! [`args::parse`] is pure, and the handlers take a `&mut dyn Write`.

pub mod args;
pub mod commands;
pub mod exit;
pub mod json;

#[cfg(test)]
mod testsupport;

use std::io::Write;

use args::{Command, ParseError};

/// The version string reported by `prism --version`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The `--help` / `help` text.
pub const HELP: &str = "\
prism — local-first dependency topology analysis

USAGE:
    prism <command> [options]

COMMANDS:
    init [path]                                   create a .prism workspace (default: .)
    ingest <path>                                 ingest a manifest or lockfile
    status                                        summarize runs, packages, graphs
    graph stats --run <id>                        graph size/depth/SCC/duplicates
    graph neighborhood --run <id> --node <id> --depth <n>
                                                  local dependency neighborhood
    graph impact --run <id> --node <id>           reverse dependency impact
    graph duplicates --run <id>                   package families at >1 version
    tda run --run <id> [--filtration depth]       build a filtration + persistence
    tda support --pair <id>                       graph support of a persistence pair
    candidates --requirement <id>                 candidate ranges for a requirement
    candidates --package <purl> --constraint <e>  candidate ranges for a constraint
    conflicts --package <purl>                    same-package conflict pressure
    resolve <manifest> [--policy lowest|highest|locked]
                                                  synthetic resolve a manifest
    export json --run <id>                        emit the graph as JSON
    export graphml --run <id>                      emit the graph as GraphML (XML)

    help, -h, --help                              show this help
    --version, -V                                 show the version
";

/// Parse `args` (without the program name), dispatch, and return the process
/// exit code. Output goes to `out`; errors and usage go to `err`.
pub fn run<I, S>(args: I, out: &mut dyn Write, err: &mut dyn Write) -> i32
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let cmd = match args::parse(args) {
        Ok(c) => c,
        Err(e) => return usage_error(err, &e),
    };

    let result = match &cmd {
        Command::Help => {
            let _ = write!(out, "{HELP}");
            return exit::OK;
        }
        Command::Version => {
            let _ = writeln!(out, "prism {VERSION}");
            return exit::OK;
        }
        Command::Init { path } => commands::init(out, path),
        Command::Ingest { path } => commands::ingest(out, path),
        Command::Status => commands::status(out),
        Command::GraphStats { run } => commands::graph_stats(out, *run),
        Command::GraphNeighborhood { run, node, depth } => {
            commands::graph_neighborhood(out, *run, *node, *depth)
        }
        Command::GraphImpact { run, node } => commands::graph_impact(out, *run, *node),
        Command::GraphDuplicates { run } => commands::graph_duplicates(out, *run),
        Command::GraphScope { run } => commands::graph_scope(out, *run),
        Command::GraphChokepoints { run, limit } => commands::graph_chokepoints(out, *run, *limit),
        Command::GraphWhy { run, node } => commands::graph_why(out, *run, *node),
        Command::GraphCycles { run } => commands::graph_cycles(out, *run),
        Command::GraphDupCauses { run } => commands::graph_dup_causes(out, *run),
        Command::GraphDiff { base, head } => commands::graph_diff(out, *base, *head),
        Command::GraphRisk { run } => commands::graph_risk(out, *run),
        Command::GraphFragmentation { run } => commands::graph_fragmentation(out, *run),
        Command::GraphFidelity { run } => commands::graph_fidelity(out, *run),
        Command::GraphSubsystems { run } => commands::graph_subsystems(out, *run),
        Command::GraphDiamonds { run, limit } => commands::graph_diamonds(out, *run, *limit),
        Command::GraphMinimal { run } => commands::graph_minimal(out, *run),
        Command::GraphOverlay { run, file } => commands::graph_overlay(out, *run, file),
        Command::Regions => commands::regions(out),
        Command::TdaRun { run, filtration } => commands::tda_run(out, *run, filtration),
        Command::TdaSupport { pair } => commands::tda_support(out, *pair),
        Command::Candidates {
            requirement,
            package,
            constraint,
        } => commands::candidates(
            out,
            *requirement,
            package.as_deref(),
            constraint.as_deref(),
        ),
        Command::Conflicts { package } => commands::conflicts(out, package),
        Command::Resolve { manifest, policy } => commands::resolve(out, manifest, policy),
        Command::ExportJson { run } => commands::export_json(out, *run),
        Command::ExportGraphml { run } => commands::export_graphml(out, *run),
    };

    match result {
        Ok(()) => exit::OK,
        Err(e) => {
            let _ = writeln!(err, "prism: {}", exit::message_for(&e));
            exit::code_for(&e)
        }
    }
}

/// Print a usage error and the help hint, returning the `USAGE` code.
fn usage_error(err: &mut dyn Write, e: &ParseError) -> i32 {
    let _ = writeln!(err, "prism: {e}");
    let _ = writeln!(err, "try `prism help`");
    exit::USAGE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drive(args: &[&str]) -> (i32, String, String) {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run(args.iter().copied(), &mut out, &mut err);
        (
            code,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    #[test]
    fn help_is_ok_and_prints_usage() {
        let (code, out, _) = drive(&["help"]);
        assert_eq!(code, exit::OK);
        assert!(out.contains("USAGE"));
        assert!(out.contains("graph stats"));
    }

    #[test]
    fn version_is_ok() {
        let (code, out, _) = drive(&["--version"]);
        assert_eq!(code, exit::OK);
        assert!(out.contains(VERSION));
    }

    #[test]
    fn unknown_command_is_usage_error() {
        let (code, _out, err) = drive(&["frobnicate"]);
        assert_eq!(code, exit::USAGE);
        assert!(err.contains("unknown command"));
        assert!(err.contains("try `prism help`"));
    }

    #[test]
    fn missing_flag_is_usage_error() {
        let (code, _out, err) = drive(&["graph", "stats"]);
        assert_eq!(code, exit::USAGE);
        assert!(err.contains("missing argument"));
    }

    #[test]
    fn no_args_is_usage_error() {
        let (code, _out, err) = drive(&[]);
        assert_eq!(code, exit::USAGE);
        assert!(err.contains("no command"));
    }
}
