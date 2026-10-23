pub mod bch;
pub mod crypto;
pub mod dct;
pub mod dwt_qim;
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
    embed_image_file, embed_image_file_with_auto_strength, extract_image_file,
    extract_image_file_with_options, extract_image_file_with_progress, EmbedOptions, EmbedReport,
    ExtractOptions, ExtractReport, GeometrySearchProgress, IntegrityStatus, PublicWatermarkHeader,
    TemporalContext,
};

pub mod recovery;
pub use recovery::{RecoveryMode, RecoveryOptions};
pub use watermark::extract_image_file_with_recovery;

mod spectral;

mod pilot;
pub use watermark::embed_image_file_with_sync;
