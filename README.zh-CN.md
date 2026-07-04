# 图隐私水印编解码器

[English](README.md) | [简体中文](README.zh-CN.md)

图隐私水印编解码器是一款本地优先的桌面应用，用于把加密后的隐形隐私水印嵌入图片或视频，也可以使用匹配的 `.key` 文件或自定义密码提取水印文本，并通过感知指纹报告疑似篡改状态。

> 项目基于 Tauri、Vue 和 Rust。媒体文件、密码、密钥文件都在本机处理，不上传网络。

## AI 编程提示

本项目由 AI 辅助编程完成。正式发布或生产使用前，应由开发者复核代码、在目标平台重新测试、使用自有样本验证水印效果，并确认所有第三方二进制文件的许可证合规性。

## 功能概览

- 支持单张图片、批量图片、视频水印编码。
- 支持图片和视频水印解码。
- 支持独立密钥、批次共享密钥、自定义密码三种密钥模式。
- 使用 PBKDF2-HMAC-SHA256 派生密钥，使用 ChaCha20-Poly1305 加密载荷。
- 使用 DCT 频域嵌入、BCH 纠错、同步模板辅助配准和空间重复投票提高恢复鲁棒性。
- 使用感知指纹报告全局篡改状态和疑似篡改区域。
- 支持未知图片扫描项目水印头，以及常见隐私/AI 元数据痕迹。
- 支持中英文界面。
- Windows 安装后提供分组图片右键菜单。

## 项目结构

```text
privacy-watermark-codec/
├─ src/                         Vue 前端
├─ src/components/              UI 组件和品牌标识
├─ src-tauri/                   Tauri 壳、命令、打包配置、视频调度
├─ src-tauri/icons/             应用图标
├─ src-tauri/linux/             Linux 桌面入口、安装脚本和 AppImage 辅助文件
├─ src-tauri/vendor/ffmpeg/     内置 FFmpeg 文件和生成的清单
├─ src-tauri/windows/           NSIS 安装钩子
├─ crates/watermark-core/       Rust 水印、加密、密钥、扫描与篡改检测核心
├─ scripts/                     开发、发布、FFmpeg、AppImage 辅助脚本
└─ .github/workflows/           发布工作流
```

## 环境准备

需要安装 Node.js、Rust，以及 Tauri 在当前平台所需的原生依赖。

Linux 主机构建 AppImage 还需要：

```text
sudo apt install patchelf binutils file
```

## 克隆项目并恢复 FFmpeg 大文件

推荐使用 Git 克隆：

```text
git clone https://github.com/SELFEMO/privacy_watermark_codec.git
cd privacy_watermark_codec
```

如果 FFmpeg 二进制通过 Git LFS 管理，克隆后执行：

```text
git lfs install
git lfs pull
```

如果 Ubuntu 或 Debian 提示没有 `git lfs` 命令：

```text
sudo apt update
sudo apt install -y git-lfs
git lfs install
git lfs pull
```

GitHub 页面里的源码 ZIP 不一定包含 Git LFS 二进制对象。需要直接构建时，建议使用完整 Git 克隆或发布包源码。

## FFmpeg 文件

视频功能需要 `ffmpeg` 和 `ffprobe`。项目默认从以下目录读取内置文件：

```text
src-tauri/vendor/ffmpeg/
```

最低目录结构：

```text
src-tauri/vendor/ffmpeg/windows_x64/ffmpeg.exe
src-tauri/vendor/ffmpeg/windows_x64/ffprobe.exe
src-tauri/vendor/ffmpeg/windows_arm64/ffmpeg.exe
src-tauri/vendor/ffmpeg/windows_arm64/ffprobe.exe

src-tauri/vendor/ffmpeg/macos_x64/ffmpeg
src-tauri/vendor/ffmpeg/macos_x64/ffprobe
src-tauri/vendor/ffmpeg/macos_amd64/ffmpeg
src-tauri/vendor/ffmpeg/macos_amd64/ffprobe
src-tauri/vendor/ffmpeg/macos_arm64/ffmpeg
src-tauri/vendor/ffmpeg/macos_arm64/ffprobe

src-tauri/vendor/ffmpeg/linux_x64/ffmpeg
src-tauri/vendor/ffmpeg/linux_x64/ffprobe
src-tauri/vendor/ffmpeg/linux_amd64/ffmpeg
src-tauri/vendor/ffmpeg/linux_amd64/ffprobe
src-tauri/vendor/ffmpeg/linux_arm64/ffmpeg
src-tauri/vendor/ffmpeg/linux_arm64/ffprobe
```

`ffplay` 可选，缺少它不影响水印编码和解码。

添加或替换 FFmpeg 文件后，刷新清单：

```text
npm run ffmpeg:manifest
```

macOS 和 Linux 还要确保运行文件有执行权限：

