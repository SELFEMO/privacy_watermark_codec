import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { delimiter, join, resolve } from "node:path";

function cargoFileName() {
  return process.platform === "win32" ? "cargo.exe" : "cargo";
}

function pathEnvKey(env) {
  // Windows 上 Node 可能暴露 Path 而不是 PATH；必须改实际存在的键，否则子进程仍看不到 cargo。
  // On Windows Node may expose Path rather than PATH; the existing key must be updated or child processes still cannot see cargo.
  return Object.keys(env).find((key) => key.toLowerCase() === "path")
    || (process.platform === "win32" ? "Path" : "PATH");
}

function sameDirectory(left, right) {
  const a = resolve(left);
  const b = resolve(right);
  return process.platform === "win32" ? a.toLowerCase() === b.toLowerCase() : a === b;
}

function toolRunnable(name, env) {
  const result = spawnSync(name, ["--version"], {
    env,
    encoding: "utf8",
    windowsHide: true,
    timeout: 20000,
  });
  return result.status === 0;
}

export function resolveCargoBinDirectory() {
  const home = process.env.USERPROFILE || process.env.HOME || homedir();
  const candidates = [];
  if (process.env.CARGO_HOME) candidates.push(join(process.env.CARGO_HOME, "bin"));
  if (process.env.USERPROFILE) candidates.push(join(process.env.USERPROFILE, ".cargo", "bin"));
  if (process.env.HOME) candidates.push(join(process.env.HOME, ".cargo", "bin"));
  candidates.push(join(home, ".cargo", "bin"));

  const cargoName = cargoFileName();
  const seen = new Set();
  for (const directory of candidates) {
    const normalized = resolve(directory);
    const key = process.platform === "win32" ? normalized.toLowerCase() : normalized;
    if (seen.has(key)) continue;
    seen.add(key);
    if (existsSync(join(normalized, cargoName))) return normalized;
  }
  return null;
}

function prependPath(env, directory) {
  const key = pathEnvKey(env);
  const current = env[key] || "";
  const parts = current.split(delimiter).filter(Boolean);
  if (parts.some((part) => sameDirectory(part, directory))) return;
  env[key] = current ? `${directory}${delimiter}${current}` : directory;
}

export function ensureRustToolchainOnPath(env = process.env) {
  if (toolRunnable("cargo", env) && toolRunnable("rustc", env)) {
    return { env, prepended: null };
  }

  const cargoBin = resolveCargoBinDirectory();
  if (!cargoBin) return { env, prepended: null };

  prependPath(env, cargoBin);
  return { env, prepended: cargoBin };
}

export function applyRustToolchainToProcessEnv() {
  const { prepended } = ensureRustToolchainOnPath(process.env);
  if (prepended) {
    console.log(`[dev] Prepended ${prepended} to PATH (cargo was missing from this process).`);
  }
  return prepended;
}

export function assertCargoRunnable() {
  const cargo = spawnSync("cargo", ["--version"], {
    env: process.env,
    encoding: "utf8",
    windowsHide: true,
    timeout: 20000,
  });
  if (cargo.status === 0) {
    return (cargo.stdout || "").trim();
  }

  const installed = resolveCargoBinDirectory();
  const detail = cargo.error?.message || (cargo.stderr || "").trim() || "cargo --version failed.";
  if (installed) {
    throw new Error(
      `cargo is installed at ${installed} but is not runnable from this process. `
      + `Prepend that directory to PATH and retry. ${detail}`,
    );
  }
  throw new Error(
    "cargo/rustc were not found. Install rustup from https://rustup.rs/ "
    + `and ensure ${join(homedir(), ".cargo", "bin")} is on PATH.`,
  );
}
