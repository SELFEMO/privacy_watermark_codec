use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("无法读取图像：{path}；原因：{source}")]
    ImageOpen {
        path: PathBuf,
        #[source]
        source: image::ImageError,
    },

    #[error("无法保存图像：{path}；原因：{source}")]
    ImageSave {
        path: PathBuf,
        #[source]
        source: image::ImageError,
    },

    #[error("图像载荷容量不足：需要至少 {required_blocks} 个可写入载荷的 8×8 块，当前只有 {available_blocks} 个。请缩短水印文本、降低分区指纹开销，或使用更高分辨率的图片/视频")]
    InsufficientCapacity {
        required_blocks: usize,
        available_blocks: usize,
    },

    #[error("图像尺寸不足；编码至少需要 256×256 像素，受攻击媒体解码至少需保留 128×128 像素")]
    ImageTooSmall,

    #[error("图像像素数超过 64,000,000 上限，请缩小后再处理")]
    ImageTooLarge,

    #[error("扫描文件超过 80,000,000 字节上限")]
    ScanFileTooLarge,

    #[error("未检测到有效水印头；图片可能没有使用本软件编码，或旋转后铺满裁切/严重压缩破坏了块周期。无白边的小角度旋转铺满裁切可能对不齐网格；旧一级高频在旋转插值后也可能无法恢复")]
    HeaderNotFound,

    #[error("已识别到水印结构，但正文无法通过纠错与认证；图片可能被过度遮挡、压缩或修改")]
    PayloadCorrupted,

    #[error("密钥文件与媒体中的水印盐值不匹配")]
    SaltMismatch,

    #[error("密码或密钥错误，无法解密水印内容")]
    DecryptionFailed,

    #[error("密钥文件格式无效：{0}")]
    InvalidKeyFile(String),

    #[error("参数无效：{0}")]
    InvalidArgument(String),

    #[error("序列化失败：{0}")]
    Serialization(#[from] serde_json::Error),

    #[error("任务已取消")]
    Cancelled,

    #[error("I/O 操作失败：{0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, CoreError>;

#[cfg(test)]
mod tests {
    use super::CoreError;

    #[test]
    fn header_not_found_mentions_fill_crop_and_legacy_rotation() {
        let message = CoreError::HeaderNotFound.to_string();
        assert!(message.contains("铺满裁切"));
        assert!(message.contains("一级高频"));
    }
}
