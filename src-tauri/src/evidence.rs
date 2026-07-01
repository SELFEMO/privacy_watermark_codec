use std::{fs, io, path::Path};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use watermark_core::WatermarkKey;

use crate::{models::MediaType, release::ReleaseMetadata};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceEntry {
    pub input_path: String,
    pub output_path: String,
    pub media_type: MediaType,
    pub original_sha256: String,
    pub output_sha256: String,
    pub key_file_sha256: Option<String>,
    pub psnr: Option<f64>,
    pub frame_count: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnsignedEvidenceManifest {
    generated_at: String,
    hash_algorithm: String,
    signature_algorithm: String,
    release: ReleaseMetadata,
    entries: Vec<EvidenceEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EvidenceManifest {
    generated_at: String,
    hash_algorithm: String,
    signature_algorithm: String,
    release: ReleaseMetadata,
    entries: Vec<EvidenceEntry>,
    manifest_signature: String,
}

pub fn file_sha256(path: &Path) -> io::Result<String> {
    let bytes = fs::read(path)?;
    Ok(hex_sha256(&bytes))
}

pub fn write_evidence_manifest(
    path: &Path,
    entries: Vec<EvidenceEntry>,
    signing_key: &WatermarkKey,
    release: ReleaseMetadata,
) -> io::Result<()> {
    let unsigned = UnsignedEvidenceManifest {
        generated_at: Utc::now().to_rfc3339(),
        hash_algorithm: "sha256".into(),
        signature_algorithm: "key-bound-sha256".into(),
        release,
        entries,
    };
    let unsigned_json = serde_json::to_vec(&unsigned).map_err(json_error)?;
    let mut material = unsigned_json.clone();
    material.extend_from_slice(&signing_key.derived_key);
    let manifest = EvidenceManifest {
        generated_at: unsigned.generated_at,
        hash_algorithm: unsigned.hash_algorithm,
        signature_algorithm: unsigned.signature_algorithm,
        release: unsigned.release,
        entries: unsigned.entries,
        manifest_signature: hex_sha256(&material),
    };
    let content = serde_json::to_string_pretty(&manifest).map_err(json_error)?;

    // 清单签名绑定派生密钥而不写出密钥本身，使原文件哈希、输出哈希与持钥者形成可验证的证据链。
    // The manifest signature is bound to the derived key without writing the key, linking original hashes, output hashes, and key possession into an evidence chain.
    fs::write(path, content)
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

fn json_error(error: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}
