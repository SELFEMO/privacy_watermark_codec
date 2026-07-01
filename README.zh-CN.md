# 图隐私水印编解码器

[English](README.md) | [简体中文](README.zh-CN.md)

图隐私水印编解码器是一个本地桌面应用，基于 Tauri、Vue 和 Rust 构建。它可以给图片和视频嵌入不可见的加密隐私水印，也可以在提供 `.key` 文件或原始自定义密码后提取水印内容，并报告媒体是否存在疑似篡改。

本项目的目标不是把图片变成可见水印图，而是在尽量不影响画面观感的前提下，将加密信息嵌入到图像频域中，用于版权声明、内容追踪和篡改辅助判断。

## 功能概览

- 支持单张图片、批量图片和视频水印编码。
- 支持图片和视频水印解码。
- 支持独立密钥、批次共享密钥、自定义密码三种密钥模式。
- 使用 PBKDF2-HMAC-SHA256 派生密钥，使用 ChaCha20-Poly1305 加密水印载荷。
- 使用 DCT 中频嵌入、BCH 纠错、同步模板辅助配准和空间重复投票提高鲁棒性。
- 使用全局和分区感知指纹报告疑似篡改状态，并定位疑似篡改区域。
- 支持未知图片扫描项目水印头和常见隐私/AI 元数据痕迹。
- 所有媒体、密码、密钥都在本地处理，不上传网络。
- 支持中文和英文界面。
- Windows 图片右键菜单提供统一分组入口。

## 克隆或下载项目

推荐使用 Git 克隆项目：

```text
git clone https://github.com/SELFEMO/privacy_watermark_codec.git
cd privacy_watermark_codec
```

如果你只想查看源码，也可以使用 GitHub 页面中的 **Code > Download ZIP**。但 GitHub 的源码 ZIP 不一定包含 Git LFS 管理的大文件，因此不建议把它作为可直接构建的完整包。

如果仓库中的 FFmpeg 二进制通过 Git LFS 管理，克隆后继续执行：

```text
git lfs install
git lfs pull
```

这些 Git 命令在 Windows、macOS、Linux 上相同。只有本地项目路径不同，例如 Windows 可能是 `D:\MyWorkstation\Learning\Rust\privacy_watermark_codec`，macOS 或 Linux 可能是 `~/Learning/Rust/privacy_watermark_codec`。

## 准备 FFmpeg 文件

视频编码和解码需要 `ffmpeg` 和 `ffprobe`。项目默认从下面目录读取内置 FFmpeg：

```text
src-tauri/vendor/ffmpeg/
```

你可以使用仓库中已经上传的 FFmpeg 文件，也可以自己下载或编译 FFmpeg 后放入对应目录。FFmpeg 官方网站是 `https://ffmpeg.org/`，官方下载页是 `https://ffmpeg.org/download.html`。FFmpeg 官方下载页主要提供源码和已编译包入口，Windows 用户也可以从官方下载页列出的第三方构建入口获取可执行文件。

各平台至少需要放入以下文件：

```text
src-tauri/vendor/ffmpeg/windows_x64/ffmpeg.exe
src-tauri/vendor/ffmpeg/windows_x64/ffprobe.exe

src-tauri/vendor/ffmpeg/windows_arm64/ffmpeg.exe
src-tauri/vendor/ffmpeg/windows_arm64/ffprobe.exe

src-tauri/vendor/ffmpeg/macos_arm64/ffmpeg
src-tauri/vendor/ffmpeg/macos_arm64/ffprobe

src-tauri/vendor/ffmpeg/macos_x64/ffmpeg
src-tauri/vendor/ffmpeg/macos_x64/ffprobe

src-tauri/vendor/ffmpeg/linux_x64/ffmpeg
src-tauri/vendor/ffmpeg/linux_x64/ffprobe

src-tauri/vendor/ffmpeg/linux_arm64/ffmpeg
src-tauri/vendor/ffmpeg/linux_arm64/ffprobe
```

`ffplay` 是可选文件，缺少它不会影响水印编码和解码。

复制或替换 FFmpeg 文件后，在项目根目录刷新 FFmpeg 清单：

```text
npm run ffmpeg:manifest
```

macOS 和 Linux 平台还需要给可执行文件增加执行权限。下面是示例，实际目录名请按目标平台替换：

```text
chmod +x src-tauri/vendor/ffmpeg/macos_arm64/ffmpeg src-tauri/vendor/ffmpeg/macos_arm64/ffprobe
chmod +x src-tauri/vendor/ffmpeg/linux_x64/ffmpeg src-tauri/vendor/ffmpeg/linux_x64/ffprobe
```

## Git LFS 拉不全 FFmpeg 时的处理

如果你确认 GitHub 云端已经有 FFmpeg 文件，但 `git lfs pull` 或 `git lfs fetch --all` 后本地仍然不完整，优先按下面顺序处理。

第一步，确认本地没有设置 LFS 下载过滤。过滤规则会导致只下载部分路径：

