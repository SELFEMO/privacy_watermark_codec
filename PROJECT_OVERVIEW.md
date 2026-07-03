# PROJECT_OVERVIEW.md

## 1. 项目整体描述、目标与架构

本项目是一个本地优先的跨平台桌面应用“Privacy Watermark Codec / 图隐私水印编解码器”。应用用于对图片和视频嵌入加密后的不可见隐私水印，也可以通过 `.key` 文件或自定义密码提取水印文本，并基于感知指纹报告疑似篡改状态。

整体架构如下：

- 前端 UI：`src/`，基于 Vue 3 + TypeScript + Vite，负责文件选择、参数配置、任务进度展示、FFmpeg 信息展示、结果展示和中英文界面文案。
- 桌面壳与业务调度：`src-tauri/`，基于 Tauri 2，暴露前端可调用命令，负责批量任务、视频抽帧/封装、FFmpeg 运行时定位、证据清单、右键菜单启动上下文、取消任务和跨平台打包资源。
- 水印核心库：`crates/watermark-core/`，纯 Rust 核心逻辑，负责密钥派生、载荷加密、DCT 频域嵌入/提取、BCH/Hamming 纠错、同步模板、感知指纹和未知图片扫描。
- 脚本层：`scripts/`，负责开发启动前置检查、FFmpeg manifest 生成、发布构建、Linux 桌面入口/AppImage 辅助流程。
- 打包配置：`src-tauri/tauri.conf.json`、`.github/workflows/release.yml` 和平台资源目录，负责 Windows/macOS/Linux 安装包与内置 FFmpeg 资源。

核心数据流：

1. 前端组装 `EncodeRequest` / `DecodeRequest` / `ScanRequest` 并通过 Tauri invoke 调用 Rust 命令。
2. `src-tauri/src/commands.rs` 校验请求、创建输出目录、生成/读取密钥、区分图片与视频。
3. 图片直接调用 `watermark-core` 的嵌入/提取/扫描函数。
4. 视频由 `src-tauri/src/video.rs` 使用内置 FFmpeg 抽帧，对每帧调用图片水印逻辑，再重新封装为视频。
5. 编码任务写出媒体文件、`.key` 文件和证据清单；解码任务返回文本、完整性状态、纠错信息和篡改区域；扫描任务返回无密钥检测结果。

## 2. 各文件职责及相互依赖关系

### 根目录

- `Cargo.toml`：Rust workspace 配置，包含 `crates/watermark-core` 和 `src-tauri` 两个成员。
- `Cargo.lock`：Rust 依赖锁定文件。
- `package.json`：前端、Tauri、FFmpeg manifest、打包脚本的 npm 命令入口。
- `package-lock.json`：Node 依赖锁定文件。
- `vite.config.ts`：Vite 配置，开发端口固定为 `127.0.0.1:1420`，并忽略 Rust 目录监听，避免开发时重复触发。
- `tsconfig.json`：TypeScript 编译配置。
- `index.html`：Vite 前端 HTML 入口。
- `dist/`：已构建的前端静态资源。
- `README.md`、`README.zh-CN.md`：英文/中文项目说明。
- `LICENSE`：项目许可证。
- `.cargo/config.toml`：Cargo 本地配置。
- `.npmrc`：npm 配置。
- `.gitattributes`、`.gitignore`：Git 属性和忽略规则。
- `GITHUB_UPLOAD_COMMANDS.md`、`LINUX_ARM64_BUILD.md`、`UPDATED_FILES.txt`：项目维护、上传和平台构建说明。
- `PROJECT_OVERVIEW.md`：项目总览文件，供后续维护接手使用。
- `PROJECT_RECORD.md`：项目更新日志，记录每次修改。

### `src/` 前端

- `src/main.ts`：Vue 应用入口，加载 `App.vue` 和全局样式。
- `src/App.vue`：主界面与主要前端逻辑。包含编码、解码、扫描、FFmpeg 信息四个标签页；维护表单状态、语言、缩放、任务进度、右键菜单导入状态；调用 Tauri 命令执行后端任务。
- `src/types.ts`：前后端共享的 TypeScript 类型定义，覆盖编码/解码/扫描请求与响应、任务进度、FFmpeg 信息、发布元数据、启动上下文等。
- `src/styles.css`：全局样式、布局、响应式视觉效果。
- `src/components/BrandMark.vue`：品牌标识组件，被主界面使用。

