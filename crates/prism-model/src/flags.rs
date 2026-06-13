//! Bitflag sets used across the model (no external `bitflags` dependency).

bitflag_set! {
    /// Per-version status flags.
    pub struct VersionFlags: u32 {
        const YANKED      = 1 << 0;
        const DEPRECATED  = 1 << 1;
        const PRERELEASE  = 1 << 2;
        const PSEUDO      = 1 << 3; // Go pseudo-version
    }
}

bitflag_set! {
    /// Per-graph-node flags.
    pub struct NodeFlags: u32 {
        const ROOT        = 1 << 0;
        const DUPLICATE   = 1 << 1; // one of several versions of a family
        const TRUNCATED   = 1 << 2; // expansion cut by a budget
    }
}

bitflag_set! {
    /// Resolution-outcome flags on a resolved edge.
    pub struct EdgeFlags: u32 {
        const RESOLVED         = 1 << 0;
        const OPTIONAL_OMITTED = 1 << 1;
        const PEER_UNMET       = 1 << 2;
        const OVERRIDDEN       = 1 << 3;
        const BACK_EDGE        = 1 << 4; // closes a cycle to an existing occurrence
    }
}

bitflag_set! {
    /// Capabilities advertised by an ecosystem plugin.
    pub struct EcosystemCapabilities: u64 {
        const PARSE_MANIFEST      = 1 << 0;
        const PARSE_LOCKFILE      = 1 << 1;
        const FETCH_REGISTRY      = 1 << 2;
        const FETCH_ARTIFACT_META = 1 << 3;
        const ENUM_CANDIDATES     = 1 << 4;
        const NATIVE_RESOLVE      = 1 << 5;
        const SYNTHETIC_RESOLVE   = 1 << 6;
        const SUPPORTS_WORKSPACE  = 1 << 7;
        const SUPPORTS_FEATURES   = 1 << 8;
        const SUPPORTS_PLATFORM   = 1 << 9;
        const SUPPORTS_YANKED     = 1 << 10;
    }
}

bitflag_set! {
    /// "What was dropped" caveats attached to a run's [`crate::GraphFidelity`].
    pub struct FidelityCaveats: u32 {
        const NO_PEER_PLACEMENT      = 1 << 0;
        const NO_FEATURE_UNIFICATION = 1 << 1;
        const NO_PROFILE_EXPANSION   = 1 << 2;
        const NO_MEDIATION           = 1 << 3;
        const NO_EXTRA_ACTIVATION    = 1 << 4;
        const MARKERS_APPROXIMATED   = 1 << 5;
        const DYNAMIC_METADATA_SKIPPED = 1 << 6;
        const BUDGET_TRUNCATED       = 1 << 7;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_algebra() {
        let mut f = EdgeFlags::RESOLVED;
        assert!(f.contains(EdgeFlags::RESOLVED));
        assert!(!f.contains(EdgeFlags::OVERRIDDEN));
        f.insert(EdgeFlags::OVERRIDDEN);
        assert!(f.contains(EdgeFlags::OVERRIDDEN));
        assert!(f.intersects(EdgeFlags::RESOLVED | EdgeFlags::PEER_UNMET));
        f.remove(EdgeFlags::RESOLVED);
        assert!(!f.contains(EdgeFlags::RESOLVED));
        assert!(f.contains(EdgeFlags::OVERRIDDEN));
    }

    #[test]
    fn from_bits_truncate_drops_unknown() {
        // Set a bit far outside any named flag; it must be discarded.
        let raw = VersionFlags::YANKED.bits() | (1 << 30);
        let f = VersionFlags::from_bits_truncate(raw);
        assert_eq!(f, VersionFlags::YANKED);
        assert_eq!(f.bits() & (1 << 30), 0, "unknown bit must be dropped");
    }

    #[test]
    fn empty_and_all() {
        assert!(NodeFlags::EMPTY.is_empty());
        assert!(NodeFlags::ALL.contains(NodeFlags::ROOT));
        assert!(NodeFlags::ALL.contains(NodeFlags::TRUNCATED));
    }

    #[test]
    fn debug_lists_names() {
        let f = EdgeFlags::RESOLVED | EdgeFlags::BACK_EDGE;
        let s = format!("{f:?}");
        assert!(s.contains("RESOLVED"), "{s}");
        assert!(s.contains("BACK_EDGE"), "{s}");
        assert_eq!(format!("{:?}", EdgeFlags::EMPTY), "EdgeFlags(EMPTY)");
    }
}
