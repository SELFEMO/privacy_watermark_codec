import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, isAbsolute, join, relative, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const source = join(dirname(fileURLToPath(import.meta.url)), "desktop-workflow.mjs");

function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), "pwc workflow 中文 "));
  const scripts = join(root, "scripts");
  mkdirSync(scripts);
  const workflow = join(scripts, "desktop-workflow.mjs");
  const npmCli = join(root, "npm 入口.cjs");
  const logPath = join(root, "calls.jsonl");
  const readyPath = join(root, "ready");
  const descendantPath = join(root, "descendant.pid");
  copyFileSync(source, workflow);
  // 用真正的 Node 子进程记录 npm 调用，才能同时验证 Windows 参数边界，又避免测试清理仓库或访问网络。
  // Record npm calls through real Node child processes to verify Windows argument boundaries without cleaning the repository or accessing the network.
  writeFileSync(npmCli, `
const { appendFileSync, writeFileSync } = require("node:fs");
const { spawn } = require("node:child_process");
const args = process.argv.slice(2);
appendFileSync(process.env.PWC_WORKFLOW_LOG, JSON.stringify({ args, cwd: process.cwd(), marker: process.env.PWC_WORKFLOW_MARKER }) + "\\n");
const step = args[0] === "install" ? "install" : args[1];
if (process.env.PWC_WORKFLOW_FAIL_STEP === step) process.exit(Number(process.env.PWC_WORKFLOW_FAIL_CODE));
if (process.env.PWC_WORKFLOW_HOLD_STEP === step) {
  const descendant = spawn(process.execPath, ["-e", "setInterval(() => {}, 1000)"], { stdio: "ignore", windowsHide: true });
  writeFileSync(process.env.PWC_WORKFLOW_DESCENDANT, String(descendant.pid));
  writeFileSync(process.env.PWC_WORKFLOW_READY, "ready");
  setInterval(() => {}, 1000);
}
`, "utf8");
  t.after(() => {
    const child = relative(resolve(tmpdir()), resolve(root));
    assert.ok(child && !child.startsWith("..") && !isAbsolute(child));
    rmSync(root, { recursive: true, force: true });
  });
  const env = {
    ...process.env,
    npm_execpath: npmCli,
    PWC_WORKFLOW_LOG: logPath,
    PWC_WORKFLOW_READY: readyPath,
    PWC_WORKFLOW_DESCENDANT: descendantPath,
    PWC_WORKFLOW_MARKER: "inherited environment",
  };
  function run(args, overrides = {}, entry = workflow) {
    return spawnSync(process.execPath, [entry, ...args], {
      cwd: tmpdir(),
      env: { ...env, ...overrides },
      encoding: "utf8",
      shell: false,
      windowsHide: true,
      timeout: 15000,
    });
  }
  function calls() {
    return existsSync(logPath) ? readFileSync(logPath, "utf8").trim().split("\n").map((line) => JSON.parse(line)) : [];
  }
  return { root, workflow, npmCli, logPath, readyPath, descendantPath, run, calls };
}

for (const mode of ["build", "dev"]) {
  test(`${mode} executes clean, mirror install, and the existing desktop entry in order`, (t) => {
    const f = fixture(t);
    const result = f.run([mode]);
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(f.calls().map((call) => call.args), [
      ["run", "clean"],
      ["install", "--registry=https://registry.npmmirror.com"],
      ["run", `desktop:${mode}`],
    ]);
    for (const call of f.calls()) {
      assert.equal(call.cwd.toLowerCase(), f.root.toLowerCase());
      assert.equal(call.marker, "inherited environment");
    }
  });
}

for (const [step, expectedCalls] of [["clean", 1], ["install", 2], ["desktop:build", 3]]) {
  test(`${step} failure preserves the exit code and stops subsequent steps`, (t) => {
    const f = fixture(t);
    const result = f.run(["build"], { PWC_WORKFLOW_FAIL_STEP: step, PWC_WORKFLOW_FAIL_CODE: "37" });
    assert.equal(result.status, 37, result.stderr);
    assert.equal(f.calls().length, expectedCalls);
  });
}

test("build arguments remain individual arguments without shell interpretation", (t) => {
  const f = fixture(t);
  const args = ["nsis,msi", "--arch", "x64", "--target", "目标 path & | ; % ! $(echo text)"];
  const result = f.run(["build", ...args]);
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(f.calls().at(-1).args, ["run", "desktop:build", "--", ...args]);
});

