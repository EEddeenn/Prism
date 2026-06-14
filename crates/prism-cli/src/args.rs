//! Hand-rolled argument parser (no external crates).
//!
//! Parses `argv` (sans program name) into a [`Command`]. Flags are `--name value`
//! (or `--name=value`); typed flags (`--run`, `--node`, `--depth`, `--pair`) are
//! parsed as `u64`/`u32`. A malformed invocation returns a [`ParseError`] the
//! caller prints and maps to the `USAGE` exit code; `help`/`--version` short-
//! circuit to their own [`Command`] variants.

use std::fmt;

/// A fully-parsed CLI invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// `prism init [path]` — create a workspace (default `.`).
    Init { path: String },
    /// `prism ingest <path>` — ingest a manifest or lockfile.
    Ingest { path: String },
    /// `prism status` — workspace summary.
    Status,
    /// `prism graph stats --run <id>`.
    GraphStats { run: u64 },
    /// `prism graph neighborhood --run <id> --node <id> --depth <n>`.
    GraphNeighborhood { run: u64, node: u32, depth: u32 },
    /// `prism graph impact --run <id> --node <id>`.
    GraphImpact { run: u64, node: u32 },
    /// `prism graph duplicates --run <id>`.
    GraphDuplicates { run: u64 },
    /// `prism graph scope --run <id>` — per-scope footprint summary.
    GraphScope { run: u64 },
    /// `prism graph chokepoints --run <id> [--limit <n>]` — structural gateways.
    GraphChokepoints { run: u64, limit: u32 },
    /// `prism graph why --run <id> --node <id>` — why a dependency is present.
    GraphWhy { run: u64, node: u32 },
    /// `prism graph cycles --run <id>` — classified dependency cycles.
    GraphCycles { run: u64 },
    /// `prism graph dup-causes --run <id>` — duplicate-version root cause.
    GraphDupCauses { run: u64 },
    /// `prism graph diff --base <id> --head <id>` — interpretive cross-run diff.
    GraphDiff { base: u64, head: u64 },
    /// `prism graph risk --run <id>` — structural risk profile.
    GraphRisk { run: u64 },
    /// `prism graph fragmentation --run <id>` — connected-component breakdown.
    GraphFragmentation { run: u64 },
    /// `prism graph fidelity --run <id>` — per-metric fidelity notes.
    GraphFidelity { run: u64 },
    /// `prism graph subsystems --run <id>` — top-level subsystem partition.
    GraphSubsystems { run: u64 },
    /// `prism graph diamonds --run <id> [--limit <n>]` — diamond-reuse foundations.
    GraphDiamonds { run: u64, limit: u32 },
    /// `prism graph minimal --run <id>` — minimal (dominator-tree) explanation.
    GraphMinimal { run: u64 },
    /// `prism graph overlay --run <id> --file <path>` — join a risk overlay.
    GraphOverlay { run: u64, file: String },
    /// `prism graph distance --base <id> --head <id>` — topology shape distance.
    GraphDistance { base: u64, head: u64 },
    /// `prism graph budget --head <id> [--base <id>] --file <path>` — CI gate.
    GraphBudget { head: u64, base: Option<u64>, file: String },
    /// `prism graph shared --run <id> [--limit <n>]` — higher-order overlap.
    GraphShared { run: u64, limit: u32 },
    /// `prism regions` — every run as a comparable dependency region.
    Regions,
    /// `prism tda run --run <id> [--filtration depth]`.
    TdaRun { run: u64, filtration: String },
    /// `prism tda support --pair <id>`.
    TdaSupport { pair: u64 },
    /// `prism tda compare --run <id>` — topology under multiple filtration lenses.
    TdaCompare { run: u64 },
    /// `prism candidates --requirement <id>` or
    /// `prism candidates --package <purl> --constraint <expr>`.
    Candidates {
        /// A stored requirement id, or `None` when `--package`/`--constraint`
        /// were given instead.
        requirement: Option<u64>,
        /// A package purl, paired with `constraint`.
        package: Option<String>,
        /// A raw constraint expression, paired with `package`.
        constraint: Option<String>,
    },
    /// `prism conflicts --package <purl>`.
    Conflicts { package: String },
    /// `prism resolve <manifest-path> --policy <lowest|highest|locked>`.
    Resolve { manifest: String, policy: String },
    /// `prism export json --run <id>`.
    ExportJson { run: u64 },
    /// `prism export graphml --run <id>`.
    ExportGraphml { run: u64 },
    /// `prism help` / `-h` / `--help`.
    Help,
    /// `prism --version` / `-V`.
    Version,
}

