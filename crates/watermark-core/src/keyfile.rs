use std::{fs, path::Path};

use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::{
    crypto::{derive_key, random_bytes, DEFAULT_PBKDF2_ITERATIONS, KEY_LEN, SALT_LEN},
    error::{CoreError, Result},
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KeyMode {
    Independent,
    Shared,
    Custom,
}

#[derive(Debug, Clone)]
pub struct WatermarkKey {
    pub mode: KeyMode,
    pub salt: [u8; SALT_LEN],
    pub derived_key: [u8; KEY_LEN],
    pub iterations: u32,
}

#[derive(Debug, Clone)]
pub enum KeySource {
    KeyFile(KeyFile),
    CustomPassword(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyFile {
    pub version: u8,
    pub algorithm: String,
    pub iterations: u32,
    pub salt: String,
    pub derived_key: String,
    pub created_at: String,
    pub mode: KeyMode,
}

impl WatermarkKey {
    pub fn random(mode: KeyMode) -> Self {
        let salt = random_bytes::<SALT_LEN>();
        let secret = random_bytes::<KEY_LEN>();
        let derived_key = derive_key(&secret, &salt, DEFAULT_PBKDF2_ITERATIONS);
        Self {
            mode,
            salt,
            derived_key,
            iterations: DEFAULT_PBKDF2_ITERATIONS,
        }
    }

    pub fn from_password(password: &str, salt: Option<[u8; SALT_LEN]>) -> Result<Self> {
        if password.is_empty() {
            return Err(CoreError::InvalidArgument("自定义密码不能为空".into()));
        }
        let salt = salt.unwrap_or_else(random_bytes::<SALT_LEN>);
        let derived_key = derive_key(password.as_bytes(), &salt, DEFAULT_PBKDF2_ITERATIONS);
        Ok(Self {
            mode: KeyMode::Custom,
            salt,
            derived_key,
            iterations: DEFAULT_PBKDF2_ITERATIONS,
        })
    }

    pub fn to_key_file(&self) -> KeyFile {
        KeyFile {
            version: 1,
            algorithm: "PBKDF2-HMAC-SHA256".into(),
            iterations: self.iterations,
            salt: STANDARD.encode(self.salt),
            derived_key: STANDARD.encode(self.derived_key),
            created_at: Utc::now().to_rfc3339(),
            mode: self.mode,
        }
    }
}

impl KeyFile {
    pub fn read(path: impl AsRef<Path>) -> Result<Self> {
        let content = fs::read_to_string(path)?;
        let file: Self = serde_json::from_str(&content)
            .map_err(|error| CoreError::InvalidKeyFile(error.to_string()))?;
        file.validate()?;
        Ok(file)
    }

    pub fn write(&self, path: impl AsRef<Path>) -> Result<()> {
        let content = serde_json::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            return Err(CoreError::InvalidKeyFile(format!(
                "不支持的版本：{}",
                self.version
            )));
        }
        if self.algorithm != "PBKDF2-HMAC-SHA256" {
            return Err(CoreError::InvalidKeyFile(format!(
                "不支持的算法：{}",
                self.algorithm
            )));
        }
        if self.iterations < 100_000 {
            return Err(CoreError::InvalidKeyFile("PBKDF2 迭代次数过低".into()));
        }
        let _ = self.to_watermark_key()?;
        Ok(())
    }

    pub fn to_watermark_key(&self) -> Result<WatermarkKey> {
        let salt_vec = STANDARD
            .decode(&self.salt)
            .map_err(|error| CoreError::InvalidKeyFile(error.to_string()))?;
        let key_vec = STANDARD
            .decode(&self.derived_key)
            .map_err(|error| CoreError::InvalidKeyFile(error.to_string()))?;

        let salt: [u8; SALT_LEN] = salt_vec
            .try_into()
            .map_err(|_| CoreError::InvalidKeyFile("salt 长度必须为 16 字节".into()))?;
        let derived_key: [u8; KEY_LEN] = key_vec
            .try_into()
            .map_err(|_| CoreError::InvalidKeyFile("derived_key 长度必须为 32 字节".into()))?;

        Ok(WatermarkKey {
            mode: self.mode,
            salt,
            derived_key,
            iterations: self.iterations,
        })
    }
}