test("dry run shows all stages without creating files or starting npm", (t) => {
  const f = fixture(t);
  const before = readdirSync(f.root);
  const result = f.run(["build", "--dry-run", "nsis", "--arch", "arm64"]);
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /Dry run/);
  assert.match(result.stdout, /"clean"/);
  assert.match(result.stdout, /https:\/\/registry\.npmmirror\.com/);
  assert.match(result.stdout, /"desktop:build" "--" "nsis" "--arch" "arm64"/);
  assert.deepEqual(readdirSync(f.root), before);
  assert.deepEqual(f.calls(), []);
});

test("arguments after the separator are forwarded even when named dry-run", (t) => {
  const f = fixture(t);
  const result = f.run(["build", "--", "--dry-run"]);
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(f.calls().at(-1).args, ["run", "desktop:build", "--", "--dry-run"]);
});

test("invalid mode or npm CLI is rejected before any cleanup", (t) => {
  const f = fixture(t);
  const cases = [
    [[], {}],
    [["unknown"], {}],
    [["build"], { npm_execpath: "" }],
    [["build"], { npm_execpath: "relative-cli.cjs" }],
    [["build"], { npm_execpath: join(f.root, "missing.cjs") }],
    [["build"], { npm_execpath: f.root }],
  ];
  for (const [args, env] of cases) {
    const result = f.run(args, env);
    assert.equal(result.status, 1, result.stderr);
    assert.deepEqual(f.calls(), []);
  }
});

test("a child startup error returns nonzero and never starts a later step", (t) => {
  const f = fixture(t);
  const harness = join(f.root, "startup-error.mjs");
  // 隔离父进程使用不存在的 Node 路径，以真实的启动错误验证停止行为，而不依赖操作系统允许移动正在使用的目录。
  // Give the isolated parent a missing Node path to test a real startup failure without relying on the OS allowing an in-use directory to move.
  writeFileSync(harness, `
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { runDesktopWorkflow } from "./scripts/desktop-workflow.mjs";
const root = dirname(fileURLToPath(import.meta.url));
process.execPath = join(root, "missing-node.exe");
process.exitCode = await runDesktopWorkflow(["build"]);
`, "utf8");
  const result = f.run([], {}, harness);
  assert.equal(result.status, 1, result.stderr);
  assert.match(result.stderr, /Failed to start npm/);
  assert.deepEqual(f.calls(), []);
});

for (const [signal, code] of [["SIGINT", 130], ["SIGTERM", 143]]) {
  test(`${signal} stops the owned process tree and never starts installation`, async (t) => {
    const f = fixture(t);
    const harness = join(f.root, "interrupt.mjs");
    // Windows 的 kill 不是控制台信号；在独立父进程中触发同一处理器，实际验证后代清理与退出状态。
    // Windows kill does not deliver a console signal; trigger the same handler in an isolated parent to verify descendant cleanup and exit status.
    writeFileSync(harness, `
import { existsSync } from "node:fs";
import { runDesktopWorkflow } from "./scripts/desktop-workflow.mjs";
const poll = setInterval(() => {
  if (existsSync(process.env.PWC_WORKFLOW_READY)) {
    clearInterval(poll);
    process.emit(${JSON.stringify(signal)});
  }
}, 20);
try { process.exitCode = await runDesktopWorkflow(["build"]); }
finally { clearInterval(poll); }
`, "utf8");
    const result = f.run([], { PWC_WORKFLOW_HOLD_STEP: "clean" }, harness);
    const pid = existsSync(f.descendantPath) ? Number(readFileSync(f.descendantPath, "utf8")) : null;
    // 即使断言失败也清理测试子进程，避免测试故障留下常驻 Node。
    // Clean up test children even when an assertion fails so a test failure cannot leave a persistent Node process.
    t.after(() => { if (pid) { try { process.kill(pid); } catch {} } });
    assert.equal(result.status, code, result.stderr);
    assert.equal(f.calls().length, 1);
    assert.ok(pid);
    let running = true;
    for (let attempt = 0; attempt < 30 && running; attempt += 1) {
      try { process.kill(pid, 0); } catch (error) {
        if (error.code === "ESRCH") running = false;
        else throw error;
      }
      if (running) await new Promise((resolveWait) => setTimeout(resolveWait, 20));
    }
    assert.equal(running, false, "The descendant process must stop with the workflow.");
  });
}
