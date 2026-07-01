import { createHash } from "node:crypto";
import {
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  statSync,
  writeFileSync,
  cpSync,
  rmSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const strict = process.argv.includes("--strict");
const scriptDir = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(scriptDir, "..");
const ffmpegRoot = resolve(projectRoot, "src-tauri", "vendor", "ffmpeg");
const manifestPath = join(ffmpegRoot, "manifest.json");
const versionPath = join(ffmpegRoot, "VERSION.txt");

const platformDefinitions = {
  windows_x64: ["ffmpeg.exe", "ffprobe.exe", "ffplay.exe"],
  windows_arm64: ["ffmpeg.exe", "ffprobe.exe", "ffplay.exe"],
  macos_x64: ["ffmpeg", "ffprobe", "ffplay"],
  macos_arm64: ["ffmpeg", "ffprobe", "ffplay"],
  linux_x64: ["ffmpeg", "ffprobe", "ffplay"],
  linux_amd64: ["ffmpeg", "ffprobe", "ffplay"],
  linux_arm64: ["ffmpeg", "ffprobe", "ffplay"],
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

function currentPlatformKey() {
  if (process.platform === "win32") return process.arch === "arm64" ? "windows_arm64" : "windows_x64";
  if (process.platform === "darwin") return process.arch === "arm64" ? "macos_arm64" : "macos_x64";
  return process.arch === "arm64" ? "linux_arm64" : "linux_x64";
}

function firstExistingFfmpeg() {
  const preferred = currentPlatformKey();
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


function mirrorCurrentPlatformResources() {
  const currentPlatform = currentPlatformKey();
  for (const profile of ["debug", "release"]) {
    const destination = resolve(projectRoot, "target", profile, "vendor", "ffmpeg");
    try {
      rmSync(destination, { recursive: true, force: true });
      mkdirSync(destination, { recursive: true });
      for (const file of ["manifest.json", "LICENSE.txt", "README.md", "VERSION.txt"]) {
        const sourceFile = join(ffmpegRoot, file);
        if (existsSync(sourceFile)) cpSync(sourceFile, join(destination, file));
      }
      const sourcePlatform = join(ffmpegRoot, currentPlatform);
      if (existsSync(sourcePlatform)) cpSync(sourcePlatform, join(destination, currentPlatform), { recursive: true });
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

const currentPlatform = currentPlatformKey();
const current = manifest.platforms[currentPlatform];
const hasCurrentRuntime = Boolean(current?.ffmpeg?.sha256 && current?.ffprobe?.sha256);

// 发布包必须内置当前平台的 ffmpeg/ffprobe；开发阶段允许先生成清单，便于只调试图片功能。
// Release builds must include ffmpeg/ffprobe for the current platform; development can still generate a manifest for image-only work.
if (strict && !hasCurrentRuntime) {
  console.error(
    `Missing bundled FFmpeg runtime for ${currentPlatform}. Copy ffmpeg and ffprobe into ${join(ffmpegRoot, currentPlatform)}, then run npm run ffmpeg:manifest.`
  );
  process.exit(1);
}

if (populatedPlatformCount === 0 || !hasCurrentRuntime) {
  console.warn(
    `No complete FFmpeg runtime found for ${currentPlatform}. Video features will report a clear error until binaries are copied and manifest is regenerated.`
  );
}
