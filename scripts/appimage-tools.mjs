import { createServer } from "node:http";
import { request as httpRequest } from "node:http";
import { request as httpsRequest } from "node:https";
import { createWriteStream, existsSync, mkdirSync, renameSync, rmSync, statSync, chmodSync, createReadStream } from "node:fs";
import { dirname, join } from "node:path";
import { pipeline } from "node:stream/promises";

const DEFAULT_DOWNLOAD_TIMEOUT_MS = 300000;
const DEFAULT_PROBE_TIMEOUT_MS = 45000;
const PROBE_BYTES = 256 * 1024;

function normalizedTimeout(value, fallback) {
  const parsed = Number.parseInt(String(value || ""), 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

export function appImageToolArch(targetPlatform) {
  return targetPlatform && targetPlatform.endsWith("arm64") ? "aarch64" : "x86_64";
}

export function appImageReleaseToolSpecs(targetPlatform) {
  const arch = appImageToolArch(targetPlatform);
  return [
    {
      owner: "tauri-apps",
      repo: "binary-releases",
      version: "apprun-old",
      asset: `AppRun-${arch}`,
      minimumBytes: 1024,
    },
    {
      owner: "tauri-apps",
      repo: "binary-releases",
      version: "linuxdeploy",
      asset: `linuxdeploy-${arch}.AppImage`,
      minimumBytes: 1024 * 1024,
    },
    {
      owner: "linuxdeploy",
      repo: "linuxdeploy-plugin-appimage",
      version: "continuous",
      asset: `linuxdeploy-plugin-appimage-${arch}.AppImage`,
      minimumBytes: 1024 * 1024,
    },
  ];
}

export function appImageRawHelperSpecs() {
  return [
    {
      url: "https://raw.githubusercontent.com/tauri-apps/linuxdeploy-plugin-gtk/master/linuxdeploy-plugin-gtk.sh",
      fileName: "linuxdeploy-plugin-gtk.sh",
      minimumBytes: 1024,
    },
    {
      url: "https://raw.githubusercontent.com/tauri-apps/linuxdeploy-plugin-gstreamer/master/linuxdeploy-plugin-gstreamer.sh",
      fileName: "linuxdeploy-plugin-gstreamer.sh",
      minimumBytes: 1024,
    },
  ];
}

export function githubReleaseUrl(spec) {
  return `https://github.com/${spec.owner}/${spec.repo}/releases/download/${spec.version}/${spec.asset}`;
}

function renderMirrorTemplate(template, spec) {
  return template
    .replaceAll("<owner>", spec.owner)
    .replaceAll("<repo>", spec.repo)
    .replaceAll("<version>", spec.version)
    .replaceAll("<asset>", spec.asset);
}

function normalizeMirrorBase(base) {
  return base.replace(/\/+$/, "");
}

export function candidateDownloadUrls(spec, env = process.env) {
  const urls = [];
  const explicitTemplate = env.PWC_APPIMAGE_TOOLS_MIRROR_TEMPLATE || env.TAURI_BUNDLER_TOOLS_GITHUB_MIRROR_TEMPLATE;
  if (explicitTemplate) urls.push(renderMirrorTemplate(explicitTemplate, spec));

  const mirrorBase = env.PWC_APPIMAGE_TOOLS_GITHUB_MIRROR || env.TAURI_BUNDLER_TOOLS_GITHUB_MIRROR;
  if (mirrorBase) {
    urls.push(`${normalizeMirrorBase(mirrorBase)}/${spec.owner}/${spec.repo}/releases/download/${spec.version}/${spec.asset}`);
  }

  urls.push(githubReleaseUrl(spec));
  return [...new Set(urls)];
}

function requestForUrl(url) {
  return url.startsWith("http://") ? httpRequest : httpsRequest;
}

function pipeRequestToFile(url, destination, timeoutMs, redirectsLeft) {
  return new Promise((resolveDownload, rejectDownload) => {
    const requestFn = requestForUrl(url);
    const request = requestFn(url, {
      headers: {
        "User-Agent": "privacy-watermark-codec-appimage-helper",
      },
      timeout: timeoutMs,
    }, async (response) => {
      const status = response.statusCode || 0;
      if (status >= 300 && status < 400 && response.headers.location && redirectsLeft > 0) {
        response.resume();
        const nextUrl = new URL(response.headers.location, url).toString();
        try {
          await pipeRequestToFile(nextUrl, destination, timeoutMs, redirectsLeft - 1);
          resolveDownload();
        } catch (error) {
          rejectDownload(error);
        }
        return;
      }

      if (status < 200 || status >= 300) {
        response.resume();
        rejectDownload(new Error(`${url} returned HTTP ${status}`));
        return;
      }

      try {
        await pipeline(response, createWriteStream(destination));
        resolveDownload();
      } catch (error) {
        rejectDownload(error);
      }
    });

    request.on("timeout", () => {
      request.destroy(new Error(`${url} timed out after ${timeoutMs}ms`));
    });
    request.on("error", rejectDownload);
    request.end();
  });
}

async function downloadOne(spec, destination, timeoutMs, log = console.log) {
  mkdirSync(dirname(destination), { recursive: true });
  const temp = `${destination}.download`;
  let lastError = null;
  for (const url of candidateDownloadUrls(spec)) {
    rmSync(temp, { force: true });
    try {
      log(`Downloading AppImage helper ${spec.asset} from ${url}`);
      await pipeRequestToFile(url, temp, timeoutMs, 8);
      const size = statSync(temp).size;
      if (size < spec.minimumBytes) {
        throw new Error(`${url} downloaded only ${size} bytes`);
      }
      renameSync(temp, destination);
      chmodSync(destination, 0o755);
      log(`Cached AppImage helper ${spec.asset} (${size} bytes)`);
      return;
    } catch (error) {
      lastError = error;
      rmSync(temp, { force: true });
      log(`AppImage helper download failed for ${spec.asset}: ${error.message}`);
    }
  }
  throw lastError || new Error(`failed to download ${spec.asset}`);
}


async function downloadRawHelperScript(spec, destination, timeoutMs, log = console.log) {
  mkdirSync(dirname(destination), { recursive: true });
  const temp = `${destination}.download`;
  rmSync(temp, { force: true });
  try {
    log(`Downloading AppImage raw helper ${spec.fileName} from ${spec.url}`);
    await pipeRequestToFile(spec.url, temp, timeoutMs, 8);
    const size = statSync(temp).size;
    if (size < spec.minimumBytes) {
      throw new Error(`${spec.url} downloaded only ${size} bytes`);
    }
    renameSync(temp, destination);
    chmodSync(destination, 0o755);
    log(`Cached AppImage raw helper ${spec.fileName} (${size} bytes)`);
  } catch (error) {
    rmSync(temp, { force: true });
    throw error;
  }
}

export async function ensureAppImageRawHelperScripts({ projectRoot, log = console.log } = {}) {
  if (!projectRoot) throw new Error("projectRoot is required");
  const timeoutMs = normalizedTimeout(process.env.PWC_APPIMAGE_DOWNLOAD_TIMEOUT_MS, DEFAULT_DOWNLOAD_TIMEOUT_MS);
  const cacheDir = join(projectRoot, "target", ".tauri");
  mkdirSync(cacheDir, { recursive: true });

  const results = [];
  for (const spec of appImageRawHelperSpecs()) {
    const filePath = join(cacheDir, spec.fileName);
    if (existsSync(filePath) && statSync(filePath).size >= spec.minimumBytes) {
      chmodSync(filePath, 0o755);
      log(`Reusing cached AppImage raw helper ${spec.fileName}`);
    } else {
      // Tauri 也会下载 raw.githubusercontent.com 上的插件脚本；提前写入 target/.tauri 可减少 raw GitHub 网络抖动导致的 AppImage 失败。
      // Tauri also downloads plugin scripts from raw.githubusercontent.com; pre-caching them under target/.tauri reduces AppImage failures caused by raw GitHub network jitter.
      await downloadRawHelperScript(spec, filePath, timeoutMs, log);
    }
    results.push({ spec, filePath });
  }
  return results;
}

export async function ensureAppImageReleaseTools({ projectRoot, targetPlatform, log = console.log } = {}) {
  if (!projectRoot) throw new Error("projectRoot is required");
  const timeoutMs = normalizedTimeout(process.env.PWC_APPIMAGE_DOWNLOAD_TIMEOUT_MS, DEFAULT_DOWNLOAD_TIMEOUT_MS);
  const arch = appImageToolArch(targetPlatform);
  const cacheDir = join(projectRoot, "target", ".tauri", "pwc-appimage-tools", arch);
  mkdirSync(cacheDir, { recursive: true });

  const results = [];
  for (const spec of appImageReleaseToolSpecs(targetPlatform)) {
    const filePath = join(cacheDir, spec.owner, spec.repo, spec.version, spec.asset);
    if (existsSync(filePath) && statSync(filePath).size >= spec.minimumBytes) {
      chmodSync(filePath, 0o755);
      log(`Reusing cached AppImage helper ${spec.asset}`);
    } else {
      // Tauri 自带下载器在部分网络下会被 release-assets CDN 的全局超时打断；这里先用更长超时缓存 helper，再让 Tauri 从本地读取。
      // Tauri's built-in downloader can hit a global timeout on the release-assets CDN; caching helpers with a longer timeout lets Tauri read them locally.
      await downloadOne(spec, filePath, timeoutMs, log);
    }
    results.push({ spec, filePath });
  }
  return results;
}

function serverPathForSpec(spec) {
  return `/${spec.owner}/${spec.repo}/releases/download/${spec.version}/${spec.asset}`;
}

export async function startAppImageToolsMirror({ projectRoot, targetPlatform, log = console.log } = {}) {
  if (process.env.PWC_APPIMAGE_USE_TAURI_DOWNLOADER === "1") return null;
  await ensureAppImageRawHelperScripts({ projectRoot, log });
  const tools = await ensureAppImageReleaseTools({ projectRoot, targetPlatform, log });
  const byPath = new Map(tools.map((tool) => [serverPathForSpec(tool.spec), tool.filePath]));

  const server = createServer((request, response) => {
    const url = new URL(request.url || "/", "http://127.0.0.1");
    const filePath = byPath.get(decodeURIComponent(url.pathname));
    if (!filePath) {
      response.writeHead(404, { "Content-Type": "text/plain; charset=utf-8" });
      response.end("not found");
      return;
    }
    const size = statSync(filePath).size;
    response.writeHead(200, {
      "Content-Type": "application/octet-stream",
      "Content-Length": String(size),
    });
    createReadStream(filePath).pipe(response);
  });

  await new Promise((resolveListen, rejectListen) => {
    server.on("error", rejectListen);
    server.listen(0, "127.0.0.1", resolveListen);
  });

  const address = server.address();
  const port = typeof address === "object" && address ? address.port : null;
  if (!port) {
    await new Promise((resolveClose) => server.close(resolveClose));
    throw new Error("failed to allocate local AppImage helper mirror port");
  }

  const mirrorTemplate = `http://127.0.0.1:${port}/<owner>/<repo>/releases/download/<version>/<asset>`;
  log(`Serving cached AppImage helpers through local Tauri mirror: ${mirrorTemplate}`);
  return {
    env: {
      TAURI_BUNDLER_TOOLS_GITHUB_MIRROR_TEMPLATE: mirrorTemplate,
    },
    close: () => new Promise((resolveClose) => server.close(resolveClose)),
  };
}

export function probeDownloadUrl(url, { timeoutMs = DEFAULT_PROBE_TIMEOUT_MS, bytes = PROBE_BYTES } = {}) {
  return new Promise((resolveProbe) => {
    let settled = false;
    const finish = (result) => {
      if (settled) return;
      settled = true;
      resolveProbe(result);
    };
    const requestFn = requestForUrl(url);
    const request = requestFn(url, {
      headers: {
        "Range": `bytes=0-${Math.max(0, bytes - 1)}`,
        "User-Agent": "privacy-watermark-codec-appimage-probe",
      },
      timeout: timeoutMs,
    }, (response) => {
      const status = response.statusCode || 0;
      if (status >= 300 && status < 400 && response.headers.location) {
        response.resume();
        probeDownloadUrl(new URL(response.headers.location, url).toString(), { timeoutMs, bytes }).then(finish);
        return;
      }
      if (status < 200 || status >= 300) {
        response.resume();
        finish({ ok: false, status: `HTTP ${status}`, bytes: 0 });
        return;
      }

      let received = 0;
      response.on("data", (chunk) => {
        received += chunk.length;
        if (received >= bytes) {
          finish({ ok: true, status: status === 206 ? "206 partial" : "partial", bytes: received });
          response.destroy();
        }
      });
      response.on("end", () => finish({ ok: received > 0, status: status === 206 ? "206 partial" : String(status), bytes: received }));
      response.on("error", (error) => {
        if (received > 0) finish({ ok: true, status: "partial", bytes: received });
        else finish({ ok: false, status: error.message, bytes: received });
      });
    });

    request.on("timeout", () => {
      request.destroy(new Error(`timeout after ${timeoutMs}ms`));
    });
    request.on("error", (error) => finish({ ok: false, status: error.message, bytes: 0 }));
    request.end();
  });
}
