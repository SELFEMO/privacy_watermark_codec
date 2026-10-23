import { spawn } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { applyRustToolchainToProcessEnv } from "./rust-env.mjs";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const nodePath = process.execPath;
const npmBin = process.platform === "win32" ? "npm.cmd" : "npm";
const preflightScript = join(projectRoot, "scripts", "preflight-tauri.mjs");
const manifestScript = join(projectRoot, "scripts", "generate-ffmpeg-manifest.mjs");
const desktopEntryScript = join(projectRoot, "scripts", "linux-desktop-entry.mjs");
let activeChild = null;
let shuttingDown = false;

async function main() {
  installSignalHandlers();
  // 父进程先补 PATH，是因为 preflight 是子进程，它改环境变量传不回 npm/tauri。
  // The parent restores PATH first because preflight runs as a child and cannot push env changes back to npm/tauri.
  applyRustToolchainToProcessEnv();
  // 保留增量编译缓存才能让日常启动快速完成；需要排除缓存问题时仍可显式运行 clean。
  // Keeping incremental compilation caches makes everyday startup fast; explicit clean remains available for cache troubleshooting.
  await run(nodePath, [preflightScript]);
  await run(nodePath, [manifestScript]);

  if (process.platform === "linux") {
    await run(nodePath, [desktopEntryScript, "ensure-dev"]);
  }

  try {
    await run(npmBin, ["run", "tauri", "--", "dev"], {
      PWC_LINUX_DESKTOP_ENTRY_ACTIVE: process.platform === "linux" ? "1" : "0",
      PWC_SKIP_CLEAN: "1",
    });
  } finally {
    await cleanupLinuxDesktopEntry();
  }
}

function run(command, args, extraEnv = {}) {
  return new Promise((resolveRun, rejectRun) => {
    const invocation = normalizeSpawnInvocation(command, args);
    const child = spawn(invocation.command, invocation.args, {
      cwd: projectRoot,
      windowsHide: true,
      env: { ...process.env, ...extraEnv },
      stdio: "inherit",
    });
    activeChild = child;
    child.on("error", rejectRun);
    child.on("close", (code, signal) => {
      if (activeChild === child) {
        activeChild = null;
      }
      if (code === 0 || shuttingDown) {
        resolveRun();
        return;
      }
      rejectRun(new Error(`${command} ${args.join(" ")} failed with ${signal || code}`));
    });
  });
}

function normalizeSpawnInvocation(command, args) {
  const needsCommandShell = process.platform === "win32" && /\.(?:cmd|bat)$/i.test(command);
  if (!needsCommandShell) {
    return { command, args };
  }

  // Windows 不能像 Linux/macOS 那样稳定地把 .cmd/.bat 当作普通可执行文件直接 spawn；通过 ComSpec 转发可避免 npm.cmd 在部分 Node/Windows 组合下抛出 EINVAL。
  // Windows cannot reliably spawn .cmd/.bat files as ordinary executables like Linux/macOS; forwarding through ComSpec avoids EINVAL from npm.cmd on some Node/Windows combinations.
  return {
    command: process.env.ComSpec || "cmd.exe",
    args: ["/d", "/s", "/c", command, ...args],
  };
}

function installSignalHandlers() {
  for (const signal of ["SIGINT", "SIGTERM"]) {
    process.once(signal, async () => {
      shuttingDown = true;
      if (activeChild && !activeChild.killed) {
        activeChild.kill(signal);
      }
      await cleanupLinuxDesktopEntry();
      process.exit(signal === "SIGINT" ? 130 : 143);
    });
  }
}

async function cleanupLinuxDesktopEntry() {
  if (process.platform !== "linux") {
    return;
  }
  try {
    await run(nodePath, [desktopEntryScript, "cleanup-dev"]);
  } catch (error) {
    console.warn(`Linux desktop entry cleanup failed: ${error.message}`);
  }
}

main().catch(async (error) => {
  await cleanupLinuxDesktopEntry();
  console.error(error);
  process.exit(1);
});