### `src-tauri/` Tauri 桌面层

- `src-tauri/Cargo.toml`：Tauri 应用 crate 配置，依赖 `watermark-core`、Tauri 插件、serde、sha2、walkdir 等。
- `src-tauri/build.rs`：Tauri 构建脚本入口。
- `src-tauri/tauri.conf.json`：Tauri 2 配置，定义窗口、CSP、bundle targets、图标、内置 FFmpeg 资源、Windows NSIS、Linux deb/rpm/AppImage 文件映射。
- `src-tauri/capabilities/default.json`：Tauri 权限能力配置。
- `src-tauri/icons/`：跨平台应用图标。
- `src-tauri/vendor/ffmpeg/`：内置 FFmpeg/FFprobe/FFplay 平台资源、license/readme/version、manifest 清单。
- `src-tauri/windows/installer-hooks.nsh`：Windows NSIS 安装钩子，用于安装集成逻辑。
- `src-tauri/linux/`：Linux desktop entry、metainfo、安装后/卸载后脚本、AppImage 辅助占位文件。

### `src-tauri/src/` Rust 桌面业务文件

- `main.rs`：Tauri 主程序入口，调用库 crate 的 `run()`。
- `lib.rs`：Tauri 应用初始化。设置日志、插件、单实例回调、状态管理、窗口创建、命令注册和 Linux 显示后端偏好。
- `commands.rs`：前端命令核心实现。提供 `encode_media`、`decode_media`、`scan_privacy_watermark`、`cancel_task`，并在阻塞线程中执行图片/视频批处理。
- `models.rs`：Tauri 命令请求/响应结构体和枚举，和 `src/types.ts` 对应。
- `media.rs`：媒体类型识别和安全文件名 stem 生成。
- `video.rs`：视频抽帧、逐帧嵌入/提取、并行度限制、FFmpeg 进度解析、重新封装视频。
- `ffmpeg.rs`：内置 FFmpeg manifest 读取、平台匹配、二进制 hash 校验、编码资源释放、运行信息读取。
- `storage.rs`：应用数据目录、webview 数据目录、临时工作目录创建与清理；优先使用可执行文件旁目录，不可写时回退到用户数据目录。
- `evidence.rs`：文件 SHA-256 和证据清单写入。证据清单签名绑定派生密钥但不泄露密钥本身。
- `progress.rs`：任务进度事件名、进度事件发送、百分比裁剪。
- `cancellation.rs`：任务取消注册表和取消 token，支持前端取消正在进行的任务。
- `release.rs`：发布元数据读取，只有声明更新清单和签名时才启用自动更新展示。
- `launch.rs`：解析命令行/右键菜单启动上下文，并合并单实例传入的文件列表。
- `desktop_entry.rs`：Linux 桌面入口相关逻辑。

### `crates/watermark-core/` 水印核心库

- `crates/watermark-core/Cargo.toml`：核心库 crate 配置，依赖 image、PBKDF2、ChaCha20-Poly1305、serde、sha2、crc32、zeroize 等。
- `src/lib.rs`：核心库模块导出和公共 API 重导出。
- `src/error.rs`：`CoreError` 和 `Result<T>`，统一图像、容量、密钥、解密、序列化、I/O 等错误。
- `src/crypto.rs`：随机字节、PBKDF2-HMAC-SHA256 派生、ChaCha20-Poly1305 加解密。
- `src/keyfile.rs`：`KeyMode`、`WatermarkKey`、`KeySource`、`KeyFile`，负责随机密钥、自定义密码密钥、JSON key 文件读写与校验。
- `src/payload.rs`：水印载荷格式。包含公共头、内部载荷、加密体、CRC32、bit/byte 转换。
- `src/watermark.rs`：图片水印主算法。负责 DCT 频域嵌入/提取、公共头读取、容量计算、PSNR 检查、路由选择、完整性分类。
- `src/dct.rs`：8×8 DCT/IDCT、亮度块读写与频域 bit 嵌入/提取。
- `src/fingerprint.rs`：全局差异 hash、4×4 分区感知指纹、分区篡改区域比较。
- `src/scan.rs`：未知图片扫描，检测项目公共水印头和常见隐私/AI 元数据痕迹。
- `src/sync.rs`：同步模板嵌入、同步评分、旋转/缩放候选配准，提升裁剪/压缩/几何变化后的提取能力。
- `src/bch.rs`：BCH(31,16) 风格纠错编码/解码，最多枚举有限 bit 错误用于恢复载荷。
- `src/hamming.rs`：Hamming(7,4) 编码/解码，保留为轻量纠错工具。
- `tests/roundtrip.rs`：核心库回环测试。

