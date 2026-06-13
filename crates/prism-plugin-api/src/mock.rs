//! A tiny in-crate mock [`EcosystemPlugin`] used to prove the trait surface is
//! object-safe and behaves as intended. Test-only; never compiled into the
//! library proper.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use prism_model::{
    ConditionKind, EcosystemCapabilities, EcosystemId, EdgeFlags, EnvironmentId, FidelityTier,
    GraphFidelity, NodeFlags, RequirementId, SnapshotId, VersionId,
};

use crate::activation::{ActivationContext, GateInput};
use crate::capabilities::*;
use crate::coord::PackageName;
use crate::fetch::*;
use crate::parse::*;
use crate::plugin::EcosystemPlugin;
use crate::resolve::*;
use crate::url::SafeUrl;

// --- a dependency-free, safe block_on for the boxed-future capabilities -----

/// A no-op waker built without `unsafe` (this crate forbids `unsafe`): the
/// [`Wake`] trait + `Arc` give a safe path to a real [`Waker`].
struct NoopWake;
impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
    fn wake_by_ref(self: &Arc<Self>) {}
}

/// Drive a `Future` to completion on the current thread. The mock futures are
/// always immediately `Ready`, so one poll suffices; we loop defensively (the
/// no-op waker never re-schedules, so a genuinely `Pending` future would spin —
/// the mock never produces one).
fn block_on<F: Future>(fut: F) -> F::Output {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut cx = Context::from_waker(&waker);
    let mut fut = std::pin::pin!(fut);
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => continue,
        }
    }
}

// --- mock capability impls -------------------------------------------------

struct MockNames;
impl NameNormalizer for MockNames {
    fn normalize(&self, raw: &str) -> Result<String, ParseError> {
        if raw.is_empty() {
            return Err(ParseError::new("empty-name", "name is empty"));
        }
        Ok(raw.to_ascii_lowercase())
    }
}

struct MockVersions;
impl VersionScheme for MockVersions {
    fn parse(&self, raw: &str) -> Result<String, ParseError> {
        let v = raw.strip_prefix('v').unwrap_or(raw);
        // `<numeric.dotted>[-<alnum prerelease>]`.
        let (release, _pre) = v.split_once('-').map_or((v, ""), |(r, p)| (r, p));
        if release.is_empty() || !release.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
            return Err(ParseError::new("bad-version", raw));
        }
        Ok(v.to_string())
    }
    fn compare(&self, a: &str, b: &str) -> Result<VersionOrdering, ParseError> {
        Ok(self.sort_key(a)?.cmp(&self.sort_key(b)?))
    }
    fn sort_key(&self, raw: &str) -> Result<Vec<u8>, ParseError> {
        // Numeric-segment key: each dot-separated numeric segment as a u32 BE.
        let norm = self.parse(raw)?;
        let base = norm.split('-').next().unwrap_or(&norm);
        let mut key = Vec::new();
        for seg in base.split('.') {
            let n: u32 = seg.parse().map_err(|_| ParseError::new("bad-segment", seg))?;
            key.extend_from_slice(&n.to_be_bytes());
        }
        Ok(key)
    }
    fn is_prerelease(&self, raw: &str) -> Result<bool, ParseError> {
        Ok(self.parse(raw)?.contains('-'))
    }
}

struct MockReqs;
impl RequirementParser for MockReqs {
    fn canonicalize(&self, raw: &str) -> Result<String, ParseError> {
        Ok(raw.trim().to_string())
    }
    fn matches(&self, raw: &str, version: &str) -> Result<bool, ParseError> {
        // `*` matches anything; otherwise exact match.
        Ok(raw.trim() == "*" || raw.trim() == version.trim())
    }
}

struct MockConditions;
impl ConditionEvaluator for MockConditions {
    fn evaluate(&self, raw: Option<&str>, input: GateInput<'_>) -> Result<bool, ParseError> {
        match raw {
            None => Ok(true),
            Some(expr) => match input {
                GateInput::Unconditional => Ok(true),
                GateInput::Environment { .. } => Ok(expr.contains("linux")),
                GateInput::Activation { anchor, context } => {
                    // `feature:NAME` is active iff NAME is activated on the anchor.
                    let name = expr.strip_prefix("feature:").unwrap_or(expr);
                    Ok(context.is_active(anchor, name))
                }
            },
        }
    }
    fn classify(&self, raw: &str) -> Result<ConditionKind, ParseError> {
        if raw.starts_with("feature:") || raw.starts_with("extra") {
            Ok(ConditionKind::Activation)
        } else {
            Ok(ConditionKind::Environment)
        }
    }
}