```text
git config --show-origin --get-regexp "lfs\.(fetchinclude|fetchexclude)"
```

如果上面命令输出了 `lfs.fetchinclude` 或 `lfs.fetchexclude`，清理本仓库和全局过滤配置：

```text
git config --local --unset-all lfs.fetchinclude
git config --local --unset-all lfs.fetchexclude
git config --global --unset-all lfs.fetchinclude
git config --global --unset-all lfs.fetchexclude
```

如果某一条提示没有该配置，可以忽略，继续执行下一条。

第二步，只拉取 FFmpeg 目录，并把 LFS 缓存中的真实文件检出到工作区：

```text
git lfs install --force
git lfs fetch origin main --include="src-tauri/vendor/ffmpeg/**" --exclude=""
git lfs checkout
git lfs pull origin main --include="src-tauri/vendor/ffmpeg/**" --exclude=""
```

这里的关键是 `git lfs checkout`。`git lfs fetch --all` 只是把对象下载到本地 LFS 缓存，不一定会自动替换工作区里的 pointer 文本文件；`git lfs checkout` 才会把缓存中的真实二进制写回工作区。

第三步，检查本地是否仍是 pointer 文件。真实的 `ffmpeg.exe` 或 `ffmpeg` 通常不会是几百字节的小文本文件：

```text
git lfs ls-files
```

如果 `git lfs ls-files` 能看到 FFmpeg 条目，但工作区文件仍然是小文本 pointer，再执行一次：

```text
git lfs checkout src-tauri/vendor/ffmpeg
```

如果仍然失败，通常不是普通 clone 命令的问题，而是以下原因之一：当前分支没有引用这些 LFS 对象、远端 LFS 对象没有成功推送到对应远端、仓库 LFS 权限/额度异常，或本地 Git LFS 被代理/网络拦截。维护者应参考 `GITHUB_UPLOAD_COMMANDS.md` 重新推送 LFS 对象。

## 安装、启动与打包

安装前端依赖：

```text
npm install
```

开发模式启动：

```text
npm run tauri:dev
```

构建安装包：

```text
npm run tauri:build
```

Windows NSIS 安装包默认输出到：

```text
target/release/bundle/nsis/
```

## 使用教程

### 编码水印

1. 打开软件，进入编码模式。
2. 选择图片或视频文件。
3. 选择输出目录。
4. 输入需要嵌入的水印文本。
5. 选择密钥模式：独立密钥、批次共享密钥或自定义密码。
6. 点击开始编码。
7. 编码完成后，保存输出媒体文件、`.key` 文件和证据清单。

### 解码水印

1. 打开软件，进入解码模式。
2. 选择已编码的图片或视频。
3. 选择 `.key` 文件，或输入编码时使用的自定义密码。
4. 点击开始解码与检测。
5. 查看提取出的水印文本、篡改判断和疑似篡改区域。

### 扫描未知图片

1. 选择一个或多个未知图片，或通过 Windows 右键菜单导入。
2. 点击扫描。
3. 查看是否发现项目水印头、常见隐私元数据或 AI 水印痕迹。

检测到项目水印头，只表示图片可能含有本项目加密水印；没有密钥或密码时不会显示水印正文。未检出水印也不等于图片一定没有水印。

## 当前验证状态

| 平台 | 当前状态 | 说明 |
| --- | --- | --- |
| Windows x64 | 已作为主要路径验证 | 当前主要开发、调试和打包目标，默认使用 NSIS 安装包。 |
| Windows ARM64 | 未测试 | 已预留目录，需要对应 FFmpeg 文件和真机/虚拟环境验证。 |
| macOS ARM64 | 未测试 | 需要 macOS 环境、FFmpeg 执行权限、签名/公证方案和平台验证。 |
| macOS x64 | 未测试 | 需要对应构建环境和平台验证。 |
| Linux x64 | 未测试 | 需要 Linux Tauri 系统依赖、对应 FFmpeg 文件和平台验证。 |
| Linux ARM64 | 未测试 | 需要对应构建环境、对应 FFmpeg 文件和平台验证。 |

## 运行数据存储策略

软件不使用 `%APPDATA%` 作为自身运行数据目录。运行数据默认存放在可执行文件旁边：

```text
PrivacyWatermarkCodecData/
├─ webview-data/
└─ work/
```

Windows 安装器会尽量选择非系统盘。卸载时会删除安装目录下的 `PrivacyWatermarkCodecData`。

## Windows 右键菜单

Windows 安装后会注册统一的图片右键菜单：

```text
Privacy Watermark Codec
├─ 编码隐私水印 / Encode privacy watermark
├─ 检查隐私水印 / Decode and inspect
└─ 无密钥扫描 / Keyless scan
```

菜单带软件图标，支持多选图片。多选文件会由单实例机制合并导入。

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

## AI 编程提示

本项目由 AI 辅助编程完成。正式发布或生产使用前，应由开发者复核代码、使用自有样本验证水印效果，并确认所有第三方二进制文件的许可证合规性。
