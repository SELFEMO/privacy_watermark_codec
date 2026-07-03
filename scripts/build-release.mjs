import { spawn, spawnSync } from "node:child_process";
import { appendFileSync, chmodSync, cpSync, existsSync, mkdirSync, readdirSync, renameSync, rmSync, statSync, unlinkSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { startAppImageToolsMirror } from "./appimage-tools.mjs";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const configPath = join(projectRoot, "src-tauri", "tauri.conf.json");
const ffmpegManifestPath = join(projectRoot, "src-tauri", "vendor", "ffmpeg", "manifest.json");
const releaseOutputDir = join(projectRoot, "release");
const encodedFfmpegMagic = Buffer.from("PWC_FFMPEG_XOR_V1\n", "utf8");
const encodedFfmpegXorKey = 0xA5;
const appImageFfmpegBackupDir = join(projectRoot, "target", ".tauri", "pwc-appimage-ffmpeg-backup");
const appImageFfmpegLegacyBackupSuffix = ".pwc-appimage-raw";
const appImageFfmpegEncodedSuffix = ".pwcbin";
const args = process.argv.slice(2);

function optionValue(name) {
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : null;
}

const explicitTargetTriple = optionValue("--target");
const archAlias = optionValue("--arch");

function targetTripleFromArchAlias(alias) {
  if (!alias) return null;
  const normalized = alias.toLowerCase();
  // x64/arm64 是项目推荐的脚本称谓；保留 amd64/aarch64 输入是为了兼容 Linux 包架构和 Rust 目标三元组习惯。
  // x64/arm64 are the recommended script names; amd64/aarch64 inputs stay supported for Linux package-architecture and Rust target-triple conventions.
  const isX64 = normalized === "x64" || normalized === "amd64" || normalized === "x86_64";
  const isArm64 = normalized === "arm64" || normalized === "aarch64";
  if (!isX64 && !isArm64) {
    throw new Error(`Unsupported architecture alias: ${alias}`);
  }

  if (process.platform === "win32") return isArm64 ? "aarch64-pc-windows-msvc" : "x86_64-pc-windows-msvc";
  if (process.platform === "darwin") return isArm64 ? "aarch64-apple-darwin" : "x86_64-apple-darwin";
  return isArm64 ? "aarch64-unknown-linux-gnu" : "x86_64-unknown-linux-gnu";
}

const requestedTargetTriple = explicitTargetTriple || targetTripleFromArchAlias(archAlias);
const ignoredOptionIndexes = new Set();
for (const name of ["--target", "--arch"]) {
  const index = args.indexOf(name);
  if (index >= 0) {
    ignoredOptionIndexes.add(index);
    ignoredOptionIndexes.add(index + 1);
  }
}
const requestedBundle = args.find((arg, index) => !ignoredOptionIndexes.has(index) && !arg.startsWith("--"));

function defaultBundleForPlatform() {
  if (process.platform === "win32") return "nsis";
  if (process.platform === "darwin") return "app";
  if (process.platform === "linux") return "appimage";
  return "app";
}

function targetPlatformFromTriple(targetTriple) {
  if (!targetTriple) {
    if (process.platform === "win32") return process.arch === "arm64" ? "windows_arm64" : "windows_x64";
    if (process.platform === "darwin") return process.arch === "arm64" ? "macos_arm64" : "macos_x64";
    return process.arch === "arm64" ? "linux_arm64" : "linux_x64";
  }
  if (targetTriple.includes("apple-darwin")) return targetTriple.startsWith("aarch64") ? "macos_arm64" : "macos_x64";
  if (targetTriple.includes("windows")) return targetTriple.startsWith("aarch64") ? "windows_arm64" : "windows_x64";
  if (targetTriple.includes("linux")) return targetTriple.startsWith("aarch64") ? "linux_arm64" : "linux_x64";
  return null;
}

function bundleNamesFromText(value) {
  if (value === "all") return ["nsis", "msi", "app", "dmg", "appimage", "deb", "rpm"];
  return value.split(",").map((item) => item.trim()).filter(Boolean);
}

function hostTargetTripleForCurrentPlatform() {
  if (process.platform === "win32") return process.arch === "arm64" ? "aarch64-pc-windows-msvc" : "x86_64-pc-windows-msvc";
  if (process.platform === "darwin") return process.arch === "arm64" ? "aarch64-apple-darwin" : "x86_64-apple-darwin";
  return process.arch === "arm64" ? "aarch64-unknown-linux-gnu" : "x86_64-unknown-linux-gnu";
}

function shouldPinLinuxHostTarget(bundleText) {
  if (requestedTargetTriple || process.platform !== "linux") return false;
  const names = bundleNamesFromText(bundleText);
  // 单独 AppImage 已在显式 target 目录中验证通过；默认 Linux AppImage 命令也固定宿主 target，避免 deb/rpm 与 AppImage 复用 target/release 的中间状态。
  // Standalone AppImage has been verified in an explicit-target directory; default Linux AppImage commands pin the host target too so deb/rpm and AppImage do not reuse target/release intermediates.
  return names.includes("appimage");
}

const bundle = requestedBundle || defaultBundleForPlatform();
const targetTriple = requestedTargetTriple || (shouldPinLinuxHostTarget(bundle) ? hostTargetTripleForCurrentPlatform() : null);
const targetPlatform = targetPlatformFromTriple(targetTriple);
if (!requestedTargetTriple && targetTriple) {
  console.log(`Using host target ${targetTriple} for Linux AppImage-compatible bundling.`);
}

function sanitize(text) {
  // 构建日志中的体积、耗时和包文件名本身就是排错依据；这里不再删除数字，避免把 Vite/RPM/Tauri 输出清空成误导信息。
  // Build sizes, timings, and package file names are diagnostics, so numbers are no longer stripped from Vite/RPM/Tauri output.
  return text;
}

function quoteForShell(value) {
  return `"${String(value).replace(/"/g, '\\"')}"`;
}

function writeLogFile(logPath, text, append = false) {
  mkdirSync(dirname(logPath), { recursive: true });
  if (append) appendFileSync(logPath, text, "utf8");
  else writeFileSync(logPath, text, "utf8");
}

function commandExists(command) {
  const result = spawnSync("sh", ["-lc", `command -v ${command}`], { encoding: "utf8" });
  return result.status === 0 ? result.stdout.trim() : null;
}


function run(command, args, filterOutput = false, extraEnv = {}, options = {}) {
  return new Promise((resolveRun, rejectRun) => {
    const useShell = process.platform === "win32";
    const finalCommand = useShell ? quoteForShell(command) : command;
    const env = { ...process.env, NO_COLOR: "1" };
    for (const [key, value] of Object.entries(extraEnv)) {
      if (value === null) delete env[key];
      else env[key] = value;
    }

    if (options.logPath) {
      // Tauri 可能会在 AppImage 打包开始时重建 bundle 目录；每次写日志前都确保目录存在，避免日志功能反过来中断构建。
      // Tauri may recreate bundle directories when AppImage bundling starts; ensuring the directory before each write prevents logging from interrupting the build.
      writeLogFile(options.logPath, `$ ${command} ${args.join(" ")}
`);
    }

    const child = spawn(finalCommand, args, {
      cwd: projectRoot,
      shell: useShell,
      windowsHide: true,
      // 额外环境变量只注入当前子进程，避免把 Linux AppImage 的兼容开关污染到其它 npm/cargo 步骤。
      // Extra environment variables are scoped to this child process so Linux AppImage compatibility flags do not leak into other npm/cargo steps.
      env,
      stdio: ["ignore", "pipe", "pipe"],
    });

    const writeChunk = (stream, chunk) => {
      const text = chunk.toString();
      const output = filterOutput ? sanitize(text) : text;
      stream.write(output);
      if (options.logPath) writeLogFile(options.logPath, output, true);
    };

    child.stdout.on("data", (chunk) => writeChunk(process.stdout, chunk));
    child.stderr.on("data", (chunk) => writeChunk(process.stderr, chunk));
    child.on("error", rejectRun);
    child.on("close", (code) => {
      if (code === 0) {
        resolveRun();
      } else {
        const error = new Error(`${command} ${args.join(" ")} failed`);
        if (options.logPath) error.logPath = options.logPath;
        rejectRun(error);
      }
    });
  });
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function bundleNamesFromRequest() {
  return bundleNamesFromText(bundle);
}

function buildOrderBundleNames() {
  const names = bundleNamesFromRequest();
  if (process.platform !== "linux" || names.length <= 1 || !names.includes("appimage")) {
    return names;
  }
  // Linux 的 AppImage 阶段需要下载和运行额外打包工具；把 deb/rpm 放在前面可确保网络超时时仍先产出可安装包。
  // Linux AppImage needs extra downloaded bundling tools; building deb/rpm first ensures installable packages exist before any network timeout.
  return [...names.filter((name) => name !== "appimage"), "appimage"];
}

function tauriBuildArgs(bundleName) {
  const tauriArgs = ["build", "--bundles", bundleName];
  if (targetTriple) {
    // 目标三元组只影响当前构建任务，用于分别产出 x64 与 arm64 包，避免把平台架构写入项目配置。
    // The target triple is kept per build so x64 and arm64 packages can be produced without hard-coding one architecture in project config.
    tauriArgs.push("--target", targetTriple);
  }
  if (process.platform === "linux" && bundleName === "appimage" && process.env.PWC_APPIMAGE_VERBOSE === "1") {
    // AppImage 失败点通常藏在 linuxdeploy 子进程内；按需开启 verbose 可保留常规构建输出的可读性。
    // AppImage failures are usually hidden inside the linuxdeploy subprocess; opt-in verbose keeps normal build output readable.
    tauriArgs.push("--verbose");
  }
  return tauriArgs;
}

function environmentForBundle(bundleName) {
  if (process.platform === "linux" && bundleName === "appimage") {
    return {
      // AppImage 工具自身也是 AppImage；在无 FUSE 或受限容器中展开运行可减少 Linux 打包失败面。
      // AppImage tools are AppImages themselves; extract-and-run reduces Linux bundling failures in no-FUSE or restricted environments.
      APPIMAGE_EXTRACT_AND_RUN: "1",
      // linuxdeploy 会遍历 AppDir 并尝试 strip；项目会携带 FFmpeg 与图标资源，禁用 strip 能避免非 ELF 文件误处理导致 AppImage 阶段失败。
      // linuxdeploy walks the AppDir and may try to strip files; this project ships FFmpeg and icon assets, so disabling strip avoids failures on non-ELF files.
      NO_STRIP: "true",
      // Tauri/linuxdeploy 会生成或复制 AppStream 元数据，但 appimagetool 会把元数据警告当作构建失败；跳过可选校验可保留可运行 AppImage，同时 DEB/RPM 仍保留系统元数据。
      // Tauri/linuxdeploy can generate or copy AppStream metadata, and appimagetool treats metadata warnings as build failures; skipping this optional check keeps the AppImage buildable while DEB/RPM still ship system metadata.
      LDAI_NO_APPSTREAM: "1",
      // 可复现时间戳会改变 appimagetool 的打包路径；清掉它可以减少不同发行版 shell 环境对 AppImage 的干扰。
      // Reproducible timestamp variables alter appimagetool packaging behavior; clearing it reduces distribution-specific environment interference.
      SOURCE_DATE_EPOCH: null,
    };
  }
  return {};
}

function appImageBundleLogPath(bundleName) {
  if (process.platform !== "linux" || bundleName !== "appimage") return null;
  const releaseRoot = targetTriple
    ? join(projectRoot, "target", targetTriple, "release")
    : join(projectRoot, "target", "release");
  // 日志放在 release 根目录而不是 bundle/appimage 下，因为 Tauri/linuxdeploy 会清理或重建 bundle 子目录。
  // The log lives under the release root instead of bundle/appimage because Tauri/linuxdeploy can clean or recreate bundle subdirectories.
  return join(releaseRoot, "pwc-appimage-build.log");
}

function missingAppImagePrerequisiteCommands() {
  if (process.platform !== "linux") return [];
  if (process.env.PWC_SKIP_APPIMAGE_PREREQ_CHECK === "1") return [];
  // linuxdeploy 依赖这些宿主机命令来识别和修补 ELF 依赖；提前检查能把晦涩的 linuxdeploy 失败转成可执行提示。
  // linuxdeploy relies on these host commands to inspect and patch ELF dependencies; checking early turns opaque linuxdeploy failures into actionable guidance.
  return ["file", "ldd", "readelf", "strip", "patchelf"].filter((command) => !commandExists(command));
}

function appImagePrerequisiteFailureReason(bundleName) {
  if (process.platform !== "linux" || bundleName !== "appimage") return null;
  const missing = missingAppImagePrerequisiteCommands();
  if (missing.length === 0) return null;
  return `Missing AppImage build prerequisite command(s): ${missing.join(", ")}. On Ubuntu/Debian, install them with: sudo apt install patchelf binutils file`;
}

function ffmpegBinaryEntriesForPlatform(platform) {
  if (!platform || !existsSync(ffmpegManifestPath)) return [];
  const manifest = JSON.parse(readFileSync(ffmpegManifestPath, "utf8"));
  const platformEntry = manifest.platforms?.[platform];
  if (!platformEntry) return [];

  return [platformEntry.ffmpeg, platformEntry.ffprobe, platformEntry.ffplay]
    .filter((entry) => entry?.file)
    .map((entry) => entry.file);
}

function encodeFfmpegBinaryBytes(bytes) {
  const encoded = Buffer.alloc(encodedFfmpegMagic.length + bytes.length);
  encodedFfmpegMagic.copy(encoded, 0);
  for (let index = 0; index < bytes.length; index += 1) {
    encoded[encodedFfmpegMagic.length + index] = bytes[index] ^ encodedFfmpegXorKey;
  }
  return encoded;
}

function restorePossiblyStaleAppImageFfmpegFile(filePath, backupPath) {
  const legacyBackupPath = `${filePath}${appImageFfmpegLegacyBackupSuffix}`;
  const encodedPath = `${filePath}${appImageFfmpegEncodedSuffix}`;
  if (!existsSync(filePath) && existsSync(backupPath)) {
    renameSync(backupPath, filePath);
  }
  if (!existsSync(filePath) && existsSync(legacyBackupPath)) {
    renameSync(legacyBackupPath, filePath);
  }
  rmSync(legacyBackupPath, { force: true });
  rmSync(encodedPath, { force: true });
}

function prepareAppImageFfmpegResources(platform) {
  if (process.platform !== "linux" || !platform?.startsWith("linux")) return () => {};
  if (process.env.PWC_APPIMAGE_RAW_FFMPEG === "1") return () => {};

  const vendorDir = join(projectRoot, "src-tauri", "vendor", "ffmpeg", platform);
  const backupDir = join(appImageFfmpegBackupDir, platform);
  mkdirSync(backupDir, { recursive: true });
  const transformed = [];
  const restore = () => {
    for (const item of [...transformed].reverse()) {
      rmSync(item.encodedPath, { force: true });
      if (!existsSync(item.rawPath) && existsSync(item.backupPath)) {
        renameSync(item.backupPath, item.rawPath);
      } else if (existsSync(item.backupPath)) {
        rmSync(item.backupPath, { force: true });
      }
    }
  };

  try {
    for (const fileName of ffmpegBinaryEntriesForPlatform(platform)) {
      const rawPath = join(vendorDir, fileName);
      const backupPath = join(backupDir, fileName);
      restorePossiblyStaleAppImageFfmpegFile(rawPath, backupPath);
      if (!existsSync(rawPath)) continue;

      const encodedPath = `${rawPath}${appImageFfmpegEncodedSuffix}`;
      rmSync(backupPath, { force: true });
      const rawBytes = readFileSync(rawPath);
      // AppImage 的 linuxdeploy 会扫描 AppDir 中所有 ELF 文件并尝试 patchelf；原始 FFmpeg 必须移到资源通配符目录外，否则备份文件也会被识别为 ELF。
      // linuxdeploy scans every ELF in the AppDir and tries to patchelf it; raw FFmpeg must be moved outside the resource wildcard directory or even backups will be detected as ELF.
      renameSync(rawPath, backupPath);
      writeFileSync(encodedPath, encodeFfmpegBinaryBytes(rawBytes));
      chmodSync(encodedPath, 0o644);
      transformed.push({ rawPath, backupPath, encodedPath });
    }
  } catch (error) {
    restore();
    throw error;
  }

  if (transformed.length > 0) {
    console.log("Encoded FFmpeg binaries for AppImage packaging so linuxdeploy will not patch them as ELF files:");
    for (const item of transformed) console.log(`  ${item.encodedPath}`);
  }

  return restore;
}

function readAppImageFailureLog(error) {
  const logPath = error?.logPath;
  if (!logPath || !existsSync(logPath)) return "";
  try {
    return readFileSync(logPath, "utf8");
  } catch {
    return "";
  }
}

function appImageFailureSummary(error) {
  const text = `${error?.message || ""}
${readAppImageFailureLog(error)}`;
  if (/timeout: global|timed out after|release-assets\.githubusercontent\.com|TLS close_notify|No address associated with hostname/i.test(text)) {
    return "AppImage helper download failed before linuxdeploy could finish; the script now caches helpers locally and asks Tauri to read them from localhost.";
  }
  if (/maximum file size exceeded|Failed to set rpath in ELF file: .*vendor\/ffmpeg|Call to patchelf failed/i.test(text)) {
    return "linuxdeploy tried to patch bundled FFmpeg ELF files; the build now encodes FFmpeg resources for AppImage and restores them at runtime.";
  }
  if (/Could not find suitable icon for Icon entry/i.test(text)) {
    return "linuxdeploy could not match the desktop Icon entry to an AppDir icon; AppImage installs the com.privacywatermark.codec icon into hicolor via linux.appimage.files.";
  }
  if (/Failed to validate AppStream information|appstreamcli|desktop-file-not-found|metainfo-filename-cid-mismatch/i.test(text)) {
    return "appimagetool failed on optional AppStream metadata validation; the build now sets LDAI_NO_APPSTREAM=1 for AppImage while DEB/RPM still keep system metadata.";
  }
  if (/failed to run linuxdeploy/i.test(text)) {
    return "AppImage helper download completed, but linuxdeploy returned a non-zero exit code.";
  }
  return "AppImage bundling failed.";
}

function linuxScriptTargetSuffix() {
  const platform = targetPlatform || targetPlatformFromTriple(null);
  // 命令提示必须跟随实际构建目标，而不是固定写 x64，否则 arm64 失败时会把用户引回错误架构。
  // Command hints must follow the actual build target instead of hard-coding x64, otherwise arm64 failures send users back to the wrong architecture.
  return platform === "linux_arm64" ? ":arm64" : ":x64";
}

function printAppImageFailureHelp(error) {
  const logPath = error?.logPath;
  const targetSuffix = linuxScriptTargetSuffix();
  console.warn(appImageFailureSummary(error));
  if (logPath) {
    console.warn(`Full AppImage build log: ${logPath}`);
  }
  console.warn(`Run npm run tauri:build:linux:appimage:diagnose${targetSuffix} to check local AppImage build prerequisites and download probes.`);
  console.warn("The build script automatically creates a local helper mirror and sets LDAI_NO_APPSTREAM=1 because AppStream warnings are optional but can fail appimagetool.");
  console.warn(`To bypass the local helper mirror for comparison, run PWC_APPIMAGE_USE_TAURI_DOWNLOADER=1 npm run tauri:build:linux:appimage${targetSuffix}.`);
  console.warn("If your network blocks GitHub release assets, set PWC_APPIMAGE_TOOLS_MIRROR_TEMPLATE or TAURI_BUNDLER_TOOLS_GITHUB_MIRROR_TEMPLATE.");
}

function canContinueAfterBundleFailure(bundleName, orderedBundleNames, successfulBundles) {
  const hasLinuxInstaller = successfulBundles.some((name) => name === "deb" || name === "rpm");
  // 多目标 Linux 构建中，某些格式依赖额外系统工具或网络下载；已生成安装包后继续收集产物，比整体失败更符合发布脚本用途。
  // In multi-target Linux builds, some formats need extra system tools or downloads; once an installer exists, collecting outputs is more useful than failing the whole release script.
  return process.platform === "linux"
    && orderedBundleNames.length > 1
    && hasLinuxInstaller
    && ["appimage", "rpm"].includes(bundleName);
}

function bundleOutputDirectoryNames(bundleName) {
  // Tauri 的 macOS app 目标名是 app，但实际输出目录通常是 bundle/macos；同时保留 app 目录作为兼容兜底。
  // Tauri names the macOS app target as app, but its output directory is usually bundle/macos; app is kept as a fallback for compatibility.
  if (bundleName === "app") return ["macos", "app"];
  return [bundleName];
}

function bundleDirectories(bundleName) {
  const targetRoot = targetTriple ? join(projectRoot, "target", targetTriple, "release", "bundle") : join(projectRoot, "target", "release", "bundle");
  const fallbackRoot = join(projectRoot, "target", "release", "bundle");
  const directories = [];
  for (const directoryName of bundleOutputDirectoryNames(bundleName)) {
    directories.push(join(targetRoot, directoryName));
    if (targetRoot !== fallbackRoot) directories.push(join(fallbackRoot, directoryName));
  }
  return [...new Set(directories)].filter((directory) => existsSync(directory));
}

function renameBundleFiles(bundleNames = bundleNamesFromRequest()) {
  const config = JSON.parse(readFileSync(configPath, "utf8"));
  const appVersion = config.version;

  for (const bundleName of bundleNames) {
    const rules = releaseArtifactRules(bundleName);
    for (const actualBundleDir of bundleDirectories(bundleName)) {
      for (const file of readdirSync(actualBundleDir)) {
        const source = join(actualBundleDir, file);
        const stats = statSync(source);
        if (!rules.some((rule) => rule.directory === stats.isDirectory() && file.endsWith(rule.extension))) {
          // Tauri 的 deb/rpm 目录里可能保留无扩展名的中间文件；只重命名最终产物，避免终端输出出现看似缺少扩展名的包名。
          // Tauri deb/rpm directories may keep extensionless intermediates; only final artifacts are renamed so terminal output does not show package names that appear to miss extensions.
          continue;
        }

        const versionPattern = new RegExp(`_${escapeRegExp(appVersion)}(?=_)`, "g");
        const renamed = file.replace(versionPattern, "");
        if (renamed === file) continue;

        const target = join(actualBundleDir, renamed);
        if (existsSync(target)) unlinkSync(target);
        renameSync(source, target);
        console.log(`Renamed bundle: ${renamed}`);
      }
    }
  }
}

function normalizedReleaseTarget() {
  const platform = targetPlatform || targetPlatformFromTriple(null);
  if (!platform) {
    throw new Error("Cannot determine release platform for copied package names.");
  }

  const [osName, archName] = platform.split("_");
  const normalizedPlatform = osName === "darwin" ? "macos" : osName;
  const normalizedArch = archName === "amd64" ? "x64" : archName;
  return { platform: normalizedPlatform, arch: normalizedArch };
}

function releaseArtifactRules(bundleName) {
  switch (bundleName) {
    case "nsis":
      return [{ extension: ".exe", directory: false }];
    case "msi":
      return [{ extension: ".msi", directory: false }];
    case "appimage":
      return [{ extension: ".AppImage", directory: false }];
    case "deb":
      return [{ extension: ".deb", directory: false }];
    case "rpm":
      return [{ extension: ".rpm", directory: false }];
    case "dmg":
      return [{ extension: ".dmg", directory: false }];
    case "app":
      return [{ extension: ".app", directory: true }];
    default:
      return [];
  }
}

function copyReleaseArtifacts(bundleNames = bundleNamesFromRequest()) {
  const config = JSON.parse(readFileSync(configPath, "utf8"));
  const softwareName = config.mainBinaryName || String(config.productName || "privacy-watermark-codec").toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
  const { platform, arch } = normalizedReleaseTarget();
  const copied = [];

  mkdirSync(releaseOutputDir, { recursive: true });

  for (const bundleName of bundleNames) {
    const rules = releaseArtifactRules(bundleName);
    if (rules.length === 0) continue;

    for (const actualBundleDir of bundleDirectories(bundleName)) {
      for (const file of readdirSync(actualBundleDir)) {
        const source = join(actualBundleDir, file);
        const stats = statSync(source);
        const rule = rules.find((candidate) => candidate.directory === stats.isDirectory() && file.endsWith(candidate.extension));
        if (!rule) continue;

        const releaseName = `${softwareName}-${platform}-${arch}${rule.extension}`;
        const destination = join(releaseOutputDir, releaseName);
        // release 目录只保留规范命名产物，便于人工查找和 CI 上传，不依赖 Tauri 默认文件名中的版本或本地化描述。
        // The release directory keeps canonical package names for manual lookup and CI upload, independent of Tauri's versioned or localized default file names.
        rmSync(destination, { recursive: true, force: true });
        cpSync(source, destination, { recursive: true });
        copied.push(destination);
      }
    }
  }

  if (copied.length === 0) {
    console.warn(`No release package was copied to ${releaseOutputDir}. Check target/*/release/bundle for bundler outputs.`);
    return;
  }

  console.log(`Copied release packages to ${releaseOutputDir}:`);
  for (const path of [...new Set(copied)]) {
    console.log(`  ${path}`);
  }
}

function targetFfmpegResourceDirectories(platform) {
  switch (platform) {
    case "windows_x64":
      return ["windows_x64"];
    case "windows_arm64":
      return ["windows_arm64"];
    case "macos_arm64":
      return ["macos_arm64"];
    case "macos_x64":
      return ["macos_x64"];
    case "macos_amd64":
      return ["macos_amd64"];
    case "linux_arm64":
      return ["linux_arm64"];
    case "linux_x64":
      return ["linux_x64"];
    case "linux_amd64":
      return ["linux_amd64"];
    default:
      return [];
  }
}

function platformRuntimeCandidates(platform) {
  switch (platform) {
    case "macos_x64":
    case "macos_amd64":
      return ["macos_x64", "macos_amd64"];
    case "linux_x64":
    case "linux_amd64":
      return ["linux_x64", "linux_amd64"];
    default:
      return platform ? [platform] : [];
  }
}

function resolveFfmpegPlatformForBundle(platform) {
  if (!platform || !existsSync(ffmpegManifestPath)) return platform;
  const manifest = JSON.parse(readFileSync(ffmpegManifestPath, "utf8"));
  const platforms = manifest.platforms || {};
  // 打包资源必须匹配清单中实际存在的 FFmpeg 目录；这样 x64 目标仍能复用 linux_amd64/macos_amd64 兼容目录。
  // Bundled resources must match the FFmpeg directory actually present in the manifest, so x64 targets can still use linux_amd64/macos_amd64 compatibility folders.
  return platformRuntimeCandidates(platform).find((candidate) => platforms[candidate]) || platform;
}

function targetBundleResources(platform) {
  const common = [
    "vendor/ffmpeg/manifest.json",
    "vendor/ffmpeg/LICENSE.txt",
    "vendor/ffmpeg/README.md",
    "vendor/ffmpeg/VERSION.txt",
  ];

  return [
    ...common,
    ...targetFfmpegResourceDirectories(platform).map((directory) => `vendor/ffmpeg/${directory}/*`),
  ];
}

function isFfmpegResource(entry) {
  if (typeof entry === "string") {
    return entry.replace(/\\/g, "/").includes("vendor/ffmpeg/");
  }
  return JSON.stringify(entry).includes("vendor/ffmpeg/");
}

function patchTauriResourcesForTarget(platform) {
  if (!platform) {
    return () => {};
  }

  const original = readFileSync(configPath, "utf8");
  const config = JSON.parse(original);
  const currentResources = Array.isArray(config.bundle?.resources) ? config.bundle.resources : [];
  const nonFfmpegResources = currentResources.filter((entry) => !isFfmpegResource(entry));
  const resources = [...nonFfmpegResources, ...targetBundleResources(platform)];

  config.bundle = { ...(config.bundle || {}), resources };
  writeFileSync(configPath, `${JSON.stringify(config, null, 2)}
`);

  console.log(`Bundling FFmpeg resources for target platform only: ${platform}`);
  for (const resource of resources.filter((entry) => isFfmpegResource(entry))) {
    console.log(`  ${resource}`);
  }

  return () => writeFileSync(configPath, original);
}

const nodePath = process.execPath;
const manifestScript = join(projectRoot, "scripts", "generate-ffmpeg-manifest.mjs");
const preflightScript = join(projectRoot, "scripts", "preflight-tauri.mjs");
const tauriBin = process.platform === "win32"
  ? join(projectRoot, "node_modules", ".bin", "tauri.cmd")
  : join(projectRoot, "node_modules", ".bin", "tauri");

// const tauriArgs = ["build", "--bundles", bundle];
// if (targetTriple) {
//   // 目标三元组只影响当前构建任务，用于分别产出 x64 与 arm64 包，避免把平台架构写入项目配置。
//   // The target triple is kept per build so x64 and arm64 packages can be produced without hard-coding one architecture in project config.
//   tauriArgs.push("--target", targetTriple);
// }

const manifestArgs = [manifestScript, "--strict"];
if (targetPlatform) {
  // 发布构建按目标平台校验 FFmpeg，避免交叉构建时误用当前宿主架构。
  // Release builds validate FFmpeg for the target platform so cross-builds do not accidentally use the host architecture.
  manifestArgs.push("--target-platform", targetPlatform);
}

await run(nodePath, [preflightScript]);
await run(nodePath, manifestArgs);
const bundlePlatform = resolveFfmpegPlatformForBundle(targetPlatform);
const restoreTauriResources = patchTauriResourcesForTarget(bundlePlatform);
try {
  const orderedBundleNames = buildOrderBundleNames();
  const successfulBundles = [];
  const ignoredFailures = [];
  for (const bundleName of orderedBundleNames) {
    const prerequisiteFailureReason = appImagePrerequisiteFailureReason(bundleName);
    if (prerequisiteFailureReason) {
      const error = new Error(prerequisiteFailureReason);
      error.suppressStack = true;
      if (!canContinueAfterBundleFailure(bundleName, orderedBundleNames, successfulBundles)) {
        throw error;
      }
      ignoredFailures.push({ bundleName, error });
      console.warn(`Skipping Linux ${bundleName} bundling: ${prerequisiteFailureReason}`);
      console.warn(`For an Ubuntu installer that avoids AppImage host-tool requirements, run npm run tauri:build:linux:deb${linuxScriptTargetSuffix()}.`);
      continue;
    }

    let appImageToolsMirror = null;
    let restoreAppImageFfmpegResources = () => {};
    try {
      const bundleEnv = environmentForBundle(bundleName);
      if (process.platform === "linux" && bundleName === "appimage") {
        // 先把 Tauri 需要的 GitHub release helper 缓存到本地镜像，是为了绕过 release-assets CDN 在慢网络下触发的 Tauri 全局下载超时。
        // Caching Tauri's GitHub release helpers in a local mirror first avoids the global Tauri download timeout triggered by slow release-assets CDN responses.
        appImageToolsMirror = await startAppImageToolsMirror({ projectRoot, targetPlatform });
        if (appImageToolsMirror?.env) Object.assign(bundleEnv, appImageToolsMirror.env);
        restoreAppImageFfmpegResources = prepareAppImageFfmpegResources(bundlePlatform);
      }
      await run(tauriBin, tauriBuildArgs(bundleName), true, bundleEnv, {
        logPath: appImageBundleLogPath(bundleName),
      });
      successfulBundles.push(bundleName);
    } catch (error) {
      if (!canContinueAfterBundleFailure(bundleName, orderedBundleNames, successfulBundles)) {
        if (process.platform === "linux" && bundleName === "appimage") {
          error.suppressStack = true;
          printAppImageFailureHelp(error);
        }
        throw error;
      }
      ignoredFailures.push({ bundleName, error });
      console.warn(`Linux ${bundleName} bundling failed, but completed installer bundles will still be collected.`);
      if (bundleName === "appimage") printAppImageFailureHelp(error);
      console.warn(`For an Ubuntu installer that avoids AppImage helper downloads, run npm run tauri:build:linux:deb${linuxScriptTargetSuffix()}.`);
    } finally {
      restoreAppImageFfmpegResources();
      if (appImageToolsMirror) await appImageToolsMirror.close();
    }
  }
  renameBundleFiles(successfulBundles);
  copyReleaseArtifacts(successfulBundles);
  if (ignoredFailures.length > 0) {
    // 保留成功产物并明确提示失败目标，避免用户因 AppImage 网络超时误以为 deb/rpm 也没有生成。
    // Successful artifacts are kept and failed targets are reported explicitly so users do not mistake an AppImage network timeout for missing deb/rpm outputs.
    console.warn("Some optional Linux bundles failed or were skipped after earlier installers had succeeded:");
    for (const failure of ignoredFailures) {
      console.warn(`  ${failure.bundleName}: ${failure.error.message}`);
    }
  }
} catch (error) {
  if (process.platform === "darwin" && bundle.split(",").map((item) => item.trim()).includes("dmg")) {
    const targetRoot = targetTriple ? join(projectRoot, "target", targetTriple, "release") : join(projectRoot, "target", "release");
    const bundleDir = join(targetRoot, "bundle");
    // DMG 生成依赖 macOS 的 hdiutil、挂载状态和本机权限；失败时保留 .app 构建结果，便于用户先验证程序本体。
    // DMG creation depends on macOS hdiutil, mount state, and local permissions; when it fails, the .app output is preserved for application testing.
    console.error("macOS DMG bundling failed after the application was compiled.");
    console.error(`Check the generated bundle directory: ${bundleDir}`);
    console.error("You can build a runnable .app without DMG by running npm run tauri:build:macos or npm run tauri:build:macos:arm64.");
    console.error("To debug DMG generation, run the generated bundle_dmg.sh manually with bash -x.");
  }
  if (error?.suppressStack) {
    // 已知的本地依赖或 AppImage 工具链问题不是 Node 脚本崩溃；隐藏栈追踪能让用户直接看到可执行修复建议。
    // Known local prerequisite or AppImage toolchain problems are not Node script crashes; hiding the stack trace keeps the actionable fix visible.
    if (error.message) console.error(error.message);
    process.exitCode = 1;
  } else {
    throw error;
  }
} finally {
  // 只在打包期间临时收窄 bundle.resources，仓库仍保留全平台 FFmpeg 资源，避免构建后留下配置改动。
  // bundle.resources is narrowed only during packaging; the repository still keeps all-platform FFmpeg resources without leaving config changes after builds.
  restoreTauriResources();
}
