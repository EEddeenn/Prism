//! End-to-end tests for the `prism` CLI.
//!
//! Two complementary slices:
//!   1. drive the flow directly through `prism-api` (init → ingest → stats → tda)
//!      asserting the key numbers, and
//!   2. shell out to the *built* binary (`CARGO_BIN_EXE_prism`) and assert exit
//!      codes and rendered output for the whole command surface.
//!
//! Both use a small self-cleaning temp dir (no external crates), mirroring the
//! `prism-api` `TempDir` pattern.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use prism_api::{Ecosystem, Prism};
use prism_model::GraphNodeId;

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A temporary directory removed on drop.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("prism-cli-e2e-{label}-{pid}-{n}"));
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// The fixture lockfile: `sample-app` → serde, rand; serde → serde_derive;
/// rand → rand_core. Five crates, four resolved edges, a tree (no cycle).
const FIXTURE_LOCK: &str = include_str!("../fixtures/Cargo.lock");

/// Run the built `prism` binary with `args` in `cwd`, returning (code, stdout,
/// stderr).
fn run_cli(cwd: &Path, args: &[&str]) -> (i32, String, String) {
    let exe = env!("CARGO_BIN_EXE_prism");
    let out = Command::new(exe)
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("spawn prism");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn api_flow_init_ingest_stats_tda() {
    let td = TempDir::new("api");
    let lock = td.path().join("Cargo.lock");
    std::fs::write(&lock, FIXTURE_LOCK).unwrap();

    // init -> ingest.
    let mut prism = Prism::init(td.path()).unwrap();
    assert_eq!(Prism::detect(&lock), Some(Ecosystem::Cargo));
    let ingest = prism.ingest_lockfile_detailed(&lock).unwrap();
    assert_eq!(ingest.ecosystem, Ecosystem::Cargo);
    assert_eq!(ingest.nodes, 5, "five crates -> five nodes");
    assert_eq!(ingest.edges, 4, "four resolved dependency edges");
    let run = ingest.run;

    // stats: a tree -> one SCC per node, no cycle.
    let stats = prism.graph_stats(run).unwrap();
    assert_eq!(stats.nodes, 5);
    assert_eq!(stats.edges, 4);
    assert!(!stats.has_cycle);
    assert_eq!(stats.scc_count, 5);
    assert_eq!(stats.duplicate_version_count, 0);

    // tda: depth filtration of a connected tree -> exactly one H0 component, no
    // H1 cycles.
    let barcode = prism.tda_run(run, "depth").unwrap();
    assert!(barcode.is_nonempty());
    assert_eq!(barcode.component_count(), 1);
    assert_eq!(barcode.cycle_count(), 0);

    // neighborhood of the root (node 0): four descendants.
    let nbhd = prism.neighborhood(run, GraphNodeId::new(0), 10, 100).unwrap();
    assert_eq!(nbhd.nodes.len(), 4);
    assert!(!nbhd.truncated);

    // The persisted .ptda is discoverable and re-readable after reopen.
    let filtrations = prism.tda_filtrations(run);
    assert_eq!(filtrations.len(), 1);
    drop(prism);
    let prism2 = Prism::open(td.path()).unwrap();
    assert_eq!(prism2.graph_stats(run).unwrap().nodes, 5);
    assert_eq!(
        prism2.tda_barcode(run, filtrations[0]).unwrap().component_count(),
        1
    );
}

#[test]
fn cli_binary_full_flow() {
    let td = TempDir::new("bin");
    std::fs::write(td.path().join("Cargo.lock"), FIXTURE_LOCK).unwrap();

    // init in the workspace dir.
    let (code, out, _) = run_cli(td.path(), &["init", "."]);
    assert_eq!(code, 0, "init should succeed");
    assert!(out.contains("initialized"), "{out}");

    // ingest the lockfile; capture the run id from the summary line.
    let (code, out, err) = run_cli(td.path(), &["ingest", "Cargo.lock"]);
    assert_eq!(code, 0, "ingest failed: {err}");
    assert!(out.contains("ingested lockfile"), "{out}");
    assert!(out.contains("5 nodes"), "{out}");
    assert!(out.contains("4 edges"), "{out}");
    let run = parse_run_id(&out);

    // status reflects one run, five packages, one graph.
    let (code, out, _) = run_cli(td.path(), &["status"]);
    assert_eq!(code, 0);
    assert!(out.contains("runs:     1"), "{out}");
    assert!(out.contains("packages: 5"), "{out}");
    assert!(out.contains("graphs:   1"), "{out}");

    // graph stats.
    let (code, out, err) = run_cli(td.path(), &["graph", "stats", "--run", &run.to_string()]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("nodes:             5"), "{out}");
    assert!(out.contains("edges:             4"), "{out}");
    assert!(out.contains("has cycle:         false"), "{out}");

    // graph neighborhood of the root.
    let (code, out, err) = run_cli(
        td.path(),
        &["graph", "neighborhood", "--run", &run.to_string(), "--node", "0", "--depth", "10"],
    );
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("reached 4 node(s)"), "{out}");

    // graph impact of a leaf-ish node (serde_derive has ancestors serde + app).
    // Find serde_derive's node id by exporting json first.
    let (code, json, err) = run_cli(td.path(), &["export", "json", "--run", &run.to_string()]);
    assert_eq!(code, 0, "{err}");
    assert!(json.contains("\"nodes\":5"), "{json}");
    assert!(json.contains("\"edges\":4"), "{json}");
    assert!(json.contains("pkg:cargo/sample-app@0.1.0"), "{json}");

    // graph impact of node 2 (serde_derive): its ancestors are serde and the
    // root, so two dependents.
    let (code, out, err) = run_cli(
        td.path(),
        &["graph", "impact", "--run", &run.to_string(), "--node", "2"],
    );
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("2 dependent(s)"), "{out}");

    // tda run.
    let (code, out, err) = run_cli(
        td.path(),
        &["tda", "run", "--run", &run.to_string(), "--filtration", "depth"],
    );
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("components (H0 essential): 1"), "{out}");
    assert!(out.contains("cycles (H1 essential):     0"), "{out}");

    // tda support: the H0 finite pairs are addressable; locate one by scanning
    // the run's persisted .ptda from the facade, then ask the CLI for it.
    let pair = {
        let prism = Prism::open(td.path()).unwrap();
        let rid = prism_model::RunId::new(run);
        let filt = prism.tda_filtrations(rid)[0];
        let barcode = prism.tda_barcode(rid, filt).unwrap();
        // There is at least one finite H0 pair (4 of them for this tree).
        assert!(barcode.dims.iter().any(|d| d.finite > 0));
        // Find a concrete pair id by reading back the persisted result.
        find_finite_pair(&prism, rid, filt)
    };
    let (code, out, err) = run_cli(
        td.path(),
        &["tda", "support", "--pair", &pair.to_string()],
    );
    assert_eq!(code, 0, "tda support failed: {err}");
    assert!(out.contains(&format!("pair {pair}")), "{out}");
    assert!(out.contains("support object(s)"), "{out}");

    // tda support for a non-existent pair -> NotFound (exit 3).
    let (code, _out, _err) = run_cli(td.path(), &["tda", "support", "--pair", "9999999"]);
    assert_eq!(code, 3);
}

