//! Narinfo construction via nix-narinfo's builder API.
//!
//! Assembles canonical narinfo text from computed fields (NAR hash, size,
//! references, compression, file hash/size) with correct Nix field order
//! and canonical hash encodings — no hand-rolled format strings.

use anyhow::{anyhow, Result};
use nix_derivation::{NixHash, StoreDir, StorePath};
use nix_narinfo::{Compression, NarInfoBuilder, NarInfoSignature};

/// A complete narinfo ready for signing and upload.
pub struct NarInfoOutput {
    /// Canonical narinfo text bytes (unsigned)
    pub bytes: Vec<u8>,
    /// The fingerprint string that gets signed
    pub fingerprint: String,
}

/// The computed fields of one narinfo record. Everything except the
/// signatures, which are attached separately: an unsigned build is
/// needed first to compute the fingerprint they sign.
pub struct NarInfoInput<'a> {
    /// Logical store directory (usually `/nix/store`)
    pub store_dir: &'a StoreDir,
    /// Parsed store path (hashpart + name)
    pub store_path: &'a StorePath,
    /// Relative URL to the NAR blob (e.g. `nar/<nixbase32>.nar.xz`)
    pub url: &'a str,
    /// SHA-256 of uncompressed NAR bytes
    pub nar_hash: [u8; 32],
    /// Byte length of uncompressed NAR
    pub nar_size: u64,
    /// Compression format (Xz, Zstd, etc.)
    pub compression: Compression,
    /// SHA-256 of compressed bytes
    pub file_hash: [u8; 32],
    /// Byte length of compressed NAR
    pub file_size: u64,
    /// Store paths referenced by this path
    pub references: &'a [StorePath],
}

/// Build a narinfo record from computed fields.
pub fn build_narinfo(
    input: &NarInfoInput,
    signatures: &[NarInfoSignature],
) -> Result<NarInfoOutput> {
    let mut builder = NarInfoBuilder::new_in(
        input.store_dir.clone(),
        input.store_path.clone(),
        input.url.to_string(),
        NixHash::Sha256(input.nar_hash),
        input.nar_size,
    )
    .compression(input.compression.clone())
    .file_hash(Some(NixHash::Sha256(input.file_hash)))
    .file_size(Some(input.file_size));

    if !input.references.is_empty() {
        builder = builder.references(input.references.to_vec());
    }

    for sig in signatures {
        builder = builder.signature(sig.clone());
    }

    let info = builder
        .build()
        .map_err(|e| anyhow!("narinfo build error: {}", e))?;

    let fingerprint = info.fingerprint(input.store_dir);
    let bytes = info.to_canonical_bytes();

    Ok(NarInfoOutput { bytes, fingerprint })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix_narinfo::NarInfo;

    #[test]
    fn build_minimal_narinfo() {
        let store_dir = StoreDir::default();
        let store_path =
            StorePath::from_basename(b"00000000000000000000000000000000-example").unwrap();

        let input = NarInfoInput {
            store_dir: &store_dir,
            store_path: &store_path,
            url: "nar/example.nar.xz",
            nar_hash: [0x42; 32],
            nar_size: 1024,
            compression: Compression::Xz,
            file_hash: [0x43; 32],
            file_size: 512,
            references: &[],
        };
        let output = build_narinfo(&input, &[]).unwrap();

        // Canonical bytes should contain the store path
        let text = std::str::from_utf8(&output.bytes).unwrap();
        assert!(text.contains("StorePath: /nix/store/00000000000000000000000000000000-example"));
        assert!(text.contains("URL: nar/example.nar.xz"));
        assert!(text.contains("NarSize: 1024"));
        assert!(text.contains("FileSize: 512"));
        assert!(text.contains("Compression: xz"));

        // Fingerprint starts with "1;" + store path
        assert!(output
            .fingerprint
            .starts_with("1;/nix/store/00000000000000000000000000000000-example"));
    }

    #[test]
    fn roundtrip_parse_built_narinfo() {
        let store_dir = StoreDir::default();
        let store_path =
            StorePath::from_basename(b"00000000000000000000000000000000-test").unwrap();

        let input = NarInfoInput {
            store_dir: &store_dir,
            store_path: &store_path,
            url: "nar/test.nar.xz",
            nar_hash: [0xAA; 32],
            nar_size: 4096,
            compression: Compression::Xz,
            file_hash: [0xBB; 32],
            file_size: 2048,
            references: &[],
        };
        let output = build_narinfo(&input, &[]).unwrap();

        // Parse it back
        let parsed = NarInfo::parse_in(&store_dir, &output.bytes).unwrap();
        assert_eq!(parsed.url(), "nar/test.nar.xz");
        assert_eq!(parsed.nar_size(), 4096);
        assert_eq!(parsed.compression(), &Compression::Xz);
    }
}
