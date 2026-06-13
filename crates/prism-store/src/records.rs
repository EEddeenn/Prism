//! Entity (de)serialization for object-segment values (architecture doc,
//! *Object Segments*).
//!
//! Each entity has a stable [`record_type`](self) discriminant and a deterministic,
//! versioned value encoding (leading `RECORD_ENCODING_VERSION` byte). Only the
//! *value* bytes are produced here; the typed key bytes live in [`crate::index::keys`]
//! and the record framing (header, crc) in `prism-codec`.
//!
//! Encodings are deterministic — the same entity always produces the same bytes —
//! so records and their whole-file digests are reproducible. Decoding is fully
//! bounds-checked and never panics on corrupt input; it returns [`PrismError`].

use prism_codec::{read_uvarint, write_uvarint, ByteReader, ByteWriter};
use prism_model::{
    BlobId, ConditionKind, DependencyKind, EcosystemId, EnvironmentId, FidelityCaveats,
    FidelityTier, GraphFidelity, HashDigest, Package, PackageId, PackageVersion, PrismError,
    Requirement, RequirementId, ResolverRun, Result, RunId, RunStatus, SnapshotId, StringId,
    VersionFlags, VersionId,
};

/// Record-type discriminants for segment values. Stable across versions; see the
/// architecture doc's entity → storage mapping.
pub mod record_type {
    /// A `(StringId -> UTF-8 bytes)` interned atom.
    pub const STRING_ATOM: u16 = 1;
    /// A [`prism_model::Package`] family record.
    pub const PACKAGE: u16 = 2;
    /// A [`prism_model::PackageVersion`] record.
    pub const PACKAGE_VERSION: u16 = 3;
    /// A [`prism_model::Requirement`] record.
    pub const REQUIREMENT: u16 = 4;
    /// A [`prism_model::ResolverRun`] record.
    pub const RESOLVER_RUN: u16 = 5;
}

/// The value-encoding version byte prefixed to every encoded entity value.
pub const RECORD_ENCODING_VERSION: u8 = 1;

// --- small typed helpers (presence-flagged optionals, fixed-width digests) ---

fn put_opt_string(w: &mut ByteWriter, v: Option<StringId>) {
    match v {
        Some(s) => {
            w.write_u8(1);
            w.write_u32(s.get());
        }
        None => w.write_u8(0),
    }
}

fn get_opt_string(r: &mut ByteReader<'_>) -> Result<Option<StringId>> {
    Ok(match r.read_u8()? {
        0 => None,
        1 => Some(StringId::new(r.read_u32()?)),
        other => return Err(bad(format!("opt-string flag {other}"))),
    })
}

fn put_opt_blob(w: &mut ByteWriter, v: Option<BlobId>) {
    match v {
        Some(b) => {
            w.write_u8(1);
            w.write_bytes(b.digest().as_bytes());
        }
        None => w.write_u8(0),
    }
}

fn get_opt_blob(r: &mut ByteReader<'_>) -> Result<Option<BlobId>> {
    Ok(match r.read_u8()? {
        0 => None,
        1 => Some(BlobId(HashDigest::from_bytes(r.read_array()?))),
        other => return Err(bad(format!("opt-blob flag {other}"))),
    })
}

fn put_opt_hash(w: &mut ByteWriter, v: Option<HashDigest>) {
    match v {
        Some(h) => {
            w.write_u8(1);
            w.write_bytes(h.as_bytes());
        }
        None => w.write_u8(0),
    }
}

fn get_opt_hash(r: &mut ByteReader<'_>) -> Result<Option<HashDigest>> {
    Ok(match r.read_u8()? {
        0 => None,
        1 => Some(HashDigest::from_bytes(r.read_array()?)),
        other => return Err(bad(format!("opt-hash flag {other}"))),
    })
}

fn put_opt_i64(w: &mut ByteWriter, v: Option<i64>) {
    match v {
        Some(n) => {
            w.write_u8(1);
            w.write_i64(n);
        }
        None => w.write_u8(0),
    }
}

fn get_opt_i64(r: &mut ByteReader<'_>) -> Result<Option<i64>> {
    Ok(match r.read_u8()? {
        0 => None,
        1 => Some(r.read_i64()?),
        other => return Err(bad(format!("opt-i64 flag {other}"))),
    })
}

fn put_opt_pkg(w: &mut ByteWriter, v: Option<PackageId>) {
    match v {
        Some(p) => {
            w.write_u8(1);
            w.write_u64(p.get());
        }
        None => w.write_u8(0),
    }
}