/// Why an invocation could not be parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// No subcommand was given at all.
    NoCommand,
    /// The subcommand (or sub-subcommand) is not recognized.
    UnknownCommand(String),
    /// A required positional/flag argument was missing.
    MissingArg {
        /// The command/flag whose argument is missing.
        what: String,
    },
    /// A flag's value did not parse as the expected type.
    BadValue {
        /// The flag name.
        flag: String,
        /// The offending value.
        value: String,
    },
    /// An unexpected flag/positional for this command.
    Unexpected(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::NoCommand => write!(f, "no command given (try `prism help`)"),
            ParseError::UnknownCommand(c) => write!(f, "unknown command: {c}"),
            ParseError::MissingArg { what } => write!(f, "missing argument: {what}"),
            ParseError::BadValue { flag, value } => {
                write!(f, "invalid value for {flag}: {value:?}")
            }
            ParseError::Unexpected(a) => write!(f, "unexpected argument: {a}"),
        }
    }
}

/// Parse `args` (program name already removed) into a [`Command`].
pub fn parse<I, S>(args: I) -> Result<Command, ParseError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args: Vec<String> = args.into_iter().map(Into::into).collect();
    let mut it = args.iter().map(String::as_str);
    let first = match it.next() {
        Some(a) => a,
        None => return Err(ParseError::NoCommand),
    };
    match first {
        "help" | "-h" | "--help" => Ok(Command::Help),
        "--version" | "-V" => Ok(Command::Version),
        "init" => {
            // Optional positional path; default to the current directory.
            let path = it.next().map(str::to_string).unwrap_or_else(|| ".".into());
            reject_trailing(it)?;
            Ok(Command::Init { path })
        }
        "ingest" => {
            let path = it
                .next()
                .map(str::to_string)
                .ok_or_else(|| ParseError::MissingArg { what: "<path>".into() })?;
            reject_trailing(it)?;
            Ok(Command::Ingest { path })
        }
        "status" => {
            reject_trailing(it)?;
            Ok(Command::Status)
        }
        "regions" => {
            reject_trailing(it)?;
            Ok(Command::Regions)
        }
        "graph" => parse_graph(it),
        "tda" => parse_tda(it),
        "candidates" => parse_candidates(it),
        "conflicts" => parse_conflicts(it),
        "resolve" => parse_resolve(it),
        "export" => parse_export(it),
        other => Err(ParseError::UnknownCommand(other.to_string())),
    }
}

/// `prism candidates --requirement <id>` | `--package <purl> --constraint <expr>`.
fn parse_candidates<'a, I: Iterator<Item = &'a str>>(it: I) -> Result<Command, ParseError> {
    let mut flags = Flags::parse(it)?;
    let requirement = flags.take_u64_opt("--requirement")?;
    let package = flags.take_opt("--package");
    let constraint = flags.take_opt("--constraint");
    flags.finish()?;

    match (requirement, &package, &constraint) {
        // Requirement form: nothing else may be present.
        (Some(id), None, None) => Ok(Command::Candidates {
            requirement: Some(id),
            package: None,
            constraint: None,
        }),
        // Package + constraint form.
        (None, Some(_), Some(_)) => Ok(Command::Candidates {
            requirement: None,
            package,
            constraint,
        }),
        // Mixing the two forms, or supplying neither, is a usage error.
        (None, None, None) => Err(ParseError::MissingArg {
            what: "--requirement, or --package and --constraint".into(),
        }),
        (Some(_), _, _) => Err(ParseError::Unexpected(
            "--requirement cannot be combined with --package/--constraint".into(),
        )),
        (None, Some(_), None) => Err(ParseError::MissingArg { what: "--constraint".into() }),
        (None, None, Some(_)) => Err(ParseError::MissingArg { what: "--package".into() }),
    }
}