struct MockDetector;
impl ProjectDetector for MockDetector {
    fn detects(&self, root: &Path) -> bool {
        root.join("mock.toml").file_name().is_some()
    }
    fn manifest_filenames(&self) -> &'static [&'static str] {
        &["mock.toml"]
    }
    fn lockfile_filenames(&self) -> &'static [&'static str] {
        &["mock.lock"]
    }
}

struct MockManifest;
impl ManifestParser for MockManifest {
    fn parse_manifest(&self, raw: &RawDocument) -> Result<CanonicalBatch, ParseError> {
        let mut b = CanonicalBatch::new();
        let text = std::str::from_utf8(&raw.bytes)
            .map_err(|_| ParseError::new("not-utf8", "manifest is not utf-8"))?;
        b.push(CanonicalEvent::PackageSeen(PackageFact {
            name: PackageName::from(text.trim()),
            namespace: None,
            repository_url: None,
            homepage_url: None,
        }));
        Ok(b)
    }
}

struct MockLockfile;
impl LockfileParser for MockLockfile {
    fn parse_lockfile(&self, _raw: &RawDocument) -> Result<CanonicalBatch, ParseError> {
        Ok(CanonicalBatch::new())
    }
}

struct MockMeta;
impl RegistryMetadataParser for MockMeta {
    fn parse_package_index(&self, _raw: &RawDocument) -> Result<CanonicalBatch, ParseError> {
        let mut b = CanonicalBatch::new();
        b.push(CanonicalEvent::VersionSeen(VersionFact {
            name: PackageName::from("mock"),
            version_raw: "1.0.0".into(),
            released_at_unix: None,
            license_expr: None,
            is_prerelease: false,
        }));
        Ok(b)
    }
    fn parse_version_metadata(&self, _raw: &RawDocument) -> Result<CanonicalBatch, ParseError> {
        Ok(CanonicalBatch::new())
    }
    fn parse_artifact_metadata(&self, _raw: &RawDocument) -> Result<CanonicalBatch, ParseError> {
        Ok(CanonicalBatch::new())
    }
}

struct MockCandidates;
impl CandidateProvider for MockCandidates {
    fn versions_for(&self, canonical_name: &str) -> Result<Vec<String>, ParseError> {
        if canonical_name == "mock" {
            Ok(vec!["1.0.0".into(), "1.1.0".into(), "2.0.0".into()])
        } else {
            Ok(Vec::new())
        }
    }
}

struct MockFetcher;
impl RegistryFetcher for MockFetcher {
    fn fetch<'a>(
        &'a self,
        request: FetchRequest,
    ) -> Pin<Box<dyn Future<Output = Result<RawDocument, FetchError>> + Send + 'a>> {
        Box::pin(async move {
            let key = request.source.base_url.clone();
            Ok(RawDocument {
                source_key: key.clone(),
                media_type: Some("application/json".into()),
                content_hash: prism_model::HashDigest::default(),
                bytes: b"mock".to_vec(),
                fetched_at_unix: 0,
                etag: None,
                last_modified: None,
                provenance: Provenance {
                    source_id: request.source.id,
                    retrieved_from: key,
                    from_cache: false,
                    snapshot: request.snapshot,
                },
            })
        })
    }
}

struct MockResolver;
impl ResolverAdapter for MockResolver {
    fn fidelity(&self) -> GraphFidelity {
        GraphFidelity::exact(FidelityTier::ModelResolved)
    }
    fn resolve<'a>(
        &'a self,
        request: ResolveRequest,
        _ctx: &'a ResolveContext,
    ) -> Pin<Box<dyn Future<Output = Result<ResolvedGraph, ResolveError>> + Send + 'a>> {
        Box::pin(async move {
            let mut g = ResolvedGraph::new();
            let root = g.add_node(VersionId::new(1), 0, "", NodeFlags::ROOT);
            let dep = g.add_node(VersionId::new(2), 1, "dep", NodeFlags::EMPTY);
            g.add_edge(ResolvedEdge {
                from: root,
                to: dep,
                ordinal: 0,
                requirement_id: request.roots.first().copied(),
                kind: prism_model::DependencyKind::Runtime,
                flags: EdgeFlags::RESOLVED,
            });
            g.fidelity = Some(self.fidelity());
            Ok(g)
        })
    }
}

/// The mock plugin: implements all mandatory + optional capabilities.
struct MockPlugin {
    id: EcosystemId,
    names: MockNames,
    versions: MockVersions,
    reqs: MockReqs,
    conds: MockConditions,
    detector: MockDetector,
    manifest: MockManifest,
    lockfile: MockLockfile,
    meta: MockMeta,
    candidates: MockCandidates,
    fetcher: MockFetcher,
    resolver: MockResolver,
}

impl MockPlugin {
    fn new() -> Self {
        Self {
            id: EcosystemId::new(7),
            names: MockNames,
            versions: MockVersions,
            reqs: MockReqs,
            conds: MockConditions,
            detector: MockDetector,
            manifest: MockManifest,
            lockfile: MockLockfile,
            meta: MockMeta,
            candidates: MockCandidates,
            fetcher: MockFetcher,
            resolver: MockResolver,
        }
    }
}