fn get_opt_pkg(r: &mut ByteReader<'_>) -> Result<Option<PackageId>> {
    Ok(match r.read_u8()? {
        0 => None,
        1 => Some(PackageId::new(r.read_u64()?)),
        other => return Err(bad(format!("opt-pkg flag {other}"))),
    })
}

fn put_bytes(w: &mut ByteWriter, b: &[u8]) {
    write_uvarint(w, b.len() as u64);
    w.write_bytes(b);
}

fn get_bytes<'a>(r: &mut ByteReader<'a>) -> Result<&'a [u8]> {
    let n = read_uvarint(r)? as usize;
    r.read_bytes(n)
}

fn bad(detail: impl Into<String>) -> PrismError {
    PrismError::Corrupt {
        detail: detail.into(),
    }
}

fn check_version(r: &mut ByteReader<'_>) -> Result<()> {
    let v = r.read_u8()?;
    if v != RECORD_ENCODING_VERSION {
        return Err(PrismError::UnsupportedFormatVersion {
            found: v as u32,
            max: RECORD_ENCODING_VERSION as u32,
        });
    }
    Ok(())
}

// --- enum <-> u8 mappings ---

fn dep_kind_u8(k: DependencyKind) -> u8 {
    match k {
        DependencyKind::Runtime => 0,
        DependencyKind::Dev => 1,
        DependencyKind::Build => 2,
        DependencyKind::Peer => 3,
        DependencyKind::Optional => 4,
        DependencyKind::Test => 5,
        DependencyKind::Plugin => 6,
        DependencyKind::Workspace => 7,
    }
}

fn dep_kind_from(v: u8) -> Result<DependencyKind> {
    Ok(match v {
        0 => DependencyKind::Runtime,
        1 => DependencyKind::Dev,
        2 => DependencyKind::Build,
        3 => DependencyKind::Peer,
        4 => DependencyKind::Optional,
        5 => DependencyKind::Test,
        6 => DependencyKind::Plugin,
        7 => DependencyKind::Workspace,
        other => return Err(bad(format!("dependency kind {other}"))),
    })
}

fn cond_kind_u8(k: ConditionKind) -> u8 {
    match k {
        ConditionKind::None => 0,
        ConditionKind::Environment => 1,
        ConditionKind::Activation => 2,
    }
}

fn cond_kind_from(v: u8) -> Result<ConditionKind> {
    Ok(match v {
        0 => ConditionKind::None,
        1 => ConditionKind::Environment,
        2 => ConditionKind::Activation,
        other => return Err(bad(format!("condition kind {other}"))),
    })
}

fn run_status_u8(s: RunStatus) -> u8 {
    match s {
        RunStatus::Pending => 0,
        RunStatus::Complete => 1,
        RunStatus::Truncated => 2,
        RunStatus::Failed => 3,
    }
}

fn run_status_from(v: u8) -> Result<RunStatus> {
    Ok(match v {
        0 => RunStatus::Pending,
        1 => RunStatus::Complete,
        2 => RunStatus::Truncated,
        3 => RunStatus::Failed,
        other => return Err(bad(format!("run status {other}"))),
    })
}

fn fidelity_tier_u8(t: FidelityTier) -> u8 {
    match t {
        FidelityTier::LockfileExact => 0,
        FidelityTier::NativeToolResolved => 1,
        FidelityTier::ModelResolved => 2,
        FidelityTier::CandidateSpaceOnly => 3,
    }
}

fn fidelity_tier_from(v: u8) -> Result<FidelityTier> {
    Ok(match v {
        0 => FidelityTier::LockfileExact,
        1 => FidelityTier::NativeToolResolved,
        2 => FidelityTier::ModelResolved,
        3 => FidelityTier::CandidateSpaceOnly,
        other => return Err(bad(format!("fidelity tier {other}"))),
    })
}

// --- StringAtom ---

/// Encode an interned atom's value bytes (just the UTF-8 string; the [`StringId`]
/// is the key).
pub fn encode_string_atom(s: &str) -> Vec<u8> {
    let mut w = ByteWriter::new();
    w.write_u8(RECORD_ENCODING_VERSION);
    put_bytes(&mut w, s.as_bytes());
    w.into_vec()
}

/// Decode an interned atom's value bytes back to a [`String`].
pub fn decode_string_atom(bytes: &[u8]) -> Result<String> {
    let mut r = ByteReader::new(bytes);
    check_version(&mut r)?;
    let b = get_bytes(&mut r)?;
    core::str::from_utf8(b)
        .map(|s| s.to_owned())
        .map_err(|_| bad("non-utf8 string atom"))
}