/// `prism conflicts --package <purl>`.
fn parse_conflicts<'a, I: Iterator<Item = &'a str>>(it: I) -> Result<Command, ParseError> {
    let mut flags = Flags::parse(it)?;
    let package = flags
        .take_opt("--package")
        .ok_or_else(|| ParseError::MissingArg { what: "--package".into() })?;
    flags.finish()?;
    Ok(Command::Conflicts { package })
}

/// `prism resolve <manifest-path> --policy <lowest|highest|locked>`.
fn parse_resolve<'a, I: Iterator<Item = &'a str>>(mut it: I) -> Result<Command, ParseError> {
    let manifest = it
        .next()
        .map(str::to_string)
        .ok_or_else(|| ParseError::MissingArg { what: "<manifest-path>".into() })?;
    let mut flags = Flags::parse(it)?;
    // Default to the highest-non-prerelease policy when unspecified.
    let policy = flags
        .take_opt("--policy")
        .unwrap_or_else(|| "highest".to_string());
    flags.finish()?;
    match policy.as_str() {
        "lowest" | "highest" | "locked" => Ok(Command::Resolve { manifest, policy }),
        other => Err(ParseError::BadValue {
            flag: "--policy".into(),
            value: other.to_string(),
        }),
    }
}

fn parse_graph<'a, I: Iterator<Item = &'a str>>(mut it: I) -> Result<Command, ParseError> {
    let sub = it
        .next()
        .ok_or_else(|| ParseError::MissingArg { what: "graph <subcommand>".into() })?;
    match sub {
        "stats" => {
            let mut flags = Flags::parse(it)?;
            let run = flags.take_u64("--run")?;
            flags.finish()?;
            Ok(Command::GraphStats { run })
        }
        "neighborhood" => {
            let mut flags = Flags::parse(it)?;
            let run = flags.take_u64("--run")?;
            let node = flags.take_u32("--node")?;
            let depth = flags.take_u32("--depth")?;
            flags.finish()?;
            Ok(Command::GraphNeighborhood { run, node, depth })
        }
        "impact" => {
            let mut flags = Flags::parse(it)?;
            let run = flags.take_u64("--run")?;
            let node = flags.take_u32("--node")?;
            flags.finish()?;
            Ok(Command::GraphImpact { run, node })
        }
        "duplicates" => {
            let mut flags = Flags::parse(it)?;
            let run = flags.take_u64("--run")?;
            flags.finish()?;
            Ok(Command::GraphDuplicates { run })
        }
        "scope" => {
            let mut flags = Flags::parse(it)?;
            let run = flags.take_u64("--run")?;
            flags.finish()?;
            Ok(Command::GraphScope { run })
        }
        "chokepoints" => {
            let mut flags = Flags::parse(it)?;
            let run = flags.take_u64("--run")?;
            let limit = match flags.take_opt("--limit") {
                Some(s) => s.parse::<u32>().map_err(|_| ParseError::BadValue {
                    flag: "--limit".into(),
                    value: s,
                })?,
                None => 20,
            };
            flags.finish()?;
            Ok(Command::GraphChokepoints { run, limit })
        }
        "why" => {
            let mut flags = Flags::parse(it)?;
            let run = flags.take_u64("--run")?;
            let node = flags.take_u32("--node")?;
            flags.finish()?;
            Ok(Command::GraphWhy { run, node })
        }
        "cycles" => {
            let mut flags = Flags::parse(it)?;
            let run = flags.take_u64("--run")?;
            flags.finish()?;
            Ok(Command::GraphCycles { run })
        }
        "dup-causes" => {
            let mut flags = Flags::parse(it)?;
            let run = flags.take_u64("--run")?;
            flags.finish()?;
            Ok(Command::GraphDupCauses { run })
        }
        "diff" => {
            let mut flags = Flags::parse(it)?;
            let base = flags.take_u64("--base")?;
            let head = flags.take_u64("--head")?;
            flags.finish()?;
            Ok(Command::GraphDiff { base, head })
        }
        "risk" => {
            let mut flags = Flags::parse(it)?;
            let run = flags.take_u64("--run")?;
            flags.finish()?;
            Ok(Command::GraphRisk { run })
        }
        "fragmentation" => {
            let mut flags = Flags::parse(it)?;
            let run = flags.take_u64("--run")?;
            flags.finish()?;
            Ok(Command::GraphFragmentation { run })
        }
        "fidelity" => {
            let mut flags = Flags::parse(it)?;
            let run = flags.take_u64("--run")?;
            flags.finish()?;
            Ok(Command::GraphFidelity { run })
        }
        "subsystems" => {
            let mut flags = Flags::parse(it)?;
            let run = flags.take_u64("--run")?;
            flags.finish()?;
            Ok(Command::GraphSubsystems { run })
        }
        "diamonds" => {
            let mut flags = Flags::parse(it)?;
            let run = flags.take_u64("--run")?;
            let limit = match flags.take_opt("--limit") {
                Some(s) => s.parse::<u32>().map_err(|_| ParseError::BadValue {
                    flag: "--limit".into(),
                    value: s,
                })?,
                None => 15,
            };
            flags.finish()?;
            Ok(Command::GraphDiamonds { run, limit })
        }
        "minimal" => {
            let mut flags = Flags::parse(it)?;
            let run = flags.take_u64("--run")?;
            flags.finish()?;
            Ok(Command::GraphMinimal { run })
        }
        "overlay" => {
            let mut flags = Flags::parse(it)?;
            let run = flags.take_u64("--run")?;
            let file = flags
                .take_opt("--file")
                .ok_or_else(|| ParseError::MissingArg { what: "--file".into() })?;
            flags.finish()?;
            Ok(Command::GraphOverlay { run, file })
        }
        "distance" => {
            let mut flags = Flags::parse(it)?;
            let base = flags.take_u64("--base")?;
            let head = flags.take_u64("--head")?;
            flags.finish()?;
            Ok(Command::GraphDistance { base, head })
        }
        "budget" => {
            let mut flags = Flags::parse(it)?;
            let head = flags.take_u64("--head")?;
            let base = match flags.take_opt("--base") {
                Some(s) => Some(s.parse::<u64>().map_err(|_| ParseError::BadValue {
                    flag: "--base".into(),
                    value: s,
                })?),
                None => None,
            };
            let file = flags
                .take_opt("--file")
                .ok_or_else(|| ParseError::MissingArg { what: "--file".into() })?;
            flags.finish()?;
            Ok(Command::GraphBudget { head, base, file })
        }
        "shared" => {
            let mut flags = Flags::parse(it)?;
            let run = flags.take_u64("--run")?;
            let limit = match flags.take_opt("--limit") {
                Some(s) => s.parse::<u32>().map_err(|_| ParseError::BadValue {
                    flag: "--limit".into(),
                    value: s,
                })?,
                None => 15,
            };
            flags.finish()?;
            Ok(Command::GraphShared { run, limit })
        }
        other => Err(ParseError::UnknownCommand(format!("graph {other}"))),
    }
}