```text
chmod +x src-tauri/vendor/ffmpeg/linux_x64/ffmpeg src-tauri/vendor/ffmpeg/linux_x64/ffprobe
chmod +x src-tauri/vendor/ffmpeg/linux_arm64/ffmpeg src-tauri/vendor/ffmpeg/linux_arm64/ffprobe
chmod +x src-tauri/vendor/ffmpeg/macos_x64/ffmpeg src-tauri/vendor/ffmpeg/macos_x64/ffprobe
chmod +x src-tauri/vendor/ffmpeg/macos_arm64/ffmpeg src-tauri/vendor/ffmpeg/macos_arm64/ffprobe
```

## 开发运行

安装前端依赖：

```text
npm install
```

启动桌面开发模式：

```text
npm run tauri:dev
```

Linux 开发模式会在 Tauri 窗口启动前临时写入用户级 `.desktop` 文件和 hicolor 图标，让 GNOME/Ubuntu Dock 能把调试窗口匹配到真实应用图标。正常退出时，包装脚本会删除这些临时文件。

如果终端被强制关闭，或应用菜单/Dock 里残留开发态图标，请在项目根目录运行任意一个清理命令：

```text
npm run tauri:dev:cleanup
npm run linux:desktop:cleanup
```

两个清理命令调用同一个脚本。它们只删除项目托管的临时开发入口并刷新桌面/图标缓存，不会卸载 `.deb` 或 `.rpm` 安装包。

## 打包命令

构建当前平台默认包：

```text
npm run tauri:build
```

### Windows

```text
npm run tauri:build:windows
npm run tauri:build:windows:nsis:x64
npm run tauri:build:windows:nsis:arm64
npm run tauri:build:windows:msi:x64
npm run tauri:build:windows:msi:arm64
npm run tauri:build:windows:all:x64
npm run tauri:build:windows:all:arm64
```

`windows` 短命令会为当前 Windows 宿主架构构建 NSIS 安装器。`all` 命令会同时构建 NSIS 和 MSI。

### macOS

```text
npm run tauri:build:macos
npm run tauri:build:macos:x64
npm run tauri:build:macos:arm64
npm run tauri:build:macos:dmg
npm run tauri:build:macos:x64:dmg
npm run tauri:build:macos:arm64:dmg
```

`macos` 构建可运行的 app bundle。`dmg` 命令会额外尝试生成磁盘镜像。

### Linux 稳定安装包

Ubuntu、Debian、Fedora、openSUSE 等包管理器场景，优先使用 DEB/RPM：

```text
npm run tauri:build:linux:installers:x64
npm run tauri:build:linux:deb:x64
npm run tauri:build:linux:rpm:x64
```

ARM64 也提供对应别名：

```text
npm run tauri:build:linux:installers:arm64
npm run tauri:build:linux:deb:arm64
npm run tauri:build:linux:rpm:arm64
```

从项目根目录安装生成的 DEB：

```text
sudo apt install ./release/privacy-watermark-codec-linux-x64.deb
```

如果当前目录已经是 `release/`，则使用：

```text
sudo apt install ./privacy-watermark-codec-linux-x64.deb
```

### Linux AppImage，可选

AppImage 被保留为可选的 Linux 单文件分发格式。它适合“下载一个文件、赋权后直接运行”的场景，但维护成本高于 DEB/RPM，因为它依赖 linuxdeploy、AppImage helper 下载、WebKitGTK/GStreamer 依赖收集和本机 ELF 修补工具。

正常 AppImage 流程：

```text
npm run tauri:build:linux:appimage:diagnose
npm run tauri:build:linux:appimage:prefetch
npm run tauri:build:linux:appimage:x64
```

运行生成的 AppImage：

```text
chmod u+x ./release/privacy-watermark-codec-linux-x64.AppImage
./release/privacy-watermark-codec-linux-x64.AppImage
```

只有排查失败时才建议打开详细日志：

```text
PWC_APPIMAGE_VERBOSE=1 npm run tauri:build:linux:appimage:x64
```

如果 GitHub release-asset 下载慢或被拦截，构建脚本会先把所需 helper 二进制缓存到 `target/.tauri/pwc-appimage-tools`，把 raw linuxdeploy helper 脚本缓存到 `target/.tauri`，再为 Tauri 启动临时本地镜像。也可以提供镜像模板：

```text
PWC_APPIMAGE_TOOLS_MIRROR_TEMPLATE=https://mirror.example.com/<owner>/<repo>/releases/download/<version>/<asset> npm run tauri:build:linux:appimage:prefetch
TAURI_BUNDLER_TOOLS_GITHUB_MIRROR_TEMPLATE=https://mirror.example.com/<owner>/<repo>/releases/download/<version>/<asset> npm run tauri:build:linux:appimage:x64
```

AppImage 打包期间，Linux FFmpeg 二进制会临时编码为 `.pwcbin` 非 ELF 资源，避免 linuxdeploy 对大型预编译 FFmpeg 执行 `patchelf`。打包结束后源码目录会自动恢复。运行 AppImage 时，Rust 后端会把 `.pwcbin` 还原到用户可写缓存目录，并继续按清单哈希校验。AppImage 构建会设置 `LDAI_NO_APPSTREAM=1`，因为 appimagetool 可能把可选 AppStream 元数据警告当作致命错误；DEB/RPM 仍会安装系统 metainfo 文件。

