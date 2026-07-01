# 图隐私水印编解码器

[English](README.md) | [简体中文](README.zh-CN.md)

图隐私水印编解码器是一个本地优先的桌面应用，基于 Tauri、Vue 和 Rust 构建，用于给图片和视频嵌入不可见加密隐私水印，支持携带密钥或密码解码、感知级篡改检测，并可对未知来源图片扫描可能存在的隐私水印或 AI 水印痕迹。

## AI 编程提示

本项目由 AI 辅助编程完成。正式发布或生产使用前，应由开发者复核代码、在目标平台重新测试、使用自有样本验证水印效果，并确认所有第三方二进制文件的许可证合规性。

## 功能概览

- 单张图片和批量图片水印编码。
- 通过内置 FFmpeg 进行视频逐帧编码和逐帧解码。
- 支持独立密钥、批次共享密钥、自定义密码三种模式。
- 使用 PBKDF2-HMAC-SHA256 密钥派生和 ChaCha20-Poly1305 认证加密。
- 使用 DCT 中频水印、Hamming 纠错和空间重复投票。
- 使用感知指纹报告疑似篡改状态。
- 支持未知图片扫描项目水印头和常见隐私/AI 元数据痕迹。
- 所有媒体、密码、密钥均在本地处理，不上传网络。
- 支持中文和英文界面。
- Windows 图片右键菜单使用统一分组子菜单，不把功能散开。

## 当前验证状态

| 平台 | 打包状态 | 说明 |
| --- | --- | --- |
| Windows x64 | 默认目标 | 当前主要开发和打包路径，默认使用 NSIS 安装包。 |
| Windows ARM64 | 已预留但未测试 | 已有目录和 manifest 结构，需要 ARM64 FFmpeg 二进制并进行平台测试。 |
| macOS ARM64 | 已预留但未测试 | 需要 macOS 机器、FFmpeg 可执行权限、签名/公证方案和平台测试。 |
| macOS x64 | 已预留但未测试 | 需要 Intel macOS 构建环境或对应 runner，尚未验证。 |
| Linux x64 | 已预留但未测试 | 需要 Linux Tauri 系统依赖和 Linux FFmpeg 二进制，尚未验证。 |
| Linux ARM64 | 已预留但未测试 | 需要 ARM64 Linux 构建环境和 FFmpeg 二进制，尚未验证。 |

目前只有 Windows x64 可视为当前验证过的主要打包目标。其他平台仅补齐目录约定和操作说明，尚不能视为正式发布包。

## FFmpeg 目录约定

本项目将内置 FFmpeg 文件放在：

```text
src-tauri/vendor/ffmpeg/
├─ LICENSE.txt
├─ README.md
├─ VERSION.txt
├─ manifest.json
├─ windows_x64/
├─ windows_arm64/
├─ macos_arm64/
├─ macos_x64/
├─ linux_x64/
├─ linux_amd64/
└─ linux_arm64/
```

各平台至少需要：

```text
windows_x64/ffmpeg.exe
windows_x64/ffprobe.exe

windows_arm64/ffmpeg.exe
windows_arm64/ffprobe.exe

macos_arm64/ffmpeg
macos_arm64/ffprobe

macos_x64/ffmpeg
macos_x64/ffprobe

linux_x64/ffmpeg
linux_x64/ffprobe

linux_arm64/ffmpeg
linux_arm64/ffprobe
```

`ffplay` 是可选文件。如果存在，会显示在 FFmpeg 信息页中，但编码和解码不依赖它。

每次复制或替换二进制文件后，重新生成清单：

```bash
npm run ffmpeg:manifest
```

macOS 和 Linux 平台打包前，需要给运行文件加执行权限：

```bash
chmod +x src-tauri/vendor/ffmpeg/macos_arm64/ffmpeg src-tauri/vendor/ffmpeg/macos_arm64/ffprobe
chmod +x src-tauri/vendor/ffmpeg/linux_x64/ffmpeg src-tauri/vendor/ffmpeg/linux_x64/ffprobe
```

实际使用时请把目录名替换成目标平台目录。

## Windows x64 构建

安装依赖：

```bash
npm install
```

刷新 FFmpeg 清单：

```bash
npm run ffmpeg:manifest
```

开发运行：

```bash
npm run tauri:dev
```

构建 Windows 安装包：

