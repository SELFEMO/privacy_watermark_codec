import { createServer } from "node:net";
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";

const DEFAULT_HOST = "127.0.0.1";
const DEFAULT_PORT = 1420;
const PORT_SEARCH_LIMIT = 128;
const GENERATED_CONFIG_DIRECTORY = ".tauri";
const GENERATED_CONFIG_NAME = "tauri.dev.generated.json";

function parsePort(rawValue, fallback) {
  if (rawValue === undefined || rawValue === null || rawValue === "") return fallback;
  const value = Number(rawValue);
  if (!Number.isInteger(value) || value < 1 || value > 65535) {
    throw new Error(`Invalid development server port: ${rawValue}`);
  }
  return value;
}

function normalizeHost(rawValue) {
  const value = String(rawValue || DEFAULT_HOST).trim();
  if (!value) throw new Error("Development server host cannot be empty.");
  return value;
}

function probePort(host, port) {
  return new Promise((resolveProbe, rejectProbe) => {
    const server = createServer();
    server.unref();
    server.once("error", (error) => {
      if (error?.code === "EADDRINUSE" || error?.code === "EACCES") {
        resolveProbe({ available: false, reason: error.code });
        return;
      }
      rejectProbe(error);
    });
    server.listen({ host, port, exclusive: true }, () => {
      const address = server.address();
      const selectedPort = typeof address === "object" && address ? address.port : port;
      server.close((error) => {
        if (error) {
          rejectProbe(error);
          return;
        }
        resolveProbe({ available: true, port: selectedPort });
      });
    });
  });
}

export async function selectDevelopmentEndpoint(options = {}) {
  const host = normalizeHost(options.host ?? process.env.PWC_DEV_HOST);
  const preferredPort = parsePort(options.preferredPort ?? process.env.PWC_DEV_PORT, DEFAULT_PORT);
  const attempts = parsePort(options.searchLimit, PORT_SEARCH_LIMIT);
  let firstFailureReason = null;

  for (let offset = 0; offset < attempts; offset += 1) {
    const candidate = preferredPort + offset;
    if (candidate > 65535) break;
    const probe = await probePort(host, candidate);
    if (probe.available) {
      return {
        host,
        port: probe.port,
        preferredPort,
        fallbackUsed: probe.port !== preferredPort,
        firstFailureReason,
      };
    }
    firstFailureReason ||= probe.reason;
  }

  // 连续端口可能整体落入 Windows 的系统保留区；让操作系统分配临时端口可避免把某个机器的保留区间写死到项目配置中。
  // A consecutive range can fall entirely inside a Windows excluded-port block; asking the OS for an ephemeral port avoids hard-coding one machine's reservation layout.
  const ephemeral = await probePort(host, 0);
  if (ephemeral.available) {
    return {
      host,
      port: ephemeral.port,
      preferredPort,
      fallbackUsed: true,
      firstFailureReason,
    };
  }

  throw new Error(
    `Unable to bind a local development server on ${host}. `
      + "Check Windows excluded TCP port ranges, security software, or local network policy.",
  );
}

export function writeDevelopmentTauriConfig(projectRoot, endpoint) {
  const configDirectory = resolve(projectRoot, GENERATED_CONFIG_DIRECTORY);
  const configPath = join(configDirectory, GENERATED_CONFIG_NAME);
  mkdirSync(configDirectory, { recursive: true });

  const devUrl = `http://${endpoint.host}:${endpoint.port}`;
  const config = {
    build: {
      devUrl,
    },
  };
  writeFileSync(configPath, `${JSON.stringify(config, null, 2)}\n`, "utf8");

  // 使用相对路径传给 Tauri，是为了避免 Windows 项目路径含空格时 shell 对绝对路径再次拆分。
  // Passing a relative path to Tauri prevents a Windows shell from splitting an absolute project path that contains spaces.
  return {
    configPath,
    configArgument: relative(projectRoot, configPath).replaceAll("\\", "/"),
    devUrl,
  };
}

export function removeDevelopmentTauriConfig(projectRoot) {
  const configPath = resolve(projectRoot, GENERATED_CONFIG_DIRECTORY, GENERATED_CONFIG_NAME);
  rmSync(configPath, { force: true });
}
