use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use ring::{digest, signature};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::io;

pub const ROOT_PUBLIC_KEY: &[u8; 32] = include_bytes!("../updates/root.pub");
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
pub const MAX_TARGET_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseManifest {
    pub schema_version: u32,
    pub version: String,
    pub issued_unix_secs: u64,
    pub expires_unix_secs: u64,
    pub targets: Vec<ReleaseTarget>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseTarget {
    pub platform: String,
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

pub fn platform() -> String {
    format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH)
}

pub fn verify_manifest(
    manifest: &[u8],
    signature_b64: &[u8],
    public_key: &[u8],
    now: u64,
    current: &Version,
    target_platform: &str,
) -> io::Result<Option<(Version, ReleaseTarget)>> {
    if manifest.len() > MAX_MANIFEST_BYTES {
        return Err(invalid("release manifest too large"));
    }
    let signature = STANDARD
        .decode(String::from_utf8_lossy(signature_b64).trim())
        .map_err(|_| invalid("invalid release signature encoding"))?;
    signature::UnparsedPublicKey::new(&signature::ED25519, public_key)
        .verify(manifest, &signature)
        .map_err(|_| invalid("release signature verification failed"))?;
    let release: ReleaseManifest =
        serde_json::from_slice(manifest).map_err(|_| invalid("invalid release manifest"))?;
    if release.schema_version != 1
        || release.issued_unix_secs > now.saturating_add(300)
        || release.expires_unix_secs <= now
        || release.expires_unix_secs > release.issued_unix_secs.saturating_add(30 * 24 * 3600)
        || release.targets.len() > 16
    {
        return Err(invalid("release metadata is stale or invalid"));
    }
    let version =
        Version::parse(&release.version).map_err(|_| invalid("invalid release version"))?;
    if !version.pre.is_empty() || !version.build.is_empty() {
        return Err(invalid("only stable release versions are supported"));
    }
    if &version <= current {
        return Ok(None);
    }
    let Some(target) = release
        .targets
        .into_iter()
        .find(|item| item.platform == target_platform)
    else {
        return Ok(None);
    };
    if target.size == 0
        || target.size > MAX_TARGET_BYTES
        || target.path != format!("saml-agent/{target_platform}/{version}")
        || target.sha256.len() != 64
        || !target
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid("invalid release target"));
    }
    Ok(Some((version, target)))
}

pub fn verify_target(bytes: &[u8], target: &ReleaseTarget) -> io::Result<()> {
    if bytes.len() as u64 != target.size || sha256_hex(bytes) != target.sha256 {
        return Err(invalid("release target hash or size mismatch"));
    }
    Ok(())
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let hash = digest::digest(&digest::SHA256, bytes);
    hash.as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::rand::SystemRandom;
    use ring::signature::KeyPair;
    use std::fs;
    use std::path::Path;

    #[test]
    fn signed_release_rejects_tampering_expiry_and_rollback() {
        let key = signature::Ed25519KeyPair::from_pkcs8(
            signature::Ed25519KeyPair::generate_pkcs8(&SystemRandom::new())
                .unwrap()
                .as_ref(),
        )
        .unwrap();
        let artifact = b"agent executable";
        let manifest = ReleaseManifest {
            schema_version: 1,
            version: "0.3.3".into(),
            issued_unix_secs: 1000,
            expires_unix_secs: 2000,
            targets: vec![ReleaseTarget {
                platform: "macos/aarch64".into(),
                path: "saml-agent/macos/aarch64/0.3.3".into(),
                size: artifact.len() as u64,
                sha256: sha256_hex(artifact),
            }],
        };
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let signed = STANDARD.encode(key.sign(&bytes).as_ref());
        let current = Version::parse("0.3.2").unwrap();
        let result = verify_manifest(
            &bytes,
            signed.as_bytes(),
            key.public_key().as_ref(),
            1100,
            &current,
            "macos/aarch64",
        )
        .unwrap();
        verify_target(artifact, &result.unwrap().1).unwrap();
        assert!(verify_target(b"bad executable", &manifest.targets[0]).is_err());
        assert!(verify_manifest(
            &bytes,
            signed.as_bytes(),
            key.public_key().as_ref(),
            2000,
            &current,
            "macos/aarch64"
        )
        .is_err());
        assert!(verify_manifest(
            &bytes,
            signed.as_bytes(),
            key.public_key().as_ref(),
            1100,
            &Version::parse("0.3.3").unwrap(),
            "macos/aarch64"
        )
        .unwrap()
        .is_none());
        let mut tampered = bytes.clone();
        tampered[10] ^= 1;
        assert!(verify_manifest(
            &tampered,
            signed.as_bytes(),
            key.public_key().as_ref(),
            1100,
            &current,
            "macos/aarch64"
        )
        .is_err());
    }

    #[test]
    fn published_release_matches_pinned_root_and_artifact() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("updates");
        let manifest = fs::read(root.join("metadata/release.json")).unwrap();
        let signature = fs::read(root.join("metadata/release.sig")).unwrap();
        let release: ReleaseManifest = serde_json::from_slice(&manifest).unwrap();
        let (version, target) = verify_manifest(
            &manifest,
            &signature,
            ROOT_PUBLIC_KEY,
            release.issued_unix_secs + 1,
            &Version::parse("0.0.0").unwrap(),
            "macos/aarch64",
        )
        .unwrap()
        .unwrap();
        assert_eq!(version.to_string(), release.version);
        verify_target(
            &fs::read(root.join("targets").join(&target.path)).unwrap(),
            &target,
        )
        .unwrap();
    }
}