```bash
npm run tauri:build
```

安装包输出目录：

```text
target/release/bundle/nsis/
```

构建脚本会在完成后把安装包文件名里的应用版本段去掉。

## 非 Windows 平台说明

项目已经补齐 macOS 和 Linux 的目录约定，但这些平台尚未测试。

通用步骤：

```bash
npm install
npm run ffmpeg:manifest
cargo test -p watermark-core --release
npm run tauri:build
```

注意事项：

- macOS 安装包应在 macOS 上构建；本项目尚未配置签名和公证。
- Linux 构建需要安装 Tauri 所需的 WebKitGTK、AppIndicator、librsvg、patchelf 等系统依赖。
- 打包前请确认 `src-tauri/tauri.conf.json` 中的 resources 已包含目标平台的 FFmpeg 目录。
- 每个目标系统都必须重新执行编码、解码、未知图片扫描的完整流程后，才能发布给用户。

## Git 与 Git LFS

`src-tauri/vendor/ffmpeg` 目录可能超过 GitHub 普通文件大小限制，因此项目已加入：

```text
.gitattributes
```

其中包含：

```text
src-tauri/vendor/ffmpeg/** filter=lfs diff=lfs merge=lfs -text
```

这会让整个 FFmpeg vendor 目录通过 Git LFS 管理，包括没有扩展名的文件。

在第一次提交 FFmpeg 二进制文件之前，请先执行：

```bash
git lfs install
git lfs track "src-tauri/vendor/ffmpeg/**"
git add .gitattributes
```

完整上传命令已放在：

```text
GITHUB_UPLOAD_COMMANDS.md
```

## 运行数据存储策略

软件不使用 `%APPDATA%` 作为自身运行数据目录。运行数据默认存放在可执行文件旁边：

```text
PrivacyWatermarkCodecData/
├─ webview-data/
└─ work/
```

NSIS 安装器会尽量选择非系统盘。卸载时会删除安装目录下的 `PrivacyWatermarkCodecData`。

## Windows 右键菜单

Windows 安装后会注册统一的图片右键菜单：

```text
Privacy Watermark Codec
├─ 编码隐私水印 / Encode privacy watermark
├─ 检查隐私水印 / Decode and inspect
└─ 无密钥扫描 / Keyless scan
```

菜单带软件图标，支持多选图片。多选文件会由单实例机制合并导入。

## 使用说明

### 编码

1. 选择图片或视频。
2. 选择输出目录。
3. 输入水印文本。
4. 选择密钥模式。
5. 开始编码。

### 解码

1. 选择已编码媒体。
2. 选择 `.key` 文件，或输入原始自定义密码。
3. 开始解码与检测。

### 扫描未知图片

1. 选择一个或多个未知图片，或通过右键菜单导入。
2. 开始扫描。右键菜单选择“无密钥扫描”后会在导入完成后自动扫描。
3. 查看痕迹摘要。

检测到项目水印头，只能说明图片可能含有本项目加密水印；没有密钥或密码时不会显示水印正文。未检出水印也不等于图片一定没有水印。

## 密钥文件提醒

`.key` 文件包含可用于解密的派生密钥材料，应按密码文件保管。泄露 `.key` 文件后，对应水印文本可能被恢复。

## 项目结构

```text
privacy-watermark-codec/
├─ src/                         Vue 前端
├─ src/components/              UI 组件和品牌 Logo
├─ src-tauri/                   Tauri 壳、命令、安装配置、视频调度
├─ src-tauri/icons/             应用图标
├─ src-tauri/vendor/ffmpeg/     内置 FFmpeg 二进制和清单
├─ src-tauri/windows/           NSIS 安装钩子
├─ crates/watermark-core/       Rust 水印、加密、密钥、扫描与篡改检测核心
├─ scripts/                     构建、发布与 manifest 脚本
└─ .github/workflows/           Windows 发布工作流
```

## 审阅说明

当前包已静态检查明显错误路径、过期 README 引用、右键菜单 `%*` 传参、启动阶段 FFmpeg 自动探测，以及 FFmpeg 命令行窗口闪现相关代码。前端类型检查和生产构建已通过：

```bash
npm run build
```

Rust 编译和 Windows 安装包生成仍需在复制 FFmpeg 二进制后的目标 Windows 机器上再次验证。
