import { createHash } from "node:crypto";
import {
  chmodSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  realpathSync,
  statSync,
  writeFileSync,
  cpSync,
  rmSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const strict = process.argv.includes("--strict");
const targetPlatformIndex = process.argv.indexOf("--target-platform");
const explicitTargetPlatform = targetPlatformIndex >= 0 ? process.argv[targetPlatformIndex + 1] : null;
const targetTripleIndex = process.argv.indexOf("--target");
const explicitTargetTriple = targetTripleIndex >= 0 ? process.argv[targetTripleIndex + 1] : null;
const scriptDir = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(scriptDir, "..");
const ffmpegRoot = resolve(projectRoot, "src-tauri", "vendor", "ffmpeg");
const manifestPath = join(ffmpegRoot, "manifest.json");
const versionPath = join(ffmpegRoot, "VERSION.txt");

const platformDefinitions = {
  windows_x64: ["ffmpeg.exe", "ffprobe.exe", "ffplay.exe"],
  windows_arm64: ["ffmpeg.exe", "ffprobe.exe", "ffplay.exe"],
  macos_x64: ["ffmpeg", "ffprobe", "ffplay"],
  macos_amd64: ["ffmpeg", "ffprobe", "ffplay"],
  macos_arm64: ["ffmpeg", "ffprobe", "ffplay"],
  linux_x64: ["ffmpeg", "ffprobe", "ffplay"],
  linux_amd64: ["ffmpeg", "ffprobe", "ffplay"],
  linux_arm64: ["ffmpeg", "ffprobe", "ffplay"],
};

const platformAliases = {
  macos_x64: ["macos_x64", "macos_amd64"],
  macos_amd64: ["macos_x64", "macos_amd64"],
  linux_x64: ["linux_x64", "linux_amd64"],
  linux_amd64: ["linux_x64", "linux_amd64"],
};

function sha256File(path) {
  const hash = createHash("sha256");
  hash.update(readFileSync(path));
  return hash.digest("hex");
}

function safeRun(binary, args) {
  const result = spawnSync(binary, args, { encoding: "utf8" });
  if (result.error || result.status !== 0) return "";
  return `${result.stdout || ""}${result.stderr || ""}`.trim();
}

function ensureExecutable(path) {
  if (process.platform === "win32" || !existsSync(path)) return;
  const currentMode = statSync(path).mode;
  const desiredMode = currentMode | 0o755;
  if ((currentMode & 0o755) === 0o755) return;
  // macOS/Linux 的 FFmpeg 可能被 Git LFS、压缩包或 cp 复制成只读可执行文件；统一补齐 owner 写入位，避免后续 build.rs 覆盖 target 目录时 Permission denied。
  // FFmpeg on macOS/Linux may become read-only after Git LFS, archive extraction, or cp copying; owner write permission is restored so build.rs can overwrite target copies without Permission denied.
  chmodSync(path, desiredMode);
}

function currentPlatformKey() {
  if (process.platform === "win32") return process.arch === "arm64" ? "windows_arm64" : "windows_x64";
  if (process.platform === "darwin") return process.arch === "arm64" ? "macos_arm64" : "macos_x64";
  return process.arch === "arm64" ? "linux_arm64" : "linux_x64";
}

function platformKeyFromTargetTriple(targetTriple) {
  if (!targetTriple) return null;
  if (targetTriple.includes("apple-darwin")) {
    return targetTriple.startsWith("aarch64") ? "macos_arm64" : "macos_x64";
  }
  if (targetTriple.includes("pc-windows") || targetTriple.includes("windows")) {
    return targetTriple.startsWith("aarch64") ? "windows_arm64" : "windows_x64";
  }
  if (targetTriple.includes("linux")) {
    return targetTriple.startsWith("aarch64") ? "linux_arm64" : "linux_x64";
  }
  return null;
}

function requiredPlatformKey() {
  return explicitTargetPlatform || platformKeyFromTargetTriple(explicitTargetTriple) || currentPlatformKey();
}

function isCurrentHostPlatform(platform) {
  return platform === currentPlatformKey();
}

function candidatePlatformKeys(platform) {
  return platformAliases[platform] || [platform];
}

function platformHasRequiredRuntime(platform) {
  const files = platformDefinitions[platform];
  if (!files) return false;
  const directory = join(ffmpegRoot, platform);
  return files.slice(0, 2).every((file) => {
    const path = join(directory, file);
    if (!existsSync(path) || !statSync(path).isFile()) return false;
    ensureExecutable(path);
    return true;
  });
}

function executableFromPath(name) {
  const result = process.platform === "win32"
    ? spawnSync("where.exe", [name], { encoding: "utf8" })
    : spawnSync("sh", ["-lc", `command -v ${name}`], { encoding: "utf8" });
  if (result.error || result.status !== 0) return null;
  const first = String(result.stdout || "").split(/\r?\n/).map((line) => line.trim()).find(Boolean);
  if (!first || !existsSync(first)) return null;
  return realpathSync(first);
}

function bootstrapCurrentPlatformRuntimeFromPath(platform) {
  if (!isCurrentHostPlatform(platform) || platformHasRequiredRuntime(platform)) return;
  const files = platformDefinitions[platform];
  if (!files) return;
  const ffmpeg = executableFromPath(files[0]);
  const ffprobe = executableFromPath(files[1]);
  if (!ffmpeg || !ffprobe) return;

  const directory = join(ffmpegRoot, platform);
  mkdirSync(directory, { recursive: true });
  for (const file of files) {
    const source = executableFromPath(file);
    if (!source) continue;
    const destination = join(directory, file);
    // 开发/打包前自动复用 PATH 中的当前平台 FFmpeg，是为了避免 Git LFS 未拉到大文件时整个 macOS 构建被阻断。
    // Reusing the current-platform FFmpeg from PATH before development/build avoids blocking macOS builds when Git LFS binaries were not pulled.
    cpSync(source, destination);
    ensureExecutable(destination);
  }
  console.log(`Copied current-platform FFmpeg runtime from PATH to ${directory}`);
}

function resolveRuntimePlatform(platform) {
  // x64 是项目面向用户的标准目录名，amd64 仅作为 Debian/Ubuntu 包架构或历史目录别名保留，避免旧资源目录失效。
  // x64 is the user-facing project directory name; amd64 is kept only as a Debian/Ubuntu package-architecture or legacy-directory alias so older runtime folders keep working.
  return candidatePlatformKeys(platform).find(platformHasRequiredRuntime) || platform;
}

function firstExistingFfmpeg() {
  const preferred = resolveRuntimePlatform(currentPlatformKey());
  const preferredFile = join(ffmpegRoot, preferred, platformDefinitions[preferred][0]);
  if (existsSync(preferredFile) && statSync(preferredFile).isFile()) return preferredFile;

  for (const [platform, files] of Object.entries(platformDefinitions)) {
    const candidate = join(ffmpegRoot, platform, files[0]);
    if (existsSync(candidate) && statSync(candidate).isFile()) return candidate;
  }
  return null;
}

function parseVersionDeclaration() {
  if (!existsSync(versionPath)) return { declaredVersion: null, utcCompileDate: null };
  const text = readFileSync(versionPath, "utf8");
  const version = text.match(/Version:\s*([^\r\n]+)/i)?.[1]?.trim()
    || text.match(/版本[:：]\s*([^\r\n]+)/)?.[1]?.trim()
    || null;
  const utcDate = text.match(/UTC compile date:\s*([^\r\n]+)/i)?.[1]?.trim()
    || text.match(/UTC 编译日期[:：]\s*([^\r\n]+)/)?.[1]?.trim()
    || null;
  return { declaredVersion: version, utcCompileDate: utcDate };
}

function parseVersionInfo(text) {
  const firstLine = text.split(/\r?\n/).find(Boolean) || "not-detected";
  const configLine = text
    .split(/\r?\n/)
    .find((line) => line.trim().startsWith("configuration:"));
  return {
    version: firstLine,
    configure: configLine ? configLine.replace(/^\s*configuration:\s*/, "") : "not-detected",
  };
}

function inferLicense(versionText, licenseText) {
  const combined = `${versionText}\n${licenseText}`.toLowerCase();
  if (combined.includes("--enable-nonfree")) return "nonfree-build-check-required";
  if (combined.includes("--enable-gpl") || combined.includes("gnu general public license")) {
    return "GPL-2.0-or-later-build-check-required";
  }
  if (combined.includes("lesser general public license") || combined.includes("lgpl")) {
    return "LGPL-2.1-or-later-build-check-required";
  }
  return "unknown-check-ffmpeg-license-output";
}

function ensureMetadataFiles() {
  mkdirSync(ffmpegRoot, { recursive: true });
  const licensePath = join(ffmpegRoot, "LICENSE.txt");
  const readmePath = join(ffmpegRoot, "README.md");
  if (!existsSync(licensePath)) {
    // Tauri 打包资源会严格校验清单中的文件存在性；自动生成占位许可证说明可以防止缺少大文件包附属文档时中断构建。
    // Tauri validates listed resources strictly, so this placeholder license note prevents builds from failing when auxiliary FFmpeg documents are missing.
    writeFileSync(
      licensePath,
      "FFmpeg runtime license notice. Keep the upstream FFmpeg license text, source URL, and build notes with released packages. Run ffmpeg -L to inspect the effective license of the bundled binary.\n",
      "utf8",
    );
  }
  if (!existsSync(versionPath)) {
    // VERSION.txt 是可选声明文件；没有人工声明时写入占位内容，让资源列表稳定存在而不伪造具体构建版本。
    // VERSION.txt is an optional declaration file; when no manual declaration exists, a placeholder keeps the resource list stable without inventing a concrete build version.
    writeFileSync(versionPath, "Declared FFmpeg build information is not provided. Inspect manifest.json and ffmpeg -version output for detected runtime details.\n", "utf8");
  }
  if (!existsSync(readmePath)) {
    writeFileSync(readmePath, "Place platform FFmpeg binaries in this directory and regenerate manifest.json before running or packaging the app.\n", "utf8");
  }
}

function mirrorCurrentPlatformResources() {
  const currentPlatform = currentPlatformKey();
  const runtimePlatform = resolveRuntimePlatform(currentPlatform);
  for (const profile of ["debug", "release"]) {
    const destination = resolve(projectRoot, "target", profile, "vendor", "ffmpeg");
    try {
      rmSync(destination, { recursive: true, force: true });
      mkdirSync(destination, { recursive: true });
      for (const file of ["manifest.json", "LICENSE.txt", "README.md", "VERSION.txt"]) {
        const sourceFile = join(ffmpegRoot, file);
        if (existsSync(sourceFile)) cpSync(sourceFile, join(destination, file));
      }
      const sourcePlatform = join(ffmpegRoot, runtimePlatform);
      const mirroredPlatform = join(destination, runtimePlatform);
      if (existsSync(sourcePlatform)) {
        cpSync(sourcePlatform, mirroredPlatform, { recursive: true });
        for (const file of platformDefinitions[runtimePlatform] || []) {
          ensureExecutable(join(mirroredPlatform, file));
        }
      }
      console.log(`Mirrored current-platform FFmpeg resources to ${destination}`);
    } catch (error) {
      console.warn(`Could not mirror FFmpeg resources to target/${profile}: ${error.message}`);
    }
  }
}

function buildPlatformEntry(platform, files) {
  const directory = join(ffmpegRoot, platform);
  const entry = {};
  for (const file of files) {
    const name = file.replace(/\.exe$/i, "");
    const path = join(directory, file);
    const exists = existsSync(path) && statSync(path).isFile();
    if (exists) ensureExecutable(path);
    entry[name] = {
      file,
      sha256: exists ? sha256File(path) : "",
    };
  }
  return entry;
}

if (!existsSync(ffmpegRoot)) {
  mkdirSync(ffmpegRoot, { recursive: true });
}
ensureMetadataFiles();

const buildPlatform = requiredPlatformKey();
bootstrapCurrentPlatformRuntimeFromPath(buildPlatform);

const platforms = {};
let populatedPlatformCount = 0;
for (const [platform, files] of Object.entries(platformDefinitions)) {
  const directory = join(ffmpegRoot, platform);
  if (!existsSync(directory)) {
    mkdirSync(directory, { recursive: true });
  }
  const visibleFiles = readdirSync(directory).filter((name) => !name.startsWith("."));
  if (visibleFiles.length > 0) populatedPlatformCount += 1;
  platforms[platform] = buildPlatformEntry(platform, files);
}

const sample = firstExistingFfmpeg();
const versionText = sample ? safeRun(sample, ["-version"]) : "";
const licenseText = sample ? safeRun(sample, ["-L"]) : "";
const parsed = parseVersionInfo(versionText);
const declared = parseVersionDeclaration();

const manifest = {
  version: declared.declaredVersion || parsed.version,
  source:
    "Bundled FFmpeg binaries stored under src-tauri/vendor/ffmpeg. Keep upstream download URL, source URL, license files, and build notes together with this project.",
  buildLicense: inferLicense(versionText, licenseText),
  buildConfigure: parsed.configure,
  generatedAt: new Date().toISOString(),
  platforms,
};

if (declared.utcCompileDate) {
  manifest.utcCompileDate = declared.utcCompileDate;
}

writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
console.log(`Wrote ${manifestPath}`);
mirrorCurrentPlatformResources();

for (const [platform, entry] of Object.entries(platforms)) {
  const found = Object.entries(entry)
    .filter(([, binary]) => binary.sha256)
    .map(([name]) => name)
    .join(", ");
  if (found) {
    console.log(`  ${platform}: ${found}`);
  }
}

const runtimePlatform = resolveRuntimePlatform(buildPlatform);
const current = manifest.platforms[runtimePlatform];
const hasCurrentRuntime = Boolean(current?.ffmpeg?.sha256 && current?.ffprobe?.sha256);

// 严格模式按目标平台而不是宿主平台校验，这样 Apple Silicon 上构建 Intel Mac 包时不会错误要求 arm64 运行时。
// Strict mode validates the target platform instead of the host platform so Intel Mac packages built on Apple Silicon do not wrongly require the arm64 runtime.
if (strict && !hasCurrentRuntime) {
  const candidates = candidatePlatformKeys(buildPlatform).map((platform) => join(ffmpegRoot, platform)).join(" or ");
  console.error(
    `Missing bundled FFmpeg runtime for ${buildPlatform}. Copy ffmpeg and ffprobe into ${candidates}, then run npm run ffmpeg:manifest.`
  );
  process.exit(1);
}

if (populatedPlatformCount === 0 || !hasCurrentRuntime) {
  console.warn(
    `No complete FFmpeg runtime found for ${buildPlatform}. Video features will report a clear error until binaries are copied and manifest is regenerated.`
  );
}