fn parse_tda<'a, I: Iterator<Item = &'a str>>(mut it: I) -> Result<Command, ParseError> {
    let sub = it
        .next()
        .ok_or_else(|| ParseError::MissingArg { what: "tda <subcommand>".into() })?;
    match sub {
        "run" => {
            let mut flags = Flags::parse(it)?;
            let run = flags.take_u64("--run")?;
            let filtration = flags
                .take_opt("--filtration")
                .unwrap_or_else(|| "depth".to_string());
            flags.finish()?;
            Ok(Command::TdaRun { run, filtration })
        }
        "support" => {
            let mut flags = Flags::parse(it)?;
            let pair = flags.take_u64("--pair")?;
            flags.finish()?;
            Ok(Command::TdaSupport { pair })
        }
        "compare" => {
            let mut flags = Flags::parse(it)?;
            let run = flags.take_u64("--run")?;
            flags.finish()?;
            Ok(Command::TdaCompare { run })
        }
        other => Err(ParseError::UnknownCommand(format!("tda {other}"))),
    }
}

fn parse_export<'a, I: Iterator<Item = &'a str>>(mut it: I) -> Result<Command, ParseError> {
    let sub = it
        .next()
        .ok_or_else(|| ParseError::MissingArg { what: "export <format>".into() })?;
    match sub {
        "json" => {
            let mut flags = Flags::parse(it)?;
            let run = flags.take_u64("--run")?;
            flags.finish()?;
            Ok(Command::ExportJson { run })
        }
        "graphml" => {
            let mut flags = Flags::parse(it)?;
            let run = flags.take_u64("--run")?;
            flags.finish()?;
            Ok(Command::ExportGraphml { run })
        }
        // Parquet is a designed-but-unimplemented v0.1 format (a spec-compliant,
        // dependency-free columnar writer is a later milestone). Reject it with a
        // clear message rather than a generic "unknown command".
        "parquet" => Err(ParseError::UnknownCommand(
            "export parquet (not implemented in v0.1; use `json` or `graphml`)".into(),
        )),
        other => Err(ParseError::UnknownCommand(format!("export {other}"))),
    }
}