### `scripts/` 脚本层

- `scripts/tauri-dev.mjs`：开发模式总入口。依次运行 Tauri 预检、FFmpeg manifest 生成、Linux desktop entry 准备，并启动 `npm run tauri -- dev`。本次修复 Windows `.cmd/.bat` 启动问题。
- `scripts/preflight-tauri.mjs`：开发启动前检查核心 Tauri 文件是否存在，提前给出明确错误。
- `scripts/generate-ffmpeg-manifest.mjs`：生成 FFmpeg manifest、补齐 metadata、计算 hash、复制当前平台资源到 target 调试/发布目录。
- `scripts/build-release.mjs`：发布构建总入口，支持平台/架构/bundle 选择、AppImage 辅助环境、FFmpeg 资源处理、日志输出。
- `scripts/linux-desktop-entry.mjs`：Linux 开发态 desktop entry 和图标写入/清理。
- `scripts/ensure-linux-desktop-entry.mjs`：Linux desktop entry 辅助入口。
- `scripts/appimage-tools.mjs`：AppImage 工具镜像/下载辅助。
- `scripts/appimage-diagnostics.mjs`：AppImage 构建诊断。
- `scripts/appimage-prefetch.mjs`：AppImage 依赖工具预取。
- `scripts/check-env.sh`、`scripts/check-env.ps1`：平台环境检查脚本。

### `.github/workflows/`

- `.github/workflows/release.yml`：发布 CI 工作流，用于跨平台构建和发布产物。

## 3. 所有关键函数/类的作用、参数与返回值

### 前端关键函数

- `chooseEncodeInputs()`：打开文件选择器，选择待编码图片/视频；无显式返回值，更新 `encodeInputs`。
- `chooseDecodeInputs()`：选择待解码媒体；无显式返回值，更新 `decodeInputs`。
- `chooseScanInputs()`：选择待扫描图片；无显式返回值，更新 `scanInputs`。
- `chooseOutputDir()`：选择输出目录；无显式返回值，更新 `outputDir`。
- `chooseKeyFile()`：选择 `.key` 文件；无显式返回值，更新 `decodeKeyFile`。
- `runEncode()`：组装 `EncodeRequest` 并调用后端 `encode_media`；成功后更新 `encodeResult`，失败时更新 `errorMessage`。
- `runDecode()`：组装 `DecodeRequest` 并调用后端 `decode_media`；成功后更新 `decodeResult`。
- `runScan()`：组装 `ScanRequest` 并调用后端 `scan_privacy_watermark`；成功后更新 `scanResult`。
- `loadFfmpegInfo()`：调用 `get_ffmpeg_info` 并显示内置 FFmpeg 运行时和 hash 校验信息。
- `loadReleaseInfo()`：调用 `get_release_metadata`，展示发布签名/更新相关元数据。
- `cancelCurrentTask()`：调用后端 `cancel_task`，请求取消当前任务。
- `applyLaunchContext(context)`：根据右键菜单或单实例启动上下文，将文件导入编码/解码/扫描页。
- `formatProgressMessage(progress)`：把后端任务进度事件转为当前语言可读文案。
- `localizeRuntimeMessage(message, phase?)`：对后端错误和阶段消息进行本地化展示。

### Tauri 命令与桌面层关键函数/类

