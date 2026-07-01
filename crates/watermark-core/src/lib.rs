pub mod crypto;
pub mod dct;
pub mod error;
pub mod fingerprint;
pub mod hamming;
pub mod keyfile;
pub mod payload;
pub mod scan;
pub mod watermark;

pub use error::{CoreError, Result};
pub use keyfile::{KeyFile, KeyMode, KeySource, WatermarkKey};
pub use scan::{scan_image_file, PrivacyScanDetection, PrivacyScanReport, PrivacyScanStatus};
pub use watermark::{
    embed_image_file, extract_image_file, EmbedOptions, EmbedReport, ExtractReport, IntegrityStatus, PublicWatermarkHeader,
};
