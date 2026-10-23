use std::{env, fs, path::PathBuf};
use watermark_core::{embed_image_file_with_sync, EmbedOptions, KeyMode, WatermarkKey};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().collect();
    if args.len() != 3 {
        return Err("usage: sync_probe INPUT OUTPUT_DIRECTORY".into());
    }
    let output = PathBuf::from(&args[2]);
    fs::create_dir_all(&output)?;
    // 固定测试密钥让跨算法比较使用相同路由；此诊断程序不能用于保护真实内容。
    // A fixed test key gives algorithm comparisons identical routes; this diagnostic must not protect real content.
    let key = WatermarkKey {
        mode: KeyMode::Shared,
        salt: [17; 16],
        derived_key: [42; 32],
        iterations: 210_000,
    };
    key.to_key_file().write(output.join("test.key"))?;
    for strength in (3..=8).rev() {
        let options = EmbedOptions {
            text: "geometry probe".into(),
            key: key.clone(),
            strength: strength as f32,
            media_kind: "image".into(),
            temporal: None,
        };
        match embed_image_file_with_sync(&args[1], output.join("sync.png"), &options, true) {
            Ok(report) => {
                println!("sync encoded psnr={:.6} strength={strength}", report.psnr);
                return Ok(());
            }
            Err(error) => eprintln!("strength={strength}: {error}"),
        }
    }
    Err("the synchronization profile did not meet the quality gate".into())
}
