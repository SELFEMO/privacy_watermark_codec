# GitHub 上传命令

本文件只记录维护者把本地项目上传到 GitHub 所需的 Git / Git LFS 命令，不包含安装、运行、构建或使用命令。

仓库地址：

```text
https://github.com/SELFEMO/privacy_watermark_codec.git
```

## 进入项目目录

只有进入目录的命令因平台不同而不同。

Windows Command Prompt：

```text
cd /d D:\MyWorkstation\Learning\Rust\privacy_watermark_codec
```

Windows PowerShell：

```text
Set-Location D:\MyWorkstation\Learning\Rust\privacy_watermark_codec
```

macOS / Linux：

```text
cd ~/Learning/Rust/privacy_watermark_codec
```

## 首次上传到 GitHub

```text
git init
git lfs install
git lfs track "src-tauri/vendor/ffmpeg/**"
git add .gitattributes
git add .
git commit -m "Initial project upload"
git branch -M main
git remote add origin https://github.com/SELFEMO/privacy_watermark_codec.git
git push -u origin main
git lfs push --all origin main
```

## 已有仓库后继续上传修改

```text
git status
git add .
git commit -m "Update project"
git push origin main
git lfs push --all origin main
```

## 远端地址已存在但需要修正

```text
git remote set-url origin https://github.com/SELFEMO/privacy_watermark_codec.git
git push -u origin main
git lfs push --all origin main
```

## 只补传 Git LFS 对象

如果代码已经推送成功，但别人克隆后拿不到 `src-tauri/vendor/ffmpeg` 中的真实二进制文件，维护者在本地确认这些文件存在后执行：

```text
git lfs push --all origin main
```

如果项目有多个分支或标签，也可以补传所有引用到的 LFS 对象：

```text
git lfs push --all origin --all
git lfs push --all origin --tags
```

## 上传前查看 FFmpeg 是否已被 LFS 跟踪

```text
git lfs ls-files
```

输出中应能看到 `src-tauri/vendor/ffmpeg/...` 下的 `ffmpeg`、`ffprobe` 或 `ffmpeg.exe`、`ffprobe.exe`。如果看不到，说明这些大文件还没有被 Git LFS 跟踪，需要重新执行首次上传中的 `git lfs track`、`git add .gitattributes`、`git add .`、`git commit`、`git push` 和 `git lfs push --all origin main`。