// --- Package ---

/// Encode a [`Package`]'s value bytes (its [`PackageId`] is the key).
pub fn encode_package(p: &Package) -> Vec<u8> {
    let mut w = ByteWriter::new();
    w.write_u8(RECORD_ENCODING_VERSION);
    w.write_u64(p.id.get());
    w.write_u32(p.ecosystem.get());
    put_opt_string(&mut w, p.namespace);
    w.write_u32(p.name.get());
    w.write_u32(p.canonical_name.get());
    w.write_u32(p.purl_base.get());
    put_opt_string(&mut w, p.repository_url);
    put_opt_string(&mut w, p.homepage_url);
    put_opt_blob(&mut w, p.metadata_ref);
    w.into_vec()
}

/// Decode a [`Package`] from its value bytes.
pub fn decode_package(bytes: &[u8]) -> Result<Package> {
    let mut r = ByteReader::new(bytes);
    check_version(&mut r)?;
    Ok(Package {
        id: PackageId::new(r.read_u64()?),
        ecosystem: EcosystemId::new(r.read_u32()?),
        namespace: get_opt_string(&mut r)?,
        name: StringId::new(r.read_u32()?),
        canonical_name: StringId::new(r.read_u32()?),
        purl_base: StringId::new(r.read_u32()?),
        repository_url: get_opt_string(&mut r)?,
        homepage_url: get_opt_string(&mut r)?,
        metadata_ref: get_opt_blob(&mut r)?,
    })
}

// --- PackageVersion ---

/// Encode a [`PackageVersion`]'s value bytes (its [`VersionId`] is the key).
pub fn encode_package_version(v: &PackageVersion) -> Vec<u8> {
    let mut w = ByteWriter::new();
    w.write_u8(RECORD_ENCODING_VERSION);
    w.write_u64(v.id.get());
    w.write_u64(v.package_id.get());
    w.write_u32(v.version_raw.get());
    w.write_u32(v.version_norm.get());
    put_bytes(&mut w, &v.version_sort_key);
    w.write_u32(v.purl.get());
    put_opt_i64(&mut w, v.released_at_unix);
    w.write_u32(v.flags.bits());
    put_opt_string(&mut w, v.license_expr);
    put_opt_hash(&mut w, v.artifact_hash);
    put_opt_blob(&mut w, v.metadata_ref);
    w.into_vec()
}

/// Decode a [`PackageVersion`] from its value bytes.
pub fn decode_package_version(bytes: &[u8]) -> Result<PackageVersion> {
    let mut r = ByteReader::new(bytes);
    check_version(&mut r)?;
    let id = VersionId::new(r.read_u64()?);
    let package_id = PackageId::new(r.read_u64()?);
    let version_raw = StringId::new(r.read_u32()?);
    let version_norm = StringId::new(r.read_u32()?);
    let version_sort_key = get_bytes(&mut r)?.to_vec();
    let purl = StringId::new(r.read_u32()?);
    let released_at_unix = get_opt_i64(&mut r)?;
    let flags = VersionFlags::from_bits_truncate(r.read_u32()?);
    let license_expr = get_opt_string(&mut r)?;
    let artifact_hash = get_opt_hash(&mut r)?;
    let metadata_ref = get_opt_blob(&mut r)?;
    Ok(PackageVersion {
        id,
        package_id,
        version_raw,
        version_norm,
        version_sort_key,
        purl,
        released_at_unix,
        flags,
        license_expr,
        artifact_hash,
        metadata_ref,
    })
}

// --- Requirement ---

/// Encode a [`Requirement`]'s value bytes (its [`RequirementId`] is the key).
pub fn encode_requirement(req: &Requirement) -> Vec<u8> {
    let mut w = ByteWriter::new();
    w.write_u8(RECORD_ENCODING_VERSION);
    w.write_u64(req.id.get());
    w.write_u64(req.src_version_id.get());
    w.write_u32(req.declared_target.get());
    put_opt_pkg(&mut w, req.dst_package_id);
    put_opt_string(&mut w, req.target_alias);
    w.write_u32(req.raw_requirement.get());
    w.write_bytes(req.constraint_ref.digest().as_bytes());
    put_opt_blob(&mut w, req.managed_version_ref);
    w.write_u8(dep_kind_u8(req.kind));
    put_opt_string(&mut w, req.scope);
    w.write_u8(cond_kind_u8(req.condition_kind));
    put_opt_blob(&mut w, req.condition_ref);
    put_opt_blob(&mut w, req.features_or_extras_ref);
    put_opt_blob(&mut w, req.metadata_ref);
    w.into_vec()
}