/// Read back the persisted `.ptda` for `(run, filtration)` and return the id of
/// some finite (born-and-killed) pair, whose generator support is non-empty.
fn find_finite_pair(
    prism: &Prism,
    run: prism_model::RunId,
    filt: prism_model::FiltrationId,
) -> u64 {
    // The facade does not expose pair ids directly, but `tda_support` over a
    // scanned id surfaces NotFound for absent pairs; pair ids are packed as
    // `(filtration_id << 32) | ordinal` (globally unique across filtrations), so
    // the filtration's pairs start at `filtration_id << 32`. Scan a small window.
    let base = filt.get() << 32;
    for candidate in base..base + 64 {
        let pid = prism_model::PairId::new(candidate);
        if let Ok(view) = prism.tda_support(run, filt, pid) {
            if !view.objects.is_empty() {
                return candidate;
            }
        }
    }
    panic!("no finite pair found near filtration {}", base);
}

/// A Cargo.lock with two serde versions, for the candidate/conflict/resolve flow.
const TWO_SERDE_LOCK: &str = "\
# @generated by Cargo
version = 3

[[package]]
name = \"app\"
version = \"0.1.0\"
dependencies = [
 \"serde 1.0.0\",
]

[[package]]
name = \"serde\"
version = \"1.0.0\"
source = \"registry+https://github.com/rust-lang/crates.io-index\"

[[package]]
name = \"serde\"
version = \"2.0.0\"
source = \"registry+https://github.com/rust-lang/crates.io-index\"
";

