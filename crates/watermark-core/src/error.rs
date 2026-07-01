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

    #[error("图像过小，至少需要 256×256 像素")]
    ImageTooSmall,

    #[error("未检测到有效水印头；图片可能没有使用本软件编码，或已发生严重裁剪/压缩")]
    HeaderNotFound,

    #[error("水印数据校验失败；内容可能被严重修改，或解码凭据不正确")]
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

    #[error("I/O 操作失败：{0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, CoreError>;
