import { spawn } from "node:child_process";
import { existsSync, readdirSync, renameSync, unlinkSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const configPath = join(projectRoot, "src-tauri", "tauri.conf.json");
const args = process.argv.slice(2);
const targetIndex = args.indexOf("--target");
const targetTriple = targetIndex >= 0 ? args[targetIndex + 1] : null;
const requestedBundle = args.find((arg, index) => index !== targetIndex && index !== targetIndex + 1 && !arg.startsWith("--"));

function defaultBundleForPlatform() {
  if (process.platform === "win32") return "nsis";
  if (process.platform === "darwin") return "app";
  if (process.platform === "linux") return "appimage,deb";
  return "app";
}

function targetPlatformFromTriple(targetTriple) {
  if (!targetTriple) {
    if (process.platform === "win32") return process.arch === "arm64" ? "windows_arm64" : "windows_x64";
    if (process.platform === "darwin") return process.arch === "arm64" ? "macos_arm64" : "macos_amd64";
    return process.arch === "arm64" ? "linux_arm64" : "linux_x64";
  }
  if (targetTriple.includes("apple-darwin")) return targetTriple.startsWith("aarch64") ? "macos_arm64" : "macos_amd64";
  if (targetTriple.includes("windows")) return targetTriple.startsWith("aarch64") ? "windows_arm64" : "windows_x64";
  if (targetTriple.includes("linux")) return targetTriple.startsWith("aarch64") ? "linux_arm64" : "linux_x64";
  return null;
}

const bundle = requestedBundle || defaultBundleForPlatform();
const targetPlatform = targetPlatformFromTriple(targetTriple);

function sanitize(text) {
  return text
    .replace(/\sv\d+(?:\.\d+)+(?:[-+._a-zA-Z0-9]*)?/g, "")
    .replace(/_\d+(?:\.\d+)+(?=_)/g, "")
    .replace(/\d+(?:\.\d+)+(?:[-+._a-zA-Z0-9]*)?/g, "");
}

function quoteForShell(value) {
  return `"${String(value).replace(/"/g, '\\"')}"`;
}

function run(command, args, filterOutput = false) {
  return new Promise((resolveRun, rejectRun) => {
    const useShell = process.platform === "win32";
    const finalCommand = useShell ? quoteForShell(command) : command;

    const child = spawn(finalCommand, args, {
      cwd: projectRoot,
      shell: useShell,
      windowsHide: true,
      env: { ...process.env, NO_COLOR: "1" },
      stdio: ["ignore", "pipe", "pipe"],
    });

    child.stdout.on("data", (chunk) => {
      const text = chunk.toString();
      process.stdout.write(filterOutput ? sanitize(text) : text);
    });
    child.stderr.on("data", (chunk) => {
      const text = chunk.toString();
      process.stderr.write(filterOutput ? sanitize(text) : text);
    });
    child.on("error", rejectRun);
    child.on("close", (code) => {
      if (code === 0) resolveRun();
      else rejectRun(new Error(`${command} ${args.join(" ")} failed`));
    });
  });
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function renameBundleFiles() {
  const config = JSON.parse(readFileSync(configPath, "utf8"));
  const appVersion = config.version;
  const bundleNames = bundle.split(",").map((item) => item.trim()).filter(Boolean);

  for (const bundleName of bundleNames) {
    const bundleDir = join(projectRoot, "target", targetTriple || "", "release", "bundle", bundleName);
    const fallbackBundleDir = join(projectRoot, "target", "release", "bundle", bundleName);
    const actualBundleDir = existsSync(bundleDir) ? bundleDir : fallbackBundleDir;
    if (!existsSync(actualBundleDir)) continue;

    for (const file of readdirSync(actualBundleDir)) {
      const versionPattern = new RegExp(`_${escapeRegExp(appVersion)}(?=_)`, "g");
      const renamed = file.replace(versionPattern, "");
      if (renamed === file) continue;

      const source = join(actualBundleDir, file);
      const target = join(actualBundleDir, renamed);
      if (existsSync(target)) unlinkSync(target);
      renameSync(source, target);
      console.log(`Renamed bundle: ${renamed}`);
    }
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
    case "macos_amd64":
      return ["macos_amd64"];
    case "macos_x64":
      return ["macos_x64"];
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

const tauriArgs = ["build", "--bundles", bundle];
if (targetTriple) {
  // 目标三元组只影响当前构建任务，用于在 macOS 上分别产出 arm64 与 amd64 包，避免把平台架构写入项目配置。
  // The target triple is kept per build so macOS arm64 and amd64 packages can be produced without hard-coding one architecture in project config.
  tauriArgs.push("--target", targetTriple);
}

const manifestArgs = [manifestScript, "--strict"];
if (targetPlatform) {
  // 发布构建按目标平台校验 FFmpeg，避免交叉构建时误用当前宿主架构。
  // Release builds validate FFmpeg for the target platform so cross-builds do not accidentally use the host architecture.
  manifestArgs.push("--target-platform", targetPlatform);
}

await run(nodePath, [preflightScript]);
await run(nodePath, manifestArgs);
const restoreTauriResources = patchTauriResourcesForTarget(targetPlatform);
try {
  await run(tauriBin, tauriArgs, true);
  renameBundleFiles();
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
  throw error;
} finally {
  // 只在打包期间临时收窄 bundle.resources，仓库仍保留全平台 FFmpeg 资源，避免构建后留下配置改动。
  // bundle.resources is narrowed only during packaging; the repository still keeps all-platform FFmpeg resources without leaving config changes after builds.
  restoreTauriResources();
}
