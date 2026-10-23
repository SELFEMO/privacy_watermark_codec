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

    #[error(
        "未检测到通过校验的水印头；图片可能没有本软件水印，也可能超出当前恢复范围，可尝试深入恢复"
    )]
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

    #[error("已达到几何恢复时间上限；可选择深入恢复，超时不表示图片没有水印")]
    RecoveryTimeout,

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
    fn header_not_found_does_not_claim_definite_absence() {
        let message = CoreError::HeaderNotFound.to_string();
        assert!(message.contains("可能"));
        assert!(message.contains("深入恢复"));
    }
}