#[test]
fn api_resolve_manifest_synthetic() {
    let td = TempDir::new("api-resolve");
    // Seed serde versions and a manifest declaring serde ^2.
    std::fs::write(td.path().join("Cargo.lock"), TWO_SERDE_LOCK).unwrap();
    let man = td.path().join("Cargo.toml");
    std::fs::write(
        &man,
        "[package]\nname = \"root\"\nversion = \"1.0.0\"\n\n[dependencies]\nserde = \"^2\"\n",
    )
    .unwrap();

    let mut prism = Prism::init(td.path()).unwrap();
    prism.ingest_lockfile(td.path().join("Cargo.lock")).unwrap();

    // resolve_manifest ingests the manifest + runs the synthetic resolver.
    let res = prism.resolve_manifest(&man, "highest").unwrap();
    assert_eq!(res.nodes, 2, "root + serde");
    assert_eq!(res.edges, 1, "root -> serde");
    assert!(!res.truncated);

    // The run's graph is queryable and reports the same sizes.
    let stats = prism.graph_stats(res.run).unwrap();
    assert_eq!(stats.nodes, 2);
    assert_eq!(stats.edges, 1);

    // The serde package's candidate space: ^2 -> exactly 2.0.0.
    let serde = (0..stats.nodes as u32)
        .map(|i| prism.node(res.run, GraphNodeId::new(i)).unwrap())
        .find(|n| n.purl.starts_with("pkg:cargo/serde@"))
        .and_then(|n| n.package)
        .expect("serde resolved");
    let view = prism.candidates_for(serde, "^2", Ecosystem::Cargo).unwrap();
    assert_eq!(view.count(), 1);
    assert!(view.candidates[0].purl.starts_with("pkg:cargo/serde@2.0.0"));
}

#[test]
fn cli_binary_candidates_conflicts_resolve() {
    let td = TempDir::new("bin-candidates");
    std::fs::write(td.path().join("Cargo.lock"), TWO_SERDE_LOCK).unwrap();
    std::fs::write(
        td.path().join("Cargo.toml"),
        "[package]\nname = \"root\"\nversion = \"1.0.0\"\n\n[dependencies]\nserde = \"^2\"\n",
    )
    .unwrap();

    // init + seed serde versions via the lockfile.
    assert_eq!(run_cli(td.path(), &["init", "."]).0, 0);
    assert_eq!(run_cli(td.path(), &["ingest", "Cargo.lock"]).0, 0);

    // candidates --package + --constraint (no requirement needed).
    let (code, out, err) = run_cli(
        td.path(),
        &["candidates", "--package", "pkg:cargo/serde", "--constraint", "^2"],
    );
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("1 of 2 version(s) satisfy"), "{out}");
    assert!(out.contains("pkg:cargo/serde@2.0.0"), "{out}");

    // resolve <manifest> --policy highest (ingests the manifest's requirements).
    let (code, out, err) = run_cli(
        td.path(),
        &["resolve", "Cargo.toml", "--policy", "highest"],
    );
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("resolved (highest): run"), "{out}");
    assert!(out.contains("2 nodes"), "{out}");
    assert!(out.contains("1 edges"), "{out}");

    // conflicts --package: a single requirement -> no conflict.
    let (code, out, err) = run_cli(td.path(), &["conflicts", "--package", "pkg:cargo/serde"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("no conflicts"), "{out}");

    // An unknown package purl -> NotFound (exit 3).
    let (code, _o, _e) = run_cli(td.path(), &["conflicts", "--package", "pkg:cargo/nope"]);
    assert_eq!(code, 3);

    // A bad policy is a usage error (exit 2), caught by the arg parser.
    let (code, _o, _e) = run_cli(td.path(), &["resolve", "Cargo.toml", "--policy", "bogus"]);
    assert_eq!(code, 2);
}

#[test]
fn cli_help_and_version() {
    let td = TempDir::new("help");
    let (code, out, _) = run_cli(td.path(), &["help"]);
    assert_eq!(code, 0);
    assert!(out.contains("USAGE"));
    assert!(out.contains("graph stats"));

    let (code, out, _) = run_cli(td.path(), &["--version"]);
    assert_eq!(code, 0);
    assert!(out.contains("prism"));
}

#[test]
fn cli_exit_codes() {
    let td = TempDir::new("codes");

    // Usage error: unknown command -> code 2.
    let (code, _out, err) = run_cli(td.path(), &["frobnicate"]);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("unknown command"), "{err}");

    // Usage error: missing flag -> code 2.
    let (code, _out, _err) = run_cli(td.path(), &["graph", "stats"]);
    assert_eq!(code, 2);

    // Not found: query a run with no published graph in a fresh workspace -> code 3.
    let (code, _out, _) = run_cli(td.path(), &["init", "."]);
    assert_eq!(code, 0);
    let (code, _out, err) = run_cli(td.path(), &["graph", "stats", "--run", "999"]);
    assert_eq!(code, 3, "expected NotFound exit code, got {code}: {err}");
}

/// Extract the run id from an `ingest` summary line of the form
/// `... run <N> with ...`.
fn parse_run_id(out: &str) -> u64 {
    for tok in out.split_whitespace() {
        if let Ok(n) = tok.parse::<u64>() {
            return n;
        }
    }
    panic!("no run id in ingest output: {out}");
}
