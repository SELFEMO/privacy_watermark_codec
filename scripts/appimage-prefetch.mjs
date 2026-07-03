import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { ensureAppImageRawHelperScripts, ensureAppImageReleaseTools } from "./appimage-tools.mjs";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);

function optionValue(name) {
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : null;
}

function targetPlatformFromTriple(targetTriple) {
  if (!targetTriple) return null;
  if (!targetTriple.includes("linux")) {
    throw new Error(`AppImage helper prefetch only supports Linux targets: ${targetTriple}`);
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

// 预取脚本按显式 target 优先推导平台，避免在 x64 宿主机上为 arm64 包缓存错架构 helper。
// The prefetch script prioritizes the explicit target when deriving the platform so an x64 host does not cache wrong-architecture helpers for arm64 packages.
const targetPlatform = targetPlatformFromTriple(optionValue("--target"))
  || targetPlatformFromArchAlias(optionValue("--arch"))
  || currentLinuxTargetPlatform();

// 交叉准备 AppImage 时 helper 必须按目标架构缓存，否则 arm64 构建会复用宿主 x64 的 linuxdeploy 资源。
// When preparing AppImage helpers for cross-target builds, the cache must follow the target architecture or arm64 builds can reuse host x64 linuxdeploy resources.
await ensureAppImageRawHelperScripts({ projectRoot });
await ensureAppImageReleaseTools({ projectRoot, targetPlatform });
console.log(`AppImage helper target platform: ${targetPlatform}`);
console.log("AppImage raw helper scripts are cached under target/.tauri.");
console.log("AppImage release helper tools are cached under target/.tauri/pwc-appimage-tools.");
