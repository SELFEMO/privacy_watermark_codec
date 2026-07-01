import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const requiredFiles = [
  join(projectRoot, "Cargo.toml"),
  join(projectRoot, "src-tauri", "Cargo.toml"),
  join(projectRoot, "src-tauri", "tauri.conf.json"),
];

const missing = requiredFiles.filter((path) => !existsSync(path));
if (missing.length > 0) {
  // Tauri dev 会监听这些核心文件；提前检查可以把“watch No path was found”转换为明确的目录/文件缺失提示。
  // Tauri dev watches these core files; checking early turns a vague watcher error into a clear missing-file message.
  console.error("Tauri project files are missing:");
  for (const path of missing) console.error(`- ${path}`);
  console.error("Please run this command from the repository root, or restore the missing files before starting Tauri.");
  process.exit(1);
}