/// Decode a [`Requirement`] from its value bytes.
pub fn decode_requirement(bytes: &[u8]) -> Result<Requirement> {
    let mut r = ByteReader::new(bytes);
    check_version(&mut r)?;
    let id = RequirementId::new(r.read_u64()?);
    let src_version_id = VersionId::new(r.read_u64()?);
    let declared_target = StringId::new(r.read_u32()?);
    let dst_package_id = get_opt_pkg(&mut r)?;
    let target_alias = get_opt_string(&mut r)?;
    let raw_requirement = StringId::new(r.read_u32()?);
    let constraint_ref = BlobId(HashDigest::from_bytes(r.read_array()?));
    let managed_version_ref = get_opt_blob(&mut r)?;
    let kind = dep_kind_from(r.read_u8()?)?;
    let scope = get_opt_string(&mut r)?;
    let condition_kind = cond_kind_from(r.read_u8()?)?;
    let condition_ref = get_opt_blob(&mut r)?;
    let features_or_extras_ref = get_opt_blob(&mut r)?;
    let metadata_ref = get_opt_blob(&mut r)?;
    Ok(Requirement {
        id,
        src_version_id,
        declared_target,
        dst_package_id,
        target_alias,
        raw_requirement,
        constraint_ref,
        managed_version_ref,
        kind,
        scope,
        condition_kind,
        condition_ref,
        features_or_extras_ref,
        metadata_ref,
    })
}

// --- ResolverRun ---

/// Encode a [`ResolverRun`]'s value bytes (its [`RunId`] is the key).
pub fn encode_resolver_run(run: &ResolverRun) -> Vec<u8> {
    let mut w = ByteWriter::new();
    w.write_u8(RECORD_ENCODING_VERSION);
    w.write_u64(run.id.get());
    w.write_u32(run.ecosystem.get());
    w.write_bytes(run.root_set_hash.as_bytes());
    w.write_u32(run.resolver_name.get());
    put_opt_string(&mut w, run.resolver_version);
    w.write_u64(run.environment_id.get());
    w.write_u64(run.snapshot_id.get());
    put_opt_blob(&mut w, run.input_manifest_ref);
    put_opt_blob(&mut w, run.input_lockfile_ref);
    w.write_u8(run_status_u8(run.status));
    w.write_u8(fidelity_tier_u8(run.fidelity.tier));
    w.write_u32(run.fidelity.caveats.bits());
    w.write_i64(run.reference_unix);
    w.write_i64(run.created_at_unix);
    put_opt_blob(&mut w, run.metadata_ref);
    w.into_vec()
}

