import { spawn, spawnSync } from "node:child_process";
import { statSync } from "node:fs";
import { dirname, isAbsolute, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptPath = fileURLToPath(import.meta.url);
const projectRoot = resolve(dirname(scriptPath), "..");

function parseArguments(argv) {
  const [mode, ...remaining] = argv;
  if (mode !== "build" && mode !== "dev") {
    throw new Error("Use desktop:build:clean or desktop:dev:clean through npm run.");
  }

  let dryRun = false;
  let forwarding = false;
  const forwardedArgs = [];
  // 额外分隔符后的参数属于目标命令，避免工作流误吞目标命令自身的同名选项。
  // Arguments after an extra separator belong to the target command, preventing the workflow from consuming target options with the same name.
  for (const argument of remaining) {
    if (!forwarding && argument === "--") {
      forwarding = true;
    } else if (!forwarding && argument === "--dry-run") {
      dryRun = true;
    } else {
      forwardedArgs.push(argument);
    }
  }
  return { mode, dryRun, forwardedArgs };
}

function npmCliPath(env) {
  const path = env.npm_execpath;
  // 在任何清理发生前验证 npm 入口，避免直接启动脚本时清空缓存后才发现无法安装依赖。
  // Validate the npm entry before any cleanup so direct script invocation cannot discard caches before discovering it cannot install dependencies.
  if (!path || !isAbsolute(path)) {
    throw new Error("npm_execpath must point to the npm CLI. Start this workflow through npm run.");
  }
  try {
    if (statSync(path).isFile()) return path;
  } catch {
    // 路径不可读与路径缺失同样无法启动 npm，因此统一给出可操作的诊断。
    // An unreadable path prevents npm startup just like a missing path, so both receive the same actionable diagnostic.
  }
  throw new Error(`npm CLI is not a readable file: ${path}`);
}

function signalExitCode(signal) {
  return signal === "SIGINT" ? 130 : signal === "SIGTERM" ? 143 : 1;
}

function stopChildTree(child, signal) {
  if (!child?.pid) return;
  // npm 会继续启动 shell、Node 和 Cargo；只终止 npm 会留下构建，因此取消只针对本次拥有的进程树。
  // npm starts shells, Node, and Cargo; killing only npm can leave builds running, so cancellation targets the process tree owned by this workflow.
  if (process.platform === "win32") {
    const result = spawnSync("taskkill.exe", ["/PID", String(child.pid), "/T", "/F"], {
      stdio: "ignore",
      windowsHide: true,
      timeout: 5000,
    });
    if (result.status !== 0) child.kill(signal);
  } else {
    try {
      process.kill(-child.pid, signal);
    } catch (error) {
      if (error.code !== "ESRCH") throw error;
    }
  }
}

export async function runDesktopWorkflow(argv, env = process.env) {
  const { mode, dryRun, forwardedArgs } = parseArguments(argv);
  const npmCli = npmCliPath(env);
  const target = ["run", `desktop:${mode}`];
  if (forwardedArgs.length > 0) target.push("--", ...forwardedArgs);
  const steps = [
    ["run", "clean"],
    ["install", "--registry=https://registry.npmmirror.com"],
    target,
  ];

  if (dryRun) {
    console.log("[workflow] Dry run: no commands will be executed.");
    console.log(`[workflow] Working directory: ${projectRoot}`);
    for (const step of steps) console.log(`[workflow] npm ${step.map((arg) => JSON.stringify(arg)).join(" ")}`);
    return 0;
  }

  let activeChild = null;
  let interruptedCode = 0;
  const handlers = new Map();
  for (const signal of ["SIGINT", "SIGTERM"]) {
    const handler = () => {
      if (interruptedCode) return;
      interruptedCode = signalExitCode(signal);
      console.error(`[workflow] Interrupted by ${signal}; stopping the active step.`);
      stopChildTree(activeChild, signal);
    };
    handlers.set(signal, handler);
    process.on(signal, handler);
  }

  try {
    for (const step of steps) {
      if (interruptedCode) return interruptedCode;
      console.log(`[workflow] Starting npm ${step[0]} ${step[1]}`);
      const code = await new Promise((resolveStep) => {
        let startupError = null;
        // 直接用 Node 启动 npm 的 JavaScript 入口，使含空格的路径和参数不再经过 CMD 或 PowerShell 二次解析。
        // Launch npm's JavaScript entry directly with Node so paths containing spaces and arguments are not parsed again by CMD or PowerShell.
        const child = spawn(process.execPath, [npmCli, ...step], {
          cwd: projectRoot,
          env,
          shell: false,
          stdio: "inherit",
          windowsHide: true,
          detached: process.platform !== "win32",
        });
        activeChild = child;
        child.once("error", (error) => { startupError = error; });
        child.once("close", (exitCode, signal) => {
          activeChild = null;
          if (startupError) console.error(`[workflow] Failed to start npm: ${startupError.message}`);
          resolveStep(interruptedCode || (startupError ? 1 : exitCode ?? signalExitCode(signal)));
        });
      });
      // 清理或安装失败后继续会生成基于旧依赖或不完整输出的包，因此必须保留失败码并立即结束。
      // Continuing after cleanup or installation fails can package stale dependencies or incomplete outputs, so preserve the failure code and stop immediately.
      if (code !== 0) {
        console.error(`[workflow] Stopped with exit code ${code}.`);
        return code;
      }
    }
    return 0;
  } finally {
    for (const [signal, handler] of handlers) process.removeListener(signal, handler);
  }
}

const invokedPath = process.argv[1] ? resolve(process.argv[1]) : null;
if (invokedPath === resolve(scriptPath)) {
  runDesktopWorkflow(process.argv.slice(2)).then((code) => {
    process.exitCode = code;
  }).catch((error) => {
    console.error(`[workflow] ${error.message}`);
    process.exitCode = 1;
  });
}
