pub mod bch;
pub mod crypto;
pub mod dct;
pub mod error;
pub mod fingerprint;
pub mod keyfile;
pub mod payload;
pub mod scan;
pub mod sync;
pub mod watermark;

pub use error::{CoreError, Result};
pub use fingerprint::{PartitionFingerprint, TamperRegion};
pub use keyfile::{KeyFile, KeyMode, KeySource, WatermarkKey};
pub use scan::{scan_image_file, PrivacyScanDetection, PrivacyScanReport, PrivacyScanStatus};
pub use sync::SyncRegistration;
pub use watermark::{
    embed_image_file, extract_image_file, extract_image_file_with_options, EmbedOptions, EmbedReport, ExtractOptions, ExtractReport, IntegrityStatus, PublicWatermarkHeader,
};
