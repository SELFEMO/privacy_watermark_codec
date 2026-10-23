import { spawn } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { cleanProject } from "./clean.mjs";
import { applyRustToolchainToProcessEnv } from "./rust-env.mjs";
import {
  removeDevelopmentTauriConfig,
  selectDevelopmentEndpoint,
  writeDevelopmentTauriConfig,
} from "./dev-server-config.mjs";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const originalArgs = process.argv.slice(2);
const command = originalArgs.find((arg) => !arg.startsWith("-"));
const tauriBin = process.platform === "win32"
  ? join(projectRoot, "node_modules", ".bin", "tauri.cmd")
  : join(projectRoot, "node_modules", ".bin", "tauri");

function hasConfigArgument(args) {
  return args.some((arg, index) => arg === "--config" || arg === "-c" || arg.startsWith("--config=") || (
    index > 0 && (args[index - 1] === "--config" || args[index - 1] === "-c")
  ));
}

async function main() {
  // 直接 `npm run tauri -- dev` 会绕过 tauri-dev.mjs；这里同样补 PATH，避免 Tauri CLI 找不到 cargo。
  // Direct `npm run tauri -- dev` bypasses tauri-dev.mjs; restoring PATH here keeps the Tauri CLI able to find cargo.
  applyRustToolchainToProcessEnv();
  if ((command === "build" || command === "dev") && process.env.PWC_SKIP_CLEAN !== "1") {
    // 通用 Tauri 入口也执行全量清理，是为了覆盖用户直接运行 `npm run tauri -- build/dev` 而绕过项目专用包装脚本的情况。
    // The generic Tauri entry also performs a full clean so direct `npm run tauri -- build/dev` calls cannot bypass the project wrappers.
    cleanProject("all");
  }

  const args = [...originalArgs];
  const childEnv = { ...process.env };
  let generatedConfig = false;

  if (command === "dev" && !hasConfigArgument(args)) {
    const endpoint = await selectDevelopmentEndpoint();
    const runtimeConfig = writeDevelopmentTauriConfig(projectRoot, endpoint);
    args.push("--config", runtimeConfig.configArgument);
    childEnv.PWC_DEV_HOST = endpoint.host;
    childEnv.PWC_DEV_PORT = String(endpoint.port);
    childEnv.PWC_TAURI_DEV = "1";
    generatedConfig = true;

    if (endpoint.fallbackUsed) {
      const reason = endpoint.firstFailureReason === "EACCES"
        ? "permission denied or Windows excluded-port reservation"
        : "port already in use";
      console.log(
        `[dev] Preferred port ${endpoint.preferredPort} is unavailable (${reason}); using ${runtimeConfig.devUrl}.`,
      );
    } else {
      console.log(`[dev] Using ${runtimeConfig.devUrl}.`);
    }
  }

  const needsCommandShell = process.platform === "win32";
  const child = spawn(tauriBin, args, {
    cwd: projectRoot,
    env: childEnv,
    stdio: "inherit",
    windowsHide: true,
    shell: needsCommandShell,
  });

  const cleanup = () => {
    if (generatedConfig) removeDevelopmentTauriConfig(projectRoot);
  };

  child.on("error", (error) => {
    cleanup();
    console.error(`Failed to start Tauri CLI: ${error.message}`);
    process.exitCode = 1;
  });

  child.on("close", (code, signal) => {
    cleanup();
    if (signal) {
      console.error(`Tauri CLI stopped by signal ${signal}.`);
      process.exitCode = 1;
      return;
    }
    process.exitCode = code ?? 1;
  });
}

main().catch((error) => {
  removeDevelopmentTauriConfig(projectRoot);
  console.error(`[dev] ${error.message}`);
  process.exitCode = 1;
});
