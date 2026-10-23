import { existsSync, readdirSync, rmSync } from "node:fs";
import { dirname, isAbsolute, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptPath = fileURLToPath(import.meta.url);
const projectRoot = resolve(dirname(scriptPath), "..");

const CLEAN_GROUPS = Object.freeze({
  frontend: [
    "dist",
    ".vite",
    "node_modules/.vite",
    "node_modules/.cache",
    "coverage",
    ".eslintcache",
    ".stylelintcache",
    "tsconfig.tsbuildinfo",
  ],
  rust: [
    "target",
    "src-tauri/target",
    "crates/watermark-core/target",
  ],
  release: [
    "release",
    ".tauri",
    "src-tauri/.tauri",
  ],
});

const LOG_FILE_PATTERN = /^(?:npm|yarn|pnpm)-debug\.log(?:\.\d+)?$/i;

function normalizeScope(scope) {
  const normalized = (scope || "all").trim().toLowerCase();
  if (normalized === "all") return normalized;
  if (Object.hasOwn(CLEAN_GROUPS, normalized)) return normalized;
  throw new Error(`Unsupported clean scope: ${scope}. Use all, frontend, rust, or release.`);
}

function targetsForScope(scope) {
  if (scope === "all") {
    return [...new Set(Object.values(CLEAN_GROUPS).flat())];
  }
  return CLEAN_GROUPS[scope];
}

function assertSafeProjectChild(targetPath) {
  const relativePath = relative(projectRoot, targetPath);
  // 清理脚本拒绝删除项目根目录或根目录之外的路径，是为了让跨平台递归删除在配置被误改时仍保持可恢复性。
  // The cleaner refuses the project root and paths outside it so cross-platform recursive deletion remains recoverable even if a target is misconfigured.
  if (!relativePath || relativePath.startsWith("..") || isAbsolute(relativePath)) {
    throw new Error(`Refusing to remove unsafe path: ${targetPath}`);
  }
}

function removeProjectPath(relativePath, removed) {
  const targetPath = resolve(projectRoot, relativePath);
  assertSafeProjectChild(targetPath);
  if (!existsSync(targetPath)) return;

  rmSync(targetPath, {
    recursive: true,
    force: true,
    maxRetries: process.platform === "win32" ? 4 : 1,
    retryDelay: 120,
  });
  removed.push(relativePath.replaceAll("\\", "/"));
}

function removeRootDebugLogs(removed) {
  for (const entry of readdirSync(projectRoot, { withFileTypes: true })) {
    if (!entry.isFile() || !LOG_FILE_PATTERN.test(entry.name)) continue;
    removeProjectPath(entry.name, removed);
  }
}

export function cleanProject(scope = "all") {
  const normalizedScope = normalizeScope(scope);
  const removed = [];

  for (const target of targetsForScope(normalizedScope)) {
    removeProjectPath(target, removed);
  }
  if (normalizedScope === "all" || normalizedScope === "frontend") {
    removeRootDebugLogs(removed);
  }

  if (removed.length === 0) {
    const scopeLabel = normalizedScope === "all" ? "build" : `${normalizedScope} build`;
    console.log(`[clean] No ${scopeLabel} outputs or caches were present.`);
  } else {
    console.log(`[clean] Removed ${normalizedScope} build outputs and caches:`);
    for (const target of removed) console.log(`  ${target}`);
  }

  // node_modules 和内置 FFmpeg 不属于可再生构建输出；保留它们可避免每次构建都重新下载大体积依赖。
  // node_modules and bundled FFmpeg are not disposable build outputs; preserving them avoids downloading large dependencies before every build.
  console.log("[clean] Preserved node_modules dependencies and src-tauri/vendor/ffmpeg assets.");
  return removed;
}

const invokedPath = process.argv[1] ? resolve(process.argv[1]) : null;
if (invokedPath === resolve(scriptPath)) {
  try {
    cleanProject(process.argv[2] || "all");
  } catch (error) {
    console.error(`[clean] ${error.message}`);
    process.exitCode = 1;
  }
}
