# 固定水印攻击夹具 / Pinned watermark attack fixtures

这些图片由仓库的合成渐变公式生成，不含用户图片。每张图片只生成并固定保存一次密文；攻击测试必须使用这些相同字节，失败时不能重新编码、更换随机密文或密钥直到成功。

These images come from the repository's synthetic gradient formula and contain no user media. Ciphertext was generated once and pinned. Attack regressions must reuse these exact bytes rather than re-encode or change ciphertext/keys until an attack succeeds.

`tests/roundtrip.rs` 定义公开测试密钥和预期正文；这些常量不能用于保护真实内容。生产编码保持系统安全随机数。`examples/generate_fixtures.rs` 说明夹具来源；重新生成会使用新 nonce，不保证字节相同，必须作为显式测试数据变更审核。

The public test key and expected text are defined in `tests/roundtrip.rs` and must not protect real content. Production encoding retains OS randomness. `examples/generate_fixtures.rs` documents the source; regenerating uses fresh nonces and is not byte reproducible. Fixture updates require explicit test-data review.

| 图片 / Image | 实测 RGB PSNR / Actual RGB PSNR | 最小正文重复票 / Minimum body votes |
| --- | ---: | ---: |
| 640x480.png | 40.287900 dB | 2 |
| 960x1280.png | 40.463674 dB | 3 |
| 1920x1080.png | 40.313549 dB | 6 |
| 1024x768.png | 40.330688 dB | 3 |

| 图片 / Image | SHA-256 |
| --- | --- |
| 640x480.png | d8edf09bfacd2795da8db728264e1c75b9208e196cb7ad9eae0164cf34b967f1 |
| 960x1280.png | 8a5ee0f6f2526874bb9ca92d672e12618a01edb07e0081d755bb864bb505d04a |
| 1920x1080.png | f4031b13619cedf4dc39aa60cd4353447d92ad8809205e312391149a6760d085 |
| 1024x768.png | cae655e8271b36c5f08ea357c9d6bf926a2afc6f6793b088eb48851583dd5dac |
