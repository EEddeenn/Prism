//! A native lockfile [`ResolverAdapter`]: it reads a captured
//! `package-lock.json` and replays it verbatim into a [`ResolvedGraph`].
//!
//! Because [`ResolverAdapter::resolve`] receives only a [`ResolveRequest`] and a
//! [`ResolveContext`] (no document bytes), the lockfile is supplied when the
//! adapter is constructed ([`LockfileResolver::new`]). This matches how a native
//! resolver works: the lockfile *is* the resolution, and the request only
//! selects roots/policy. The policy must be [`ResolvePolicy::Locked`]; any other
//! policy is rejected, since a lockfile makes no free choices.

use std::future::Future;
use std::pin::Pin;

use prism_model::{FidelityCaveats, FidelityTier, GraphFidelity};
use prism_plugin_api::{
    ParseError, RawDocument, ResolveContext, ResolveError, ResolvePolicy, ResolveRequest,
    ResolvedGraph, ResolverAdapter,
};

use crate::lockfile::parse_package_lock;

/// A [`ResolverAdapter`] backed by an already-captured `package-lock.json`.
#[derive(Debug, Clone)]
pub struct LockfileResolver {
    lockfile: RawDocument,
}

impl LockfileResolver {
    /// Construct a resolver over a captured `package-lock.json` document.
    pub fn new(lockfile: RawDocument) -> Self {
        Self { lockfile }
    }

    /// Eagerly parse the lockfile into a [`ResolvedGraph`] (sync convenience used
    /// by callers that don't want the boxed future).
    pub fn resolve_now(&self) -> Result<ResolvedGraph, ParseError> {
        parse_package_lock(&self.lockfile)
    }
}

impl ResolverAdapter for LockfileResolver {
    fn fidelity(&self) -> GraphFidelity {
        GraphFidelity {
            tier: FidelityTier::LockfileExact,
            caveats: FidelityCaveats::NO_PEER_PLACEMENT,
        }
    }

    fn resolve<'a>(
        &'a self,
        request: ResolveRequest,
        _ctx: &'a ResolveContext,
    ) -> Pin<Box<dyn Future<Output = Result<ResolvedGraph, ResolveError>> + Send + 'a>> {
        Box::pin(async move {
            if request.policy != ResolvePolicy::Locked {
                return Err(ResolveError::NativeToolFailed {
                    detail: "lockfile resolver only supports the Locked policy".into(),
                });
            }
            let mut graph = parse_package_lock(&self.lockfile)
                .map_err(|e| ResolveError::NativeToolFailed { detail: e.to_string() })?;
            if let Some(max) = request.max_nodes {
                if graph.node_count() as u64 > max as u64 {
                    graph.truncated = true;
                }
            }
            Ok(graph)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_model::{EcosystemId, EnvironmentId, HashDigest, SnapshotId};
    use prism_plugin_api::activation::ActivationContext;
    use prism_plugin_api::{Provenance, SafeUrl};
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};

    struct NoopWake;
    impl Wake for NoopWake {
        fn wake(self: Arc<Self>) {}
        fn wake_by_ref(self: &Arc<Self>) {}
    }

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

    const LOCK: &str = r#"{
      "lockfileVersion": 3,
      "packages": {
        "": { "version": "1.0.0", "dependencies": { "x": "^1.0.0" } },
        "node_modules/x": { "version": "1.2.3" }
      }
    }"#;

    fn doc() -> RawDocument {
        RawDocument {
            source_key: SafeUrl::new("file:///package-lock.json"),
            media_type: Some("application/json".into()),
            content_hash: HashDigest::default(),
            bytes: LOCK.as_bytes().to_vec(),
            fetched_at_unix: 0,
            etag: None,
            last_modified: None,
            provenance: Provenance {
                source_id: "local".into(),
                retrieved_from: SafeUrl::new("file:///package-lock.json"),
                from_cache: false,
                snapshot: None,
            },
        }
    }

    fn ctx() -> ResolveContext {
        ResolveContext {
            environment: EnvironmentId::new(1),
            snapshot: SnapshotId::new(1),
            activation: ActivationContext::new(),
        }
    }

    fn req(policy: ResolvePolicy) -> ResolveRequest {
        ResolveRequest {
            ecosystem: EcosystemId::new(1),
            roots: Vec::new(),
            policy,
            max_nodes: None,
        }
    }

    #[test]
    fn locked_policy_replays_lockfile() {
        let r = LockfileResolver::new(doc());
        assert_eq!(r.fidelity().tier, FidelityTier::LockfileExact);
        let g = block_on(r.resolve(req(ResolvePolicy::Locked), &ctx())).unwrap();
        assert_eq!(g.node_count(), 2);
        assert_eq!(g.edge_count(), 1);
        assert!(!g.truncated);
    }

    #[test]
    fn non_locked_policy_is_rejected() {
        let r = LockfileResolver::new(doc());
        let err = block_on(r.resolve(req(ResolvePolicy::Latest), &ctx())).unwrap_err();
        assert!(matches!(err, ResolveError::NativeToolFailed { .. }));
    }

    #[test]
    fn budget_marks_truncated() {
        let r = LockfileResolver::new(doc());
        let mut request = req(ResolvePolicy::Locked);
        request.max_nodes = Some(1);
        let g = block_on(r.resolve(request, &ctx())).unwrap();
        assert!(g.truncated);
    }

    #[test]
    fn resolve_now_matches_async() {
        let r = LockfileResolver::new(doc());
        let g = r.resolve_now().unwrap();
        assert_eq!(g.node_count(), 2);
    }
}
