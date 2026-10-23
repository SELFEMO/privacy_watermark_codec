use std::{
    fs,
    io::{self, Read},
    path::Path,
};

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
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    // 证据摘要与分块方式无关，固定缓冲区可以避免为大视频额外分配整文件大小的内存。
    // Evidence digests are independent of chunk boundaries, so a fixed buffer avoids an extra whole-video allocation.
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceVerifyResult {
    pub status: String,
}

pub fn verify_evidence_manifest(
    path: &Path,
    key: &WatermarkKey,
) -> io::Result<EvidenceVerifyResult> {
    let content = fs::read_to_string(path)?;
    let manifest: EvidenceManifest = serde_json::from_str(&content).map_err(json_error)?;
    let observed_signature = manifest.manifest_signature.clone();
    let unsigned = UnsignedEvidenceManifest {
        generated_at: manifest.generated_at,
        hash_algorithm: manifest.hash_algorithm,
        signature_algorithm: manifest.signature_algorithm,
        release: manifest.release,
        entries: manifest.entries,
    };
    let unsigned_json = serde_json::to_vec(&unsigned).map_err(json_error)?;
    let mut material = unsigned_json;
    material.extend_from_slice(&key.derived_key);
    let expected = hex_sha256(&material);
    let status = if expected == observed_signature {
        "ok"
    } else {
        "mismatch"
    };
    // 核验只返回 ok/mismatch，不回传密钥或清单签名材料，避免把持钥校验码变成密钥泄漏通道。
    // Verification returns only ok/mismatch and never echoes the key or signature material, so the check cannot become a key leak.
    Ok(EvidenceVerifyResult {
        status: status.into(),
    })
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
