import { spawnSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const appId = "com.privacywatermark.codec";
const legacyWindowClass = "privacy-watermark-codec";
const appName = "图隐私水印编解码器";
const appComment = "Local invisible privacy watermark encoder and decoder";

function writeIfChanged(path, content) {
  const existing = existsSync(path) ? readFileSync(path, "utf8") : null;
  if (existing === content) return;
  writeFileSync(path, content, "utf8");
}


function refreshDesktopCaches(home, applicationsDir) {
  // 刷新缓存不是功能依赖，但能减少 GNOME Shell 沿用旧齿轮图标的时间窗口。
  // Cache refresh is not a functional dependency, but it reduces the window where GNOME Shell keeps using the old gear icon.
  spawnSync("gtk-update-icon-cache", ["-q", join(home, ".local", "share", "icons", "hicolor")], { stdio: "ignore" });
  spawnSync("update-desktop-database", [applicationsDir], { stdio: "ignore" });
}

function quoteDesktopExecPath(path) {
  return `"${path.replace(/\\/g, "\\\\").replace(/"/g, "\\\"").replace(/\$/g, "\\$").replace(/`/g, "\\`")}"`;
}

function desktopEntry({ execPath, startupClass, noDisplay }) {
  return [
    "[Desktop Entry]",
    "Type=Application",
    `Name=${appName}`,
    `Comment=${appComment}`,
    `Exec=${quoteDesktopExecPath(execPath)}`,
    `Icon=${appId}`,
    "Terminal=false",
    "Categories=Utility;",
    "StartupNotify=true",
    `StartupWMClass=${startupClass}`,
    `X-GNOME-WMClass=${startupClass}`,
    noDisplay ? "NoDisplay=true" : null,
    "",
  ].filter((line) => line !== null).join("\n");
}

if (process.platform === "linux") {
  const home = process.env.HOME;
  if (home) {
    const applicationsDir = join(home, ".local", "share", "applications");
    const iconsDir = join(home, ".local", "share", "icons", "hicolor", "128x128", "apps");
    mkdirSync(applicationsDir, { recursive: true });
    mkdirSync(iconsDir, { recursive: true });

    const themedIcons = [
      [join(projectRoot, "src-tauri", "icons", "32x32.png"), join(home, ".local", "share", "icons", "hicolor", "32x32", "apps", `${appId}.png`)],
      [join(projectRoot, "src-tauri", "icons", "128x128.png"), join(home, ".local", "share", "icons", "hicolor", "128x128", "apps", `${appId}.png`)],
      [join(projectRoot, "src-tauri", "icons", "128x128@2x.png"), join(home, ".local", "share", "icons", "hicolor", "256x256", "apps", `${appId}.png`)],
      [join(projectRoot, "src-tauri", "icons", "icon.svg"), join(home, ".local", "share", "icons", "hicolor", "scalable", "apps", `${appId}.svg`)],
    ];
    for (const [iconSource, iconTarget] of themedIcons) {
      if (existsSync(iconSource)) {
        mkdirSync(dirname(iconTarget), { recursive: true });
        // 开发命令在 Tauri/GTK 初始化前先写入多尺寸主题图标，可避免 Dock 在首次看到窗口时只能回退到齿轮图标。
        // The dev command writes multiple theme icon sizes before Tauri/GTK initializes so the Dock does not fall back to a gear when it first sees the window.
        cpSync(iconSource, iconTarget);
      }
    }

    const execPath = join(projectRoot, "target", "debug", "privacy-watermark-codec");
    writeIfChanged(
      join(applicationsDir, `${appId}.desktop`),
      desktopEntry({ execPath, startupClass: appId, noDisplay: false }),
    );
    writeIfChanged(
      join(applicationsDir, `${legacyWindowClass}.desktop`),
      desktopEntry({ execPath, startupClass: legacyWindowClass, noDisplay: true }),
    );

    refreshDesktopCaches(home, applicationsDir);
    console.log(`Ensured Linux desktop entry for ${appId}`);
  }
}