/// Reject any leftover positional/flag after a command that takes none.
fn reject_trailing<'a, I: Iterator<Item = &'a str>>(mut it: I) -> Result<(), ParseError> {
    match it.next() {
        Some(a) => Err(ParseError::Unexpected(a.to_string())),
        None => Ok(()),
    }
}

/// A collected `--name value` / `--name=value` flag map for one command.
///
/// Order-independent: every flag is gathered up front, then drained by the
/// command parser; any flag left over after the known ones are taken is an
/// [`ParseError::Unexpected`].
struct Flags {
    map: Vec<(String, String)>,
}

impl Flags {
    fn parse<'a, I: Iterator<Item = &'a str>>(mut it: I) -> Result<Flags, ParseError> {
        let mut map = Vec::new();
        while let Some(tok) = it.next() {
            if let Some(rest) = tok.strip_prefix("--") {
                if let Some((name, val)) = rest.split_once('=') {
                    map.push((format!("--{name}"), val.to_string()));
                } else {
                    let val = it.next().ok_or_else(|| ParseError::MissingArg {
                        what: tok.to_string(),
                    })?;
                    map.push((tok.to_string(), val.to_string()));
                }
            } else {
                return Err(ParseError::Unexpected(tok.to_string()));
            }
        }
        Ok(Flags { map })
    }

    /// Remove and return the value for `name`, if present.
    fn take_opt(&mut self, name: &str) -> Option<String> {
        if let Some(i) = self.map.iter().position(|(k, _)| k == name) {
            Some(self.map.remove(i).1)
        } else {
            None
        }
    }

    /// Remove a required `u64` flag.
    fn take_u64(&mut self, name: &str) -> Result<u64, ParseError> {
        let raw = self
            .take_opt(name)
            .ok_or_else(|| ParseError::MissingArg { what: name.to_string() })?;
        raw.parse::<u64>().map_err(|_| ParseError::BadValue {
            flag: name.to_string(),
            value: raw,
        })
    }

    /// Remove an optional `u64` flag, parsing it when present.
    fn take_u64_opt(&mut self, name: &str) -> Result<Option<u64>, ParseError> {
        match self.take_opt(name) {
            Some(raw) => raw
                .parse::<u64>()
                .map(Some)
                .map_err(|_| ParseError::BadValue {
                    flag: name.to_string(),
                    value: raw,
                }),
            None => Ok(None),
        }
    }

    /// Remove a required `u32` flag.
    fn take_u32(&mut self, name: &str) -> Result<u32, ParseError> {
        let raw = self
            .take_opt(name)
            .ok_or_else(|| ParseError::MissingArg { what: name.to_string() })?;
        raw.parse::<u32>().map_err(|_| ParseError::BadValue {
            flag: name.to_string(),
            value: raw,
        })
    }