### Linux 全量命令

```text
npm run tauri:build:linux:x64:all
```

该命令会先构建 DEB 和 RPM，再尝试 AppImage。如果 DEB/RPM 已经成功，而 AppImage 失败，脚本会保留 `release/` 中已经成功的安装包，并把 AppImage 标记为可选失败项。

## 发布产物

发布脚本会把可分发产物复制到：

```text
release/
```

规范文件名示例：

```text
privacy-watermark-codec-windows-x64.exe
privacy-watermark-codec-windows-x64.msi
privacy-watermark-codec-macos-x64.app
privacy-watermark-codec-macos-x64.dmg
privacy-watermark-codec-linux-x64.deb
privacy-watermark-codec-linux-x64.rpm
privacy-watermark-codec-linux-x64.AppImage
```

Tauri 原始输出仍保留在 `target/**/release/bundle/`。

## 运行数据存储策略

便携运行和开发模式会优先使用可执行文件旁边的可写目录：

```text
PrivacyWatermarkCodecData/
├─ webview-data/
└─ work/
```

Linux DEB/RPM 通常安装到普通用户不可写的系统路径。此时软件会自动回退到用户数据目录，例如：

```text
~/.local/share/privacy-watermark-codec/PrivacyWatermarkCodecData/
├─ webview-data/
└─ work/
```

该回退用于避免 Linux 安装包在创建 WebView 窗口前直接退出。

## 软件使用

### 编码

1. 进入编码模式。
2. 选择图片或视频文件。
3. 选择输出目录。
4. 输入水印文本。
5. 选择独立密钥、批次共享密钥或自定义密码。
6. 开始编码。
7. 保存输出媒体、生成的 `.key` 文件和证据清单。

### 解码

1. 进入解码模式。
2. 选择已编码的图片或视频。
3. 选择 `.key` 文件，或输入原始自定义密码。
4. 开始解码与检测。
5. 查看提取文本、篡改状态和疑似篡改区域。

### 扫描

1. 选择未知图片，或从 Windows 右键菜单导入。
2. 开始扫描。
3. 查看是否发现项目水印头、隐私元数据或 AI 水印痕迹。

检测到项目水印头只表示图片可能包含本项目加密水印；没有密钥或密码时不会显示水印正文。未检出水印也不代表图片一定没有水印。

## 当前验证状态

| 平台 | 当前状态 | 说明 |
| --- | --- | --- |
| Windows x64 | 已验证 | 编码、解码、打包和 NSIS 安装器流程已在早期验证中通过。 |
| Windows ARM64 | 已支持，待真机复核 | 脚本和 FFmpeg 目录名已预留，发布前需要真机测试。 |
| macOS ARM64 | 已验证 | Apple Silicon 构建流程和本地运行已在早期验证中通过。 |
| macOS x64 | 已支持 | `macos_x64` 是推荐的 Intel Mac 目录；`macos_amd64` 作为兼容别名保留。 |
| Linux x64 / amd64 DEB | 已验证 | 安装包可安装并启动；系统安装路径不可写时运行数据会回退到用户数据目录。 |
| Linux x64 / amd64 RPM | 已通过本地转换/安装流程验证 | 安装后可启动；仍建议在原生 RPM 发行版上复核。 |
| Linux x64 / amd64 AppImage | 可选格式 | 当前失败已经进入 appimagetool 阶段，并卡在可选 AppStream 校验。AppImage 构建现在设置 `LDAI_NO_APPSTREAM=1`；DEB/RPM 仍保留系统 metainfo。 |
| Linux ARM64 | 已支持，待真机复核 | 脚本和 FFmpeg 目录名已预留，需要在目标硬件上用 ARM64 FFmpeg 测试。 |

## 安全提醒

- `.key` 文件包含可用于解密的派生密钥材料，应按密码文件保管。
- 自定义密码模式的安全性取决于密码强度和保密性。
- 视频处理前会按清单校验 FFmpeg 文件；替换 FFmpeg 后必须重新生成清单再发布。

## 常见问题

### AppImage 提示缺少工具

```text
npm run tauri:build:linux:appimage:diagnose
sudo apt install patchelf binutils file
```


### AppImage 在 AppStream 校验阶段失败

如果 `pwc-appimage-build.log` 中出现 `Failed to validate AppStream information with appstreamcli`，请使用当前更新后的代码重新构建。AppImage 构建现在会设置 `linuxdeploy-plugin-appimage` 支持的 `LDAI_NO_APPSTREAM=1`，因为 AppImage 元数据警告可能让 `appimagetool` 直接失败；DEB/RPM 安装包仍会包含系统 metainfo。

### AppImage 输出太长

正常构建不要使用 `PWC_APPIMAGE_VERBOSE=1`。只有 AppImage 失败且需要查看 `target/**/release/pwc-appimage-build.log` 时再打开详细输出。

### Linux Dock 仍显示旧图标

先取消固定旧图标，然后清理开发态入口并重新启动：

```text
npm run linux:desktop:cleanup
```

如果安装过系统包，可以刷新桌面缓存或注销后重新登录。
