import { spawnSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const action = process.argv[2] || "ensure-dev";
const homeDir = process.env.HOME || "";
const appId = "com.privacywatermark.codec";
const legacyAppId = "privacy-watermark-codec";
const binaryName = "privacy-watermark-codec";
const appName = "图隐私水印编解码器";
const managedMarker = "X-PrivacyWatermarkCodec-Managed=dev";
const projectMarker = `X-PrivacyWatermarkCodec-Project=${escapeDesktopValue(projectRoot)}`;
const cacheDir = homeDir ? join(homeDir, ".cache", "privacy-watermark-codec") : "";
const statePath = cacheDir ? join(cacheDir, "linux-dev-desktop-entry-state.json") : "";
const applicationsDir = homeDir ? join(homeDir, ".local", "share", "applications") : "";
const iconsRoot = homeDir ? join(homeDir, ".local", "share", "icons", "hicolor") : "";
const iconSources = [
  { size: "32x32", source: join(projectRoot, "src-tauri", "icons", "32x32.png"), extension: ".png" },
  { size: "128x128", source: join(projectRoot, "src-tauri", "icons", "128x128.png"), extension: ".png" },
  { size: "256x256", source: join(projectRoot, "src-tauri", "icons", "128x128@2x.png"), extension: ".png" },
  { size: "scalable", source: join(projectRoot, "src-tauri", "icons", "icon.svg"), extension: ".svg" },
];

function main() {
  if (process.platform !== "linux") {
    return;
  }
  if (!homeDir) {
    console.warn("HOME is not set; Linux desktop entry management was skipped.");
    return;
  }

  if (action === "ensure-dev") {
    ensureDevDesktopEntry();
    return;
  }
  if (action === "cleanup-dev") {
    cleanupDevDesktopEntry({ removeKnownStaleEntries: true, quiet: false });
    return;
  }
  if (action === "refresh") {
    refreshDesktopCaches();
    return;
  }

  throw new Error(`Unsupported Linux desktop entry action: ${action}`);
}

function ensureDevDesktopEntry() {
  cleanupDevDesktopEntry({ removeKnownStaleEntries: false, quiet: true });

  mkdirSync(applicationsDir, { recursive: true });
  mkdirSync(cacheDir, { recursive: true });

  const desktopFiles = desktopFileTargets();
  const iconFiles = iconFileTargets();
  const state = createState([...desktopFiles.map((entry) => entry.path), ...iconFiles.map((entry) => entry.path)]);

  for (const entry of desktopFiles) {
    writeFileSync(entry.path, entry.content);
  }
  for (const entry of iconFiles) {
    mkdirSync(dirname(entry.path), { recursive: true });
    copyFileSync(entry.source, entry.path);
  }

  writeFileSync(statePath, `${JSON.stringify(state, null, 2)}\n`);
  refreshDesktopCaches();
  console.log(`Ensured temporary Linux desktop entry for ${appId}.`);
}

function cleanupDevDesktopEntry({ removeKnownStaleEntries, quiet }) {
  const state = readState();
  const protectedDesktopPaths = new Set();
  const protectedIconPaths = new Set();
  if (state) {
    restoreState(state);
    rmSync(statePath, { force: true });
    cleanupGnomeFavorites(new Set(state.favoriteIdsToRemove || []));
    for (const entry of state.entries || []) {
      if (!entry.existed) {
        continue;
      }
      if (String(entry.path).endsWith(".desktop")) {
        protectedDesktopPaths.add(entry.path);
      } else {
        protectedIconPaths.add(entry.path);
      }
    }
  }

  if (removeKnownStaleEntries) {
    removeKnownStaleDesktopFiles(protectedDesktopPaths);
    removeKnownLocalIcons(protectedIconPaths);
    const staleFavoriteIds = new Set([`${appId}.desktop`, `${legacyAppId}.desktop`, "Privacy Watermark Codec.desktop"]);
    for (const protectedPath of protectedDesktopPaths) {
      staleFavoriteIds.delete(protectedPath.split("/").pop());
    }
    cleanupGnomeFavorites(staleFavoriteIds);
  }

  refreshDesktopCaches();
  if (!quiet) {
    console.log(`Cleaned temporary Linux desktop entries for ${appId}.`);
  }
}

function desktopFileTargets() {
  return [
    {
      path: join(applicationsDir, `${appId}.desktop`),
      content: desktopEntryContent({ id: appId, startupWmClass: appId, noDisplay: false }),
    },
    {
      path: join(applicationsDir, `${legacyAppId}.desktop`),
      content: desktopEntryContent({ id: legacyAppId, startupWmClass: legacyAppId, noDisplay: true }),
    },
  ];
}

function desktopEntryContent({ id, startupWmClass, noDisplay }) {
  const executable = process.env.PWC_DEV_EXECUTABLE || join(projectRoot, "target", "debug", binaryName);
  const lines = [
    "[Desktop Entry]",
    "Type=Application",
    `Name=${appName}${noDisplay ? " Dev WM_CLASS Fallback" : ""}`,
    "Comment=Local invisible privacy watermark encoder and decoder",
    `Exec=${quoteDesktopExecPath(executable)}`,
    `Icon=${appId}`,
    "Terminal=false",
    "Categories=Utility;",
    "StartupNotify=true",
    `StartupWMClass=${startupWmClass}`,
    `X-GNOME-WMClass=${startupWmClass}`,
    managedMarker,
    projectMarker,
  ];
  if (noDisplay) {
    lines.push("NoDisplay=true");
  }
  return `${lines.join("\n")}\n`;
}

function iconFileTargets() {
  const targets = [];
  for (const icon of iconSources) {
    if (!existsSync(icon.source)) {
      continue;
    }
    for (const id of [appId, legacyAppId]) {
      targets.push({
        source: icon.source,
        path: join(iconsRoot, icon.size, "apps", `${id}${icon.extension}`),
      });
    }
  }
  return targets;
}

function createState(paths) {
  const entries = [];
  for (const path of paths) {
    const exists = existsSync(path);
    entries.push({
      path,
      existed: exists,
      content: exists ? readFileSync(path).toString("base64") : null,
    });
  }
  return {
    projectRoot,
    desktopIds: [`${appId}.desktop`, `${legacyAppId}.desktop`],
    favoriteIdsToRemove: paths
      .filter((path) => path.endsWith(".desktop") && !existsSync(path))
      .map((path) => path.split("/").pop()),
    entries,
  };
}

function restoreState(state) {
  for (const entry of state.entries || []) {
    if (entry.existed) {
      mkdirSync(dirname(entry.path), { recursive: true });
      writeFileSync(entry.path, Buffer.from(entry.content || "", "base64"));
    } else {
      rmSync(entry.path, { force: true });
    }
  }
}

function readState() {
  if (!statePath || !existsSync(statePath)) {
    return null;
  }
  try {
    return JSON.parse(readFileSync(statePath, "utf8"));
  } catch (error) {
    console.warn(`Could not read Linux desktop entry state: ${error.message}`);
    return null;
  }
}

function removeKnownStaleDesktopFiles(protectedDesktopPaths = new Set()) {
  if (!existsSync(applicationsDir)) {
    return;
  }
  for (const fileName of [`${appId}.desktop`, `${legacyAppId}.desktop`, "Privacy Watermark Codec.desktop"]) {
    const path = join(applicationsDir, fileName);
    if (protectedDesktopPaths.has(path) || !existsSync(path) || !isSafeToRemoveDesktopFile(path)) {
      continue;
    }
    rmSync(path, { force: true });
  }
}

function isSafeToRemoveDesktopFile(path) {
  let content = "";
  try {
    content = readFileSync(path, "utf8");
  } catch {
    return false;
  }
  return content.includes(managedMarker)
    || content.includes(projectMarker)
    || content.includes(projectRoot)
    || content.includes("PrivacyWatermarkCodecData")
    || content.includes("Local invisible privacy watermark encoder and decoder");
}

function removeKnownLocalIcons(protectedIconPaths = new Set()) {
  if (!existsSync(iconsRoot)) {
    return;
  }
  for (const size of ["32x32", "128x128", "256x256", "scalable"]) {
    for (const id of [appId, legacyAppId]) {
      for (const extension of [".png", ".svg"]) {
        const path = join(iconsRoot, size, "apps", `${id}${extension}`);
        if (!protectedIconPaths.has(path)) {
          rmSync(path, { force: true });
        }
      }
    }
  }
}

function refreshDesktopCaches() {
  // GNOME Shell 读取的是桌面入口数据库和图标主题缓存；改完文件后主动刷新，才能避免关闭开发态后留下旧图标引用。
  // GNOME Shell reads the desktop-entry database and icon-theme cache; refreshing them after file changes prevents stale dev icon references after the app closes.
  runIfAvailable("update-desktop-database", [applicationsDir]);
  runIfAvailable("gtk-update-icon-cache", ["-f", "-t", iconsRoot]);
  runIfAvailable("xdg-desktop-menu", ["forceupdate"]);
}

function cleanupGnomeFavorites(idsToRemove) {
  if (idsToRemove.size === 0) {
    return;
  }
  const current = spawnSync("gsettings", ["get", "org.gnome.shell", "favorite-apps"], { encoding: "utf8" });
  if (current.status !== 0 || !current.stdout.trim().startsWith("[")) {
    return;
  }

  const favorites = parseGSettingsStringArray(current.stdout.trim());
  const next = favorites.filter((item) => !idsToRemove.has(item));
  if (next.length === favorites.length) {
    return;
  }

  // 只移除本次开发脚本临时创建的 desktop id，避免用户把调试入口固定到 Dock 后关闭程序仍留下齿轮占位。
  // Only desktop ids created by this dev script are removed, so a pinned debug launcher does not remain as a gear placeholder after closing the app.
  spawnSync("gsettings", ["set", "org.gnome.shell", "favorite-apps", formatGSettingsStringArray(next)], { stdio: "ignore" });
}

function parseGSettingsStringArray(value) {
  const matches = value.match(/'([^']*)'/g) || [];
  return matches.map((item) => item.slice(1, -1));
}

function formatGSettingsStringArray(values) {
  return `[${values.map((value) => `'${value.replace(/'/g, "\\'")}'`).join(", ")}]`;
}

function runIfAvailable(command, args) {
  if (args.some((arg) => !arg)) {
    return;
  }
  const result = spawnSync(command, args, { stdio: "ignore" });
  if (result.error && result.error.code !== "ENOENT") {
    console.warn(`${command} failed: ${result.error.message}`);
  }
}

function quoteDesktopExecPath(path) {
  const escaped = String(path)
    .replace(/\\/g, "\\\\")
    .replace(/"/g, "\\\"")
    .replace(/\$/g, "\\$")
    .replace(/`/g, "\\`");
  return `"${escaped}"`;
}

function escapeDesktopValue(value) {
  return String(value).replace(/\n/g, " ").replace(/\r/g, " ");
}

main();