    /// Error if any flags remain unconsumed.
    fn finish(self) -> Result<(), ParseError> {
        match self.map.into_iter().next() {
            Some((k, _)) => Err(ParseError::Unexpected(k)),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Result<Command, ParseError> {
        parse(args.iter().copied())
    }

    #[test]
    fn no_command_errors() {
        assert_eq!(p(&[]), Err(ParseError::NoCommand));
    }

    #[test]
    fn help_and_version() {
        assert_eq!(p(&["help"]), Ok(Command::Help));
        assert_eq!(p(&["-h"]), Ok(Command::Help));
        assert_eq!(p(&["--help"]), Ok(Command::Help));
        assert_eq!(p(&["--version"]), Ok(Command::Version));
        assert_eq!(p(&["-V"]), Ok(Command::Version));
    }

    #[test]
    fn init_default_and_explicit_path() {
        assert_eq!(p(&["init"]), Ok(Command::Init { path: ".".into() }));
        assert_eq!(
            p(&["init", "/tmp/ws"]),
            Ok(Command::Init { path: "/tmp/ws".into() })
        );
    }

    #[test]
    fn ingest_requires_path() {
        assert_eq!(
            p(&["ingest"]),
            Err(ParseError::MissingArg { what: "<path>".into() })
        );
        assert_eq!(
            p(&["ingest", "Cargo.lock"]),
            Ok(Command::Ingest { path: "Cargo.lock".into() })
        );
    }

    #[test]
    fn status_takes_no_args() {
        assert_eq!(p(&["status"]), Ok(Command::Status));
        assert_eq!(
            p(&["status", "extra"]),
            Err(ParseError::Unexpected("extra".into()))
        );
    }

    #[test]
    fn graph_stats_parses_run() {
        assert_eq!(
            p(&["graph", "stats", "--run", "7"]),
            Ok(Command::GraphStats { run: 7 })
        );
        // `--run=7` equals-form also works.
        assert_eq!(
            p(&["graph", "stats", "--run=7"]),
            Ok(Command::GraphStats { run: 7 })
        );
    }

    #[test]
    fn graph_stats_missing_run() {
        assert_eq!(
            p(&["graph", "stats"]),
            Err(ParseError::MissingArg { what: "--run".into() })
        );
    }

    #[test]
    fn graph_neighborhood_all_flags_any_order() {
        assert_eq!(
            p(&["graph", "neighborhood", "--node", "0", "--run", "3", "--depth", "2"]),
            Ok(Command::GraphNeighborhood { run: 3, node: 0, depth: 2 })
        );
    }

    #[test]
    fn graph_impact_parses() {
        assert_eq!(
            p(&["graph", "impact", "--run", "3", "--node", "5"]),
            Ok(Command::GraphImpact { run: 3, node: 5 })
        );
    }

    #[test]
    fn tda_run_default_filtration() {
        assert_eq!(
            p(&["tda", "run", "--run", "1"]),
            Ok(Command::TdaRun { run: 1, filtration: "depth".into() })
        );
        assert_eq!(
            p(&["tda", "run", "--run", "1", "--filtration", "depth"]),
            Ok(Command::TdaRun { run: 1, filtration: "depth".into() })
        );
    }

    #[test]
    fn tda_support_parses_pair() {
        assert_eq!(
            p(&["tda", "support", "--pair", "42"]),
            Ok(Command::TdaSupport { pair: 42 })
        );
    }

    #[test]
    fn export_json_parses() {
        assert_eq!(
            p(&["export", "json", "--run", "2"]),
            Ok(Command::ExportJson { run: 2 })
        );
    }

    #[test]
    fn graph_duplicates_parses() {
        assert_eq!(
            p(&["graph", "duplicates", "--run", "7"]),
            Ok(Command::GraphDuplicates { run: 7 })
        );
    }

    #[test]
    fn export_graphml_parses() {
        assert_eq!(
            p(&["export", "graphml", "--run", "3"]),
            Ok(Command::ExportGraphml { run: 3 })
        );
    }

    #[test]
    fn candidates_by_requirement() {
        assert_eq!(
            p(&["candidates", "--requirement", "7"]),
            Ok(Command::Candidates {
                requirement: Some(7),
                package: None,
                constraint: None,
            })
        );
    }

    #[test]
    fn candidates_by_package_and_constraint() {
        assert_eq!(
            p(&["candidates", "--package", "pkg:cargo/serde", "--constraint", "^1"]),
            Ok(Command::Candidates {
                requirement: None,
                package: Some("pkg:cargo/serde".into()),
                constraint: Some("^1".into()),
            })
        );
    }

    #[test]
    fn candidates_requires_one_form() {
        // Neither form supplied.
        assert!(matches!(
            p(&["candidates"]),
            Err(ParseError::MissingArg { .. })
        ));
        // Package without a constraint.
        assert_eq!(
            p(&["candidates", "--package", "pkg:cargo/serde"]),
            Err(ParseError::MissingArg { what: "--constraint".into() })
        );
        // Mixing both forms is rejected.
        assert!(matches!(
            p(&["candidates", "--requirement", "1", "--package", "pkg:cargo/x"]),
            Err(ParseError::Unexpected(_))
        ));
    }

    #[test]
    fn conflicts_requires_package() {
        assert_eq!(
            p(&["conflicts", "--package", "pkg:cargo/serde"]),
            Ok(Command::Conflicts { package: "pkg:cargo/serde".into() })
        );
        assert_eq!(
            p(&["conflicts"]),
            Err(ParseError::MissingArg { what: "--package".into() })
        );
    }

    #[test]
    fn resolve_manifest_and_policy() {
        assert_eq!(
            p(&["resolve", "Cargo.toml", "--policy", "lowest"]),
            Ok(Command::Resolve {
                manifest: "Cargo.toml".into(),
                policy: "lowest".into()
            })
        );
        // Default policy when omitted.
        assert_eq!(
            p(&["resolve", "Cargo.toml"]),
            Ok(Command::Resolve {
                manifest: "Cargo.toml".into(),
                policy: "highest".into()
            })
        );
    }

    #[test]
    fn resolve_requires_manifest_and_rejects_bad_policy() {
        assert_eq!(
            p(&["resolve"]),
            Err(ParseError::MissingArg { what: "<manifest-path>".into() })
        );
        assert_eq!(
            p(&["resolve", "Cargo.toml", "--policy", "bogus"]),
            Err(ParseError::BadValue {
                flag: "--policy".into(),
                value: "bogus".into()
            })
        );
    }

    #[test]
    fn bad_numeric_value() {
        assert_eq!(
            p(&["graph", "stats", "--run", "abc"]),
            Err(ParseError::BadValue {
                flag: "--run".into(),
                value: "abc".into()
            })
        );
    }

    #[test]
    fn flag_without_value_errors() {
        assert_eq!(
            p(&["graph", "stats", "--run"]),
            Err(ParseError::MissingArg { what: "--run".into() })
        );
    }

    #[test]
    fn unexpected_flag_rejected() {
        assert_eq!(
            p(&["graph", "stats", "--run", "1", "--bogus", "x"]),
            Err(ParseError::Unexpected("--bogus".into()))
        );
    }

    #[test]
    fn unknown_commands() {
        assert_eq!(
            p(&["frobnicate"]),
            Err(ParseError::UnknownCommand("frobnicate".into()))
        );
        assert_eq!(
            p(&["graph", "wat", "--run", "1"]),
            Err(ParseError::UnknownCommand("graph wat".into()))
        );
        // Parquet is recognized but unimplemented in v0.1: a clear message, not a
        // bare "unknown command".
        assert!(matches!(
            p(&["export", "parquet", "--run", "1"]),
            Err(ParseError::UnknownCommand(m)) if m.contains("parquet") && m.contains("not implemented")
        ));
    }

    #[test]
    fn positional_before_flag_is_unexpected() {
        // A stray positional where flags are expected is rejected, not silently
        // dropped.
        assert_eq!(
            p(&["graph", "stats", "oops"]),
            Err(ParseError::Unexpected("oops".into()))
        );
    }
}