impl EcosystemPlugin for MockPlugin {
    fn id(&self) -> EcosystemId {
        self.id
    }
    fn capabilities(&self) -> EcosystemCapabilities {
        EcosystemCapabilities::PARSE_MANIFEST
            | EcosystemCapabilities::PARSE_LOCKFILE
            | EcosystemCapabilities::FETCH_REGISTRY
            | EcosystemCapabilities::ENUM_CANDIDATES
            | EcosystemCapabilities::SYNTHETIC_RESOLVE
            | EcosystemCapabilities::SUPPORTS_FEATURES
    }
    fn name_normalizer(&self) -> &dyn NameNormalizer {
        &self.names
    }
    fn version_scheme(&self) -> &dyn VersionScheme {
        &self.versions
    }
    fn requirement_parser(&self) -> &dyn RequirementParser {
        &self.reqs
    }
    fn condition_evaluator(&self) -> &dyn ConditionEvaluator {
        &self.conds
    }
    fn project_detector(&self) -> Option<&dyn ProjectDetector> {
        Some(&self.detector)
    }
    fn manifest_parser(&self) -> Option<&dyn ManifestParser> {
        Some(&self.manifest)
    }
    fn lockfile_parser(&self) -> Option<&dyn LockfileParser> {
        Some(&self.lockfile)
    }
    fn registry_fetcher(&self) -> Option<&dyn RegistryFetcher> {
        Some(&self.fetcher)
    }
    fn metadata_parser(&self) -> Option<&dyn RegistryMetadataParser> {
        Some(&self.meta)
    }
    fn candidate_provider(&self) -> Option<&dyn CandidateProvider> {
        Some(&self.candidates)
    }
    fn resolver_adapter(&self) -> Option<&dyn ResolverAdapter> {
        Some(&self.resolver)
    }
}

