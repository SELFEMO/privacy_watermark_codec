import { spawn } from "node:child_process";
import { existsSync, readdirSync, renameSync, unlinkSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const bundle = process.argv[2] || "nsis";
const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const configPath = join(projectRoot, "src-tauri", "tauri.conf.json");

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
  const bundleDir = join(projectRoot, "target", "release", "bundle", bundle);
  if (!existsSync(bundleDir)) return;

  for (const file of readdirSync(bundleDir)) {
    const versionPattern = new RegExp(`_${escapeRegExp(appVersion)}(?=_)`, "g");
    const renamed = file.replace(versionPattern, "");
    if (renamed === file) continue;

    const source = join(bundleDir, file);
    const target = join(bundleDir, renamed);
    if (existsSync(target)) unlinkSync(target);
    renameSync(source, target);
    console.log(`Renamed bundle: ${renamed}`);
  }
}

const nodePath = process.execPath;
const manifestScript = join(projectRoot, "scripts", "generate-ffmpeg-manifest.mjs");
const tauriBin = process.platform === "win32"
  ? join(projectRoot, "node_modules", ".bin", "tauri.cmd")
  : join(projectRoot, "node_modules", ".bin", "tauri");

await run(nodePath, [manifestScript, "--strict"]);
await run(tauriBin, ["build", "--bundles", bundle], true);
renameBundleFiles();