- `encode_media(app, cancellation, request) -> Result<EncodeResponse, String>`：异步 Tauri 命令。参数为应用句柄、取消状态、编码请求；返回批量编码结果。
- `decode_media(app, cancellation, request) -> Result<DecodeResponse, String>`：异步 Tauri 命令。参数为应用句柄、取消状态、解码请求；返回水印文本和完整性结果。
- `scan_privacy_watermark(app, cancellation, request) -> Result<ScanResponse, String>`：异步 Tauri 命令。参数为应用句柄、取消状态、扫描请求；返回无密钥扫描报告。
- `cancel_task(cancellation, request) -> Result<(), String>`：设置指定 task id 的取消标记。
- `encode_media_blocking(app, request, cancellation) -> Result<EncodeResponse, String>`：编码任务同步实现。负责验证、生成输出目录/密钥、区分图片视频、写证据清单。
- `decode_media_blocking(app, request, cancellation) -> Result<DecodeResponse, String>`：解码任务同步实现。负责读取 key/password，逐个媒体提取水印与篡改状态。
- `scan_privacy_watermark_blocking(app, request, cancellation) -> Result<ScanResponse, String>`：扫描任务同步实现。负责对未知图片执行项目水印头和隐私元数据检测。
- `detect_media_type(path) -> Option<MediaType>`：按扩展名识别图片或视频。
- `safe_stem(path) -> String`：生成跨平台安全输出文件名前缀。
- `encode_video(input, output, key, text, strength, options, progress, cancellation) -> Result<VideoEncodeReport, String>`：视频编码。抽帧、逐帧嵌入、重新封装并返回帧数和最低 PSNR。
- `decode_video(input, key_source, options, progress, cancellation) -> Result<VideoDecodeReport, String>`：视频解码。逐帧提取并汇总有效帧、修改帧、文本和篡改区域。
- `bundled_tools(app) -> Result<FfmpegTools, String>`：定位并校验当前平台内置 FFmpeg/FFprobe。
- `get_ffmpeg_info(app) -> Result<FfmpegRuntimeInfo, String>`：读取 manifest、许可证、二进制 hash 和版本信息。
- `storage_root() -> io::Result<PathBuf>`：解析应用数据根目录，优先使用便携目录，失败后回退到用户数据目录。
- `create_work_dir(prefix) -> io::Result<AppWorkDir>`：创建任务临时目录，`AppWorkDir` drop 时自动清理。
- `file_sha256(path) -> io::Result<String>`：计算文件 SHA-256。
- `write_evidence_manifest(path, entries, signing_key, release) -> io::Result<()>`：写入证据清单，并使用派生密钥参与签名材料。
- `emit_task_progress(app, event)`：向前端发送任务进度事件。
- `CancellationRegistry`：保存已请求取消的 task id 集合；提供 `clear`、`request_cancel`、`is_cancelled`。
- `CancellationToken`：单个任务的取消检查对象；提供 `check()`，drop 时清理取消标记。
- `get_release_metadata() -> ReleaseMetadata`：Tauri 命令，返回发布签名和更新元数据。
- `get_launch_context(app) -> LaunchContext`：返回初始命令行和单实例 pending 上下文合并后的启动上下文。
- `parse_launch_context(args) -> LaunchContext`：解析 `--pwc-action` 与 `--files` 参数。

### 水印核心库关键函数/类

