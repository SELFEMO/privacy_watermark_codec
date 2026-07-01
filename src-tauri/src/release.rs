use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleasePackageSigningMetadata {
    pub signature_algorithm: String,
    pub signer: String,
    pub signature: String,
    pub artifact_sha256: String,
    pub manifest_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseMetadata {
    pub automatic_update: bool,
    pub manifest_url: String,
    pub package_signing: ReleasePackageSigningMetadata,
}

#[tauri::command]
pub fn get_release_metadata() -> ReleaseMetadata {
    current_release_metadata()
}

pub fn current_release_metadata() -> ReleaseMetadata {
    let manifest_url = option_env!("PWC_UPDATE_MANIFEST_URL")
        .unwrap_or_default()
        .trim()
        .to_owned();
    let signature = option_env!("PWC_RELEASE_SIGNATURE")
        .unwrap_or_default()
        .trim()
        .to_owned();
    let artifact_sha256 = option_env!("PWC_RELEASE_ARTIFACT_SHA256")
        .unwrap_or_default()
        .trim()
        .to_owned();
    let manifest_sha256 = option_env!("PWC_RELEASE_MANIFEST_SHA256")
        .unwrap_or_default()
        .trim()
        .to_owned();
    let signer = option_env!("PWC_RELEASE_SIGNER")
        .unwrap_or("local-build")
        .trim()
        .to_owned();

    // 只有同时声明更新清单地址和发布包签名时才允许前端展示自动更新入口，避免未签名包被误当作可信更新源。
    // Automatic update is exposed only when both a manifest URL and package signature are declared, preventing unsigned packages from being treated as trusted updates.
    ReleaseMetadata {
        automatic_update: !manifest_url.is_empty() && !signature.is_empty(),
        manifest_url,
        package_signing: ReleasePackageSigningMetadata {
            signature_algorithm: "detached-signature-sha256".into(),
            signer,
            signature,
            artifact_sha256,
            manifest_sha256,
        },
    }
}