// --- tests -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed() -> Box<dyn EcosystemPlugin> {
        Box::new(MockPlugin::new())
    }

    #[test]
    fn stores_as_boxed_trait_object() {
        let p: Box<dyn EcosystemPlugin> = boxed();
        assert_eq!(p.id(), EcosystemId::new(7));
    }

    #[test]
    fn capability_flags_round_trip_with_accessors() {
        let p = boxed();
        // self_consistent checks the advertised flags match Some() accessors.
        assert!(p.self_consistent());
        let caps = p.capabilities();
        assert!(caps.contains(EcosystemCapabilities::PARSE_MANIFEST));
        assert_eq!(
            p.manifest_parser().is_some(),
            caps.contains(EcosystemCapabilities::PARSE_MANIFEST)
        );
        assert_eq!(
            p.candidate_provider().is_some(),
            caps.contains(EcosystemCapabilities::ENUM_CANDIDATES)
        );
        // A capability the mock does not advertise has no matching flag.
        assert!(!caps.contains(EcosystemCapabilities::SUPPORTS_PLATFORM));
        // bits round-trip through from_bits_truncate.
        let bits = caps.bits();
        assert_eq!(EcosystemCapabilities::from_bits_truncate(bits), caps);
    }

    #[test]
    fn mandatory_capabilities_work_through_dyn() {
        let p = boxed();
        assert_eq!(p.name_normalizer().normalize("React").unwrap(), "react");
        assert!(p.name_normalizer().names_equal("React", "react"));
        let vs = p.version_scheme();
        assert_eq!(vs.parse("v1.2.3").unwrap(), "1.2.3");
        assert_eq!(
            vs.compare("1.2.0", "1.10.0").unwrap(),
            std::cmp::Ordering::Less
        );
        assert!(vs.is_prerelease("1.0.0-rc1").unwrap());
        assert!(p.requirement_parser().matches("*", "9.9.9").unwrap());
        assert_eq!(
            p.condition_evaluator().classify("feature:foo").unwrap(),
            ConditionKind::Activation
        );
    }

    #[test]
    fn sort_key_agrees_with_compare() {
        let vs = MockVersions;
        let mut versions = ["1.10.0", "1.2.0", "2.0.0", "1.1.0"];
        versions.sort_by(|a, b| vs.compare(a, b).unwrap());
        assert_eq!(versions, ["1.1.0", "1.2.0", "1.10.0", "2.0.0"]);
        // sort_key bytes order the same way.
        let mut keyed: Vec<_> = ["1.10.0", "1.2.0"].iter().map(|v| vs.sort_key(v).unwrap()).collect();
        keyed.sort();
        assert_eq!(keyed[0], vs.sort_key("1.2.0").unwrap());
    }

    #[test]
    fn condition_gate_split_routes_to_activation() {
        let p = boxed();
        let anchor = prism_model::PackageId::new(1);
        let mut ctx = ActivationContext::new();
        let ce = p.condition_evaluator();
        // Not active yet.
        assert!(!ce
            .evaluate(
                Some("feature:derive"),
                GateInput::Activation { anchor, context: &ctx }
            )
            .unwrap());
        ctx.activate(anchor, "derive");
        assert!(ce
            .evaluate(
                Some("feature:derive"),
                GateInput::Activation { anchor, context: &ctx }
            )
            .unwrap());
        // Environment gate uses environment state, not activation.
        assert!(ce
            .evaluate(
                Some("sys_platform == linux"),
                GateInput::Environment { environment: EnvironmentId::new(1) }
            )
            .unwrap());
        // No condition is always active.
        assert!(ce.evaluate(None, GateInput::Unconditional).unwrap());
    }

    #[test]
    fn manifest_parser_collects_events() {
        let p = boxed();
        let raw = RawDocument {
            source_key: SafeUrl::new("file:///mock.toml"),
            media_type: None,
            content_hash: prism_model::HashDigest::default(),
            bytes: b"mockpkg".to_vec(),
            fetched_at_unix: 0,
            etag: None,
            last_modified: None,
            provenance: Provenance {
                source_id: "local".into(),
                retrieved_from: SafeUrl::new("file:///mock.toml"),
                from_cache: false,
                snapshot: None,
            },
        };
        let batch = p.manifest_parser().unwrap().parse_manifest(&raw).unwrap();
        assert_eq!(batch.len(), 1);
        assert_eq!(batch.fact_count(), 1);
        match &batch.events[0] {
            CanonicalEvent::PackageSeen(f) => assert_eq!(f.name.as_str(), "mockpkg"),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn metadata_parser_collects_version_fact() {
        let p = boxed();
        let raw = RawDocument {
            source_key: SafeUrl::new("https://reg/mock"),
            media_type: None,
            content_hash: prism_model::HashDigest::default(),
            bytes: Vec::new(),
            fetched_at_unix: 0,
            etag: None,
            last_modified: None,
            provenance: Provenance {
                source_id: "reg".into(),
                retrieved_from: SafeUrl::new("https://reg/mock"),
                from_cache: false,
                snapshot: None,
            },
        };
        let batch = p
            .metadata_parser()
            .unwrap()
            .parse_package_index(&raw)
            .unwrap();
        assert!(matches!(batch.events[0], CanonicalEvent::VersionSeen(_)));
    }

    #[test]
    fn candidate_provider_lists_versions() {
        let p = boxed();
        let v = p.candidate_provider().unwrap().versions_for("mock").unwrap();
        assert_eq!(v, vec!["1.0.0", "1.1.0", "2.0.0"]);
        assert!(p
            .candidate_provider()
            .unwrap()
            .versions_for("nope")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn fetcher_returns_redacted_document() {
        let p = boxed();
        let req = FetchRequest {
            ecosystem: EcosystemId::new(7),
            source: RegistrySource::new(
                "reg",
                SafeUrl::new("https://user:tok@reg.example/index"),
            ),
            target: FetchTarget::PackageIndex {
                package: PackageName::from("mock"),
            },
            auth: Some(AuthRef::new("reg-token")),
            snapshot: Some(SnapshotId::new(3)),
            cache_policy: CachePolicy::Default,
        };
        let doc = block_on(p.registry_fetcher().unwrap().fetch(req)).unwrap();
        // Credentials stripped on the way through.
        assert_eq!(doc.source_key.as_str(), "https://reg.example/index");
        assert!(!doc.provenance.retrieved_from.as_str().contains("tok"));
        assert_eq!(doc.provenance.snapshot, Some(SnapshotId::new(3)));
        assert_eq!(doc.bytes, b"mock");
    }

    #[test]
    fn resolver_returns_plain_graph() {
        let p = boxed();
        let ra = p.resolver_adapter().unwrap();
        assert_eq!(ra.fidelity().tier, FidelityTier::ModelResolved);
        let req = ResolveRequest {
            ecosystem: EcosystemId::new(7),
            roots: vec![RequirementId::new(42)],
            policy: ResolvePolicy::Latest,
            max_nodes: None,
        };
        let ctx = ResolveContext {
            environment: EnvironmentId::new(1),
            snapshot: SnapshotId::new(1),
            activation: ActivationContext::new(),
        };
        let g = block_on(ra.resolve(req, &ctx)).unwrap();
        assert_eq!(g.node_count(), 2);
        assert_eq!(g.edge_count(), 1);
        assert_eq!(g.edges[0].requirement_id, Some(RequirementId::new(42)));
        assert_eq!(g.fidelity.unwrap().tier, FidelityTier::ModelResolved);
    }
}
