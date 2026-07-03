import { spawnSync } from "node:child_process";
import { lookup } from "node:dns/promises";
import { existsSync, readdirSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { appImageRawHelperSpecs, appImageReleaseToolSpecs, candidateDownloadUrls, githubReleaseUrl, probeDownloadUrl } from "./appimage-tools.mjs";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const targetRoot = join(projectRoot, "target");
const rawHelperUrls = appImageRawHelperSpecs().map((spec) => spec.url);
const args = process.argv.slice(2);

function optionValue(name) {
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : null;
}

function targetPlatformFromTriple(targetTriple) {
  if (!targetTriple) return null;
  if (!targetTriple.includes("linux")) {
    throw new Error(`AppImage diagnostics only supports Linux targets: ${targetTriple}`);
  }
  return targetTriple.startsWith("aarch64") ? "linux_arm64" : "linux_x64";
}

function targetPlatformFromArchAlias(alias) {
  if (!alias) return null;
  const normalized = alias.toLowerCase();
  if (["x64", "amd64", "x86_64"].includes(normalized)) return "linux_x64";
  if (["arm64", "aarch64"].includes(normalized)) return "linux_arm64";
  throw new Error(`Unsupported Linux AppImage architecture alias: ${alias}`);
}

function currentLinuxTargetPlatform() {
  return process.arch === "arm64" ? "linux_arm64" : "linux_x64";
}

// 诊断和构建脚本共用相同 target 推导，确保预检的是即将下载和运行的 helper 架构。
// Diagnostics reuse the same target inference as the build script so the preflight checks the helper architecture that will actually be downloaded and run.
const targetPlatform = targetPlatformFromTriple(optionValue("--target"))
  || targetPlatformFromArchAlias(optionValue("--arch"))
  || currentLinuxTargetPlatform();

function printStatus(label, ok, detail = "", severity = null) {
  const mark = severity || (ok ? "OK" : "WARN");
  console.log(`[${mark}] ${label}${detail ? ` - ${detail}` : ""}`);
}

function commandExists(command) {
  const result = spawnSync("sh", ["-lc", `command -v ${command}`], { encoding: "utf8" });
  return result.status === 0 ? result.stdout.trim() : null;
}

function runText(command, args) {
  const result = spawnSync(command, args, { cwd: projectRoot, encoding: "utf8" });
  return {
    ok: result.status === 0,
    output: `${result.stdout || ""}${result.stderr || ""}`.trim(),
  };
}

function listCachedTauriTools() {
  const tauriToolsDir = join(targetRoot, ".tauri");
  if (!existsSync(tauriToolsDir)) return [];

  const results = [];
  const stack = [tauriToolsDir];
  while (stack.length > 0) {
    const current = stack.pop();
    for (const entry of readdirSync(current)) {
      const path = join(current, entry);
      const stats = statSync(path);
      if (stats.isDirectory()) {
        stack.push(path);
      } else if (/linuxdeploy|AppRun/i.test(entry)) {
        results.push(path);
      }
    }
  }

  // 诊断命令只列出 Tauri helper 缓存，避免把 AppDir 中成百上千个 AppImage 中间文件刷满终端。
  // The diagnostic command only lists Tauri helper caches so AppDir intermediates do not flood the terminal.
  return results.sort().slice(0, 20);
}

async function probeCandidate(label, url) {
  const result = await probeDownloadUrl(url);
  const detail = result.ok ? `${result.status}, read ${result.bytes} bytes` : result.status;
  printStatus(label, result.ok, detail, result.ok ? null : "ERROR");
  return result.ok;
}

async function main() {
  console.log("Checking Linux AppImage build prerequisites...");

  if (process.platform !== "linux") {
    printStatus("host operating system", false, "AppImage can only be bundled on Linux hosts");
    return;
  }
  printStatus("host operating system", true, `${process.platform}/${process.arch}`);
  // 诊断目标架构必须和即将构建的 AppImage 一致，否则 arm64 构建会误报 x64 helper 的下载状态。
  // The diagnostic target architecture must match the AppImage build target, otherwise arm64 builds can incorrectly report x64 helper download status.
  printStatus("AppImage helper target platform", true, targetPlatform);

  const requiredCommands = ["file", "ldd", "readelf", "strip", "patchelf", "desktop-file-validate", "gtk-update-icon-cache", "update-desktop-database"];
  const missingBuildCommands = [];
  for (const command of requiredCommands) {
    const path = commandExists(command);
    const buildCritical = ["file", "ldd", "readelf", "strip", "patchelf"].includes(command);
    if (!path && buildCritical) missingBuildCommands.push(command);
    printStatus(`command ${command}`, Boolean(path), path || "not found in PATH", !path && buildCritical ? "ERROR" : null);
  }

  if (missingBuildCommands.length > 0) {
    // 缺少宿主机 ELF 工具时，继续下载 helper 也通常只会得到 linuxdeploy 泛化失败；直接给出安装命令更利于定位。
    // When host ELF tools are missing, downloading helpers usually ends in a generic linuxdeploy failure; an install hint is more actionable.
    console.log("Missing AppImage build commands:");
    console.log(`  ${missingBuildCommands.join(", ")}`);
    console.log("Ubuntu/Debian install hint:");
    console.log("  sudo apt install patchelf binutils file");
  }

  try {
    await lookup("github.com");
    printStatus("DNS github.com", true);
  } catch (error) {
    printStatus("DNS github.com", false, error.message, "ERROR");
  }

  console.log("Checking GitHub release helper download probes...");
  for (const spec of appImageReleaseToolSpecs(targetPlatform)) {
    const candidates = candidateDownloadUrls(spec);
    let ok = false;
    for (const [index, url] of candidates.entries()) {
      // HEAD 探测会误判 release-assets CDN；这里实际读取前 256 KiB，才能发现 Tauri 日志里的 global timeout 类问题。
      // HEAD probes can miss release-assets CDN problems; reading the first 256 KiB exposes the global-timeout class seen in Tauri logs.
      ok = await probeCandidate(`${spec.asset} download probe${index === 0 ? "" : ` candidate ${index + 1}`}`, url);
      if (ok) break;
    }
    if (!ok) {
      console.log(`All download candidates failed for ${spec.asset}.`);
      console.log(`Default URL: ${githubReleaseUrl(spec)}`);
    }
  }

  console.log("Checking raw GitHub helper scripts...");
  for (const url of rawHelperUrls) {
    const result = await probeDownloadUrl(url);
    const detail = result.ok ? `${result.status}, read ${result.bytes} bytes` : result.status;
    // raw.githubusercontent.com 偶发超时通常可以由 prefetch/cache 缓解；诊断中标为 WARN，避免误判为本机环境硬错误。
    // raw.githubusercontent.com may time out intermittently and can be mitigated by prefetch/cache; WARN avoids treating it as a hard local prerequisite error.
    printStatus(`raw helper ${url}`, result.ok, detail, result.ok ? null : "WARN");
  }

  const fuseAvailable = existsSync("/dev/fuse");
  // APPIMAGE_EXTRACT_AND_RUN 已用于构建脚本；没有 FUSE 不一定失败，但这里提示可帮助判断 AppImage 工具运行环境。
  // APPIMAGE_EXTRACT_AND_RUN is used by the build script; missing FUSE is not always fatal, but this hint helps identify the AppImage tool runtime environment.
  printStatus("/dev/fuse", fuseAvailable, fuseAvailable ? "present" : "missing; build script uses APPIMAGE_EXTRACT_AND_RUN as fallback");

  const tauri = process.platform === "win32" ? join(projectRoot, "node_modules", ".bin", "tauri.cmd") : join(projectRoot, "node_modules", ".bin", "tauri");
  if (existsSync(tauri)) {
    const result = runText(tauri, ["--version"]);
    printStatus("Tauri CLI", result.ok, result.output || "unable to read version");
  } else {
    printStatus("Tauri CLI", false, "node_modules is missing; run npm install first");
  }

  // AppStream 校验由 appimagetool 执行，警告也会导致失败；实际构建中会设置 LDAI_NO_APPSTREAM=1 跳过这个可选校验。
  // AppStream validation is run by appimagetool and warnings can fail the build; the real build sets LDAI_NO_APPSTREAM=1 to skip this optional check.
  printStatus("AppStream validation mode", true, "build sets LDAI_NO_APPSTREAM=1 for AppImage only");

  const cachedTools = listCachedTauriTools();
  if (cachedTools.length > 0) {
    console.log("Cached Tauri/AppImage helper files under target/.tauri:");
    for (const path of cachedTools) console.log(`  ${path}`);
  } else {
    // 刚清理 target 后没有缓存是正常状态；用 INFO 而不是 WARN，避免用户误以为诊断失败。
    // No cache is normal right after cleaning target; INFO avoids making users think the diagnostic failed.
    printStatus("cached Tauri/AppImage helper files", false, "none found yet; run the prefetch command to create the local cache", "INFO");
  }

  const scriptTargetSuffix = targetPlatform === "linux_arm64" ? ":arm64" : ":x64";
  console.log("Suggested normal AppImage build flow:");
  console.log(`  npm run tauri:build:linux:appimage:prefetch${scriptTargetSuffix}`);
  console.log(`  npm run tauri:build:linux:appimage${scriptTargetSuffix}`);
  console.log("Verbose retry when AppImage still fails:");
  console.log(`  PWC_APPIMAGE_VERBOSE=1 npm run tauri:build:linux:appimage${scriptTargetSuffix}`);
  console.log("Optional mirror variables for restricted GitHub networks:");
  console.log("  PWC_APPIMAGE_TOOLS_MIRROR_TEMPLATE=https://mirror.example.com/<owner>/<repo>/releases/download/<version>/<asset>");
  console.log("  TAURI_BUNDLER_TOOLS_GITHUB_MIRROR_TEMPLATE=https://mirror.example.com/<owner>/<repo>/releases/download/<version>/<asset>");
}

await main();