/// Decode a [`ResolverRun`] from its value bytes.
pub fn decode_resolver_run(bytes: &[u8]) -> Result<ResolverRun> {
    let mut r = ByteReader::new(bytes);
    check_version(&mut r)?;
    let id = RunId::new(r.read_u64()?);
    let ecosystem = EcosystemId::new(r.read_u32()?);
    let root_set_hash = HashDigest::from_bytes(r.read_array()?);
    let resolver_name = StringId::new(r.read_u32()?);
    let resolver_version = get_opt_string(&mut r)?;
    let environment_id = EnvironmentId::new(r.read_u64()?);
    let snapshot_id = SnapshotId::new(r.read_u64()?);
    let input_manifest_ref = get_opt_blob(&mut r)?;
    let input_lockfile_ref = get_opt_blob(&mut r)?;
    let status = run_status_from(r.read_u8()?)?;
    let tier = fidelity_tier_from(r.read_u8()?)?;
    let caveats = FidelityCaveats::from_bits_truncate(r.read_u32()?);
    let reference_unix = r.read_i64()?;
    let created_at_unix = r.read_i64()?;
    let metadata_ref = get_opt_blob(&mut r)?;
    Ok(ResolverRun {
        id,
        ecosystem,
        root_set_hash,
        resolver_name,
        resolver_version,
        environment_id,
        snapshot_id,
        input_manifest_ref,
        input_lockfile_ref,
        status,
        fidelity: GraphFidelity { tier, caveats },
        reference_unix,
        created_at_unix,
        metadata_ref,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_codec::sha256;

    fn dig(seed: &[u8]) -> HashDigest {
        sha256(seed)
    }

    #[test]
    fn string_atom_roundtrip() {
        let bytes = encode_string_atom("npm:react");
        assert_eq!(decode_string_atom(&bytes).unwrap(), "npm:react");
        // deterministic
        assert_eq!(bytes, encode_string_atom("npm:react"));
        assert_eq!(decode_string_atom(&encode_string_atom("")).unwrap(), "");
    }

    #[test]
    fn package_roundtrip() {
        let p = Package {
            id: PackageId::new(7),
            ecosystem: EcosystemId::new(2),
            namespace: Some(StringId::new(11)),
            name: StringId::new(12),
            canonical_name: StringId::new(13),
            purl_base: StringId::new(14),
            repository_url: None,
            homepage_url: Some(StringId::new(15)),
            metadata_ref: Some(BlobId(dig(b"meta"))),
        };
        let bytes = encode_package(&p);
        assert_eq!(decode_package(&bytes).unwrap(), p);
        assert_eq!(bytes, encode_package(&p)); // deterministic
    }

    #[test]
    fn package_version_roundtrip() {
        let v = PackageVersion {
            id: VersionId::new(100),
            package_id: PackageId::new(7),
            version_raw: StringId::new(1),
            version_norm: StringId::new(2),
            version_sort_key: vec![0, 1, 2, 255, 7],
            purl: StringId::new(3),
            released_at_unix: Some(1_700_000_000),
            flags: VersionFlags::YANKED | VersionFlags::PRERELEASE,
            license_expr: Some(StringId::new(9)),
            artifact_hash: Some(dig(b"artifact")),
            metadata_ref: None,
        };
        let bytes = encode_package_version(&v);
        assert_eq!(decode_package_version(&bytes).unwrap(), v);
    }

    #[test]
    fn requirement_roundtrip() {
        let req = Requirement {
            id: RequirementId::new(5),
            src_version_id: VersionId::new(100),
            declared_target: StringId::new(20),
            dst_package_id: Some(PackageId::new(8)),
            target_alias: None,
            raw_requirement: StringId::new(21),
            constraint_ref: BlobId(dig(b"constraint")),
            managed_version_ref: Some(BlobId(dig(b"managed"))),
            kind: DependencyKind::Build,
            scope: Some(StringId::new(22)),
            condition_kind: ConditionKind::Activation,
            condition_ref: Some(BlobId(dig(b"cond"))),
            features_or_extras_ref: None,
            metadata_ref: None,
        };
        let bytes = encode_requirement(&req);
        assert_eq!(decode_requirement(&bytes).unwrap(), req);
    }

    #[test]
    fn resolver_run_roundtrip() {
        let run = ResolverRun {
            id: RunId::new(3),
            ecosystem: EcosystemId::new(1),
            root_set_hash: dig(b"roots"),
            resolver_name: StringId::new(30),
            resolver_version: Some(StringId::new(31)),
            environment_id: EnvironmentId::new(444),
            snapshot_id: SnapshotId::new(555),
            input_manifest_ref: Some(BlobId(dig(b"manifest"))),
            input_lockfile_ref: None,
            status: RunStatus::Complete,
            fidelity: GraphFidelity {
                tier: FidelityTier::ModelResolved,
                caveats: FidelityCaveats::NO_PEER_PLACEMENT | FidelityCaveats::MARKERS_APPROXIMATED,
            },
            reference_unix: 1_600_000_000,
            created_at_unix: 1_700_000_000,
            metadata_ref: None,
        };
        let bytes = encode_resolver_run(&run);
        assert_eq!(decode_resolver_run(&bytes).unwrap(), run);
    }

    #[test]
    fn truncated_value_is_error_not_panic() {
        let p = Package {
            id: PackageId::new(1),
            ecosystem: EcosystemId::new(1),
            namespace: None,
            name: StringId::new(1),
            canonical_name: StringId::new(1),
            purl_base: StringId::new(1),
            repository_url: None,
            homepage_url: None,
            metadata_ref: None,
        };
        let bytes = encode_package(&p);
        for cut in 0..bytes.len() {
            // Any truncation must be an error, never a panic.
            assert!(decode_package(&bytes[..cut]).is_err());
        }
    }

    #[test]
    fn wrong_encoding_version_is_rejected() {
        let mut bytes = encode_string_atom("x");
        bytes[0] = 99;
        assert!(matches!(
            decode_string_atom(&bytes),
            Err(PrismError::UnsupportedFormatVersion { .. })
        ));
    }
}