- `WatermarkKey::random(mode) -> WatermarkKey`：生成随机盐和随机 secret，并通过 PBKDF2 派生 32 字节密钥。
- `WatermarkKey::from_password(password, salt) -> Result<WatermarkKey>`：由用户密码和盐派生自定义密码模式密钥。
- `WatermarkKey::to_key_file() -> KeyFile`：转换为可写入 JSON 的 key 文件结构。
- `KeyFile::read(path) -> Result<KeyFile>`：读取并校验 `.key` 文件。
- `KeyFile::write(path) -> Result<()>`：写入 JSON `.key` 文件。
- `KeyFile::validate() -> Result<()>`：校验版本、算法、迭代次数、salt 和 derived_key 长度。
- `KeyFile::to_watermark_key() -> Result<WatermarkKey>`：从 key 文件恢复可用于解码的密钥对象。
- `derive_key(secret, salt, iterations) -> [u8; 32]`：PBKDF2-HMAC-SHA256 密钥派生。
- `encrypt(plaintext, key) -> Result<([u8; 12], Vec<u8>)>`：ChaCha20-Poly1305 加密并生成随机 nonce。
- `decrypt(ciphertext, nonce, key) -> Result<Zeroizing<Vec<u8>>>`：ChaCha20-Poly1305 解密，明文用 `Zeroizing` 包裹。
- `InnerPayload::new(text, image) -> InnerPayload`：创建内部载荷，包含水印文本、时间戳、全局指纹和分区指纹。
- `Header::from_body(body, salt) -> Header`：根据加密体和 salt 创建公共头。
- `Header::to_bytes() -> [u8; HEADER_LEN]` / `Header::from_bytes(bytes) -> Result<Header>`：公共头序列化/反序列化和 CRC 校验。
- `create_encrypted_body(payload, key) -> Result<Vec<u8>>`：序列化内部载荷并加密为水印 body。
- `open_encrypted_body(body, key) -> Result<InnerPayload>`：解密并反序列化内部载荷。
- `bytes_to_bits(bytes) -> Vec<bool>` / `bits_to_bytes(bits) -> Vec<u8>`：二进制载荷与 bit 序列互转。
- `embed_image_file(input, output, key, text, options) -> Result<EmbedReport>`：按固定强度嵌入图片水印。
- `embed_image_file_with_auto_strength(input, output, key, text, options) -> Result<EmbedReport>`：自动尝试强度，保证 PSNR 达到阈值。
- `probe_embedded_header_file(path) -> Result<PublicWatermarkHeader>`：无密钥读取公共水印头。
- `extract_image_file(input, key_source) -> Result<ExtractReport>`：使用默认提取选项读取水印。
- `extract_image_file_with_options(input, key_source, options) -> Result<ExtractReport>`：支持同步配准选项的水印提取。
- `difference_hash(image) -> u64`：计算全局感知 hash。
- `partition_fingerprints(image) -> Vec<PartitionFingerprint>`：计算 4×4 分区感知指纹。
- `compare_partitions(current, embedded) -> Vec<TamperRegion>`：对比分区指纹并输出疑似篡改区域。
- `bch::encode_bytes(input) -> Vec<bool>` / `bch::decode_bits(bits, output_len) -> Option<DecodeOutcome>`：BCH 纠错编码/解码。
- `hamming::encode_bytes(input) -> Vec<bool>` / `hamming::decode_bits(bits, output_len) -> Option<DecodeOutcome>`：Hamming 纠错编码/解码。
- `sync::embed_template(image, strength)`：在同步块嵌入模板。
- `sync::registration_candidates(image) -> Vec<RegistrationCandidate>`：生成旋转/缩放配准候选。
- `scan_image_file(path) -> Result<PrivacyScanReport>`：无密钥扫描图片，报告项目水印头、EXIF/XMP/AI 元数据等痕迹。

### 脚本关键函数

- `scripts/tauri-dev.mjs::main()`：开发启动主流程：预检、生成 FFmpeg manifest、Linux desktop entry 准备、启动 Tauri dev、退出时清理 Linux 临时入口。
- `scripts/tauri-dev.mjs::run(command, args, extraEnv)`：封装子进程运行，继承 stdio 并跟踪 active child。
- `scripts/tauri-dev.mjs::normalizeSpawnInvocation(command, args)`：本次新增。仅在 Windows 且命令为 `.cmd/.bat` 时通过 `ComSpec` 转发，避免 `npm.cmd` 直接 spawn 抛出 `EINVAL`；Linux/macOS 保持原有直启行为。
- `scripts/tauri-dev.mjs::installSignalHandlers()`：处理 SIGINT/SIGTERM，转发给子进程并执行 Linux 清理。
- `scripts/tauri-dev.mjs::cleanupLinuxDesktopEntry()`：仅 Linux 上清理开发态 desktop entry。
- `scripts/generate-ffmpeg-manifest.mjs::main()`：生成/刷新 manifest，补齐 metadata，镜像当前平台 FFmpeg 到 target。
- `scripts/build-release.mjs::run(command, args, filterOutput, extraEnv, options)`：发布构建子进程封装，Windows 使用 shell，Linux/macOS 直启，并可写构建日志。

## 4. 当前项目存在的关键逻辑说明

### 密钥模式

- 独立密钥模式：每个输入文件生成独立目录、独立 `.key` 文件和独立媒体输出，降低单个 key 泄露影响面。
- 共享密钥模式：整个批次生成一个共享 key，输出到批次目录，便于统一归档和批量解码。
- 自定义密码模式：由用户密码通过 PBKDF2 派生 key，可选择是否输出 `.key` 文件；不输出 key 时只能靠同一密码解码。

### 图片水印逻辑

- 公共头写入可无密钥读取的固定区域，用于标识项目水印、格式版本、body 长度、salt、CRC 等。
- 加密 body 写入图像 8×8 DCT 频域块，使用路由选择和纠错编码提高恢复能力。
- 嵌入后检查 PSNR，目标阈值为 40dB，自动强度函数会在失败时尝试降低强度。
- 内部载荷包含水印文本、创建时间、全局指纹和分区指纹，用于后续完整性判断。

### 视频水印逻辑

- 视频处理依赖内置 FFmpeg/FFprobe。
- 编码时先抽帧为图片，对每一帧执行图片水印嵌入，再将处理后的帧重新封装为视频。
- 解码时逐帧提取水印并汇总，统计有效帧、修改帧、帧数和篡改区域。
- 并行度通过请求参数传入并归一化，避免过高并发造成 CPU/磁盘 I/O 压力。

### FFmpeg 资源逻辑

- `src-tauri/vendor/ffmpeg/manifest.json` 记录平台、文件名、SHA-256、版本、许可证等信息。
- Tauri bundle resources 中包含各平台 FFmpeg 目录。
- 运行时根据当前平台选择平台 key，定位资源文件并校验 hash。
- AppImage 构建阶段会对 FFmpeg 资源做额外处理，避免 Linux 打包工具误 strip 非目标文件。

### 开发启动逻辑

- `npm run tauri:dev` 实际执行 `node scripts/tauri-dev.mjs`。
- 脚本先运行 `preflight-tauri.mjs` 检查关键文件，再运行 `generate-ffmpeg-manifest.mjs` 刷新 FFmpeg manifest。
- Linux 上还会写入临时 desktop entry 以改善调试窗口图标匹配，退出时清理。
- 本次修复后，Windows 上启动 `npm.cmd` 时会通过 `cmd.exe /d /s /c` 转发，避免 `.cmd` 直启导致 `spawn EINVAL`；Linux x64 路径不改变。

## 5. 当前功能状态及待办事项

### 已实现/当前状态

- 图片编码、批量图片编码、视频编码。
- 图片解码、视频逐帧解码。
- 未知图片扫描。
- 独立密钥、共享密钥、自定义密码三种模式。
- `.key` JSON 读写。
- PBKDF2-HMAC-SHA256 密钥派生和 ChaCha20-Poly1305 加密。
- DCT 频域水印、纠错、同步模板、感知指纹篡改检测。
- FFmpeg 内置资源 manifest 与运行时校验。
- Windows NSIS、Linux deb/rpm/AppImage、macOS app/dmg 相关脚本与配置入口。
- 前端中英文界面、任务取消、任务进度展示、右键菜单导入。
- Windows `npm run tauri:dev` 的 `spawn EINVAL` 问题已做最小范围修复。

### 待办/注意事项

- 当前环境未安装 Rust/Cargo，因此本次无法在沙箱内执行 `cargo check` 或完整 Tauri 构建；已执行 `node --check scripts/tauri-dev.mjs` 做脚本语法校验。
- 本次修复不修改 Linux x64 运行逻辑；建议在 Windows 上重新执行 `npm run tauri:dev` 验证启动链路，并在 Linux x64 上回归一次开发启动或构建。
- 若发布包需要 FFmpeg 大文件，仍需确保 Git LFS 或发布源码中包含真实二进制，而不是 LFS pointer。
- `PROJECT_OVERVIEW.md` 和 `PROJECT_RECORD.md` 后续每次修改都应同步维护。
