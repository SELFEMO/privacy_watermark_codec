<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import BrandMark from "./components/BrandMark.vue";
import type {
  CancelTaskRequest,
  DecodeRequest,
  DecodeResponse,
  EncodeRequest,
  EncodeResponse,
  FfmpegBinaryInfo,
  FfmpegRuntimeInfo,
  KeyMode,
  LaunchContext,
  ScanDetection,
  ScanRequest,
  ScanResponse,
  ReleaseMetadata,
  TaskProgressEvent,
  TaskProgressKind,
} from "./types";

type TabId = "encode" | "decode" | "scan" | "ffmpeg";
type Language = "zh" | "en";

type MessageKey = keyof typeof messages.zh;

const messages = {
  zh: {
    appTitle: "图隐私水印编解码器",
    eyebrow: "PRIVACY WATERMARK CODEC",
    subtitle: "不可见频域水印 · 加密载荷 · 感知篡改检测 · 本地离线处理",
    localOnly: "本地离线处理",
    tabEncode: "编码水印",
    tabDecode: "解码检测",
    tabScan: "未知图片扫描",
    tabFfmpeg: "FFmpeg 信息",
    mediaOutput: "媒体与输出",
    mediaOutputDesc: "单张、批量图片与常见视频。",
    chooseMedia: "选择图片或视频",
    chooseMediaHint: "点击打开系统文件选择器",
    chooseOutput: "选择输出文件夹",
    chooseOutputHint: "所有结果将写入该目录",
    contentKey: "内容与密钥",
    contentKeyDesc: "文本先加密，再写入图像亮度频域。",
    watermarkText: "水印文本",
    watermarkPlaceholder: "例如: 版权所有 / 项目编号 / 联系方式",
    modeA: "模式 A · 独立密钥",
    modeADesc: "每文件单独生成密钥，隔离性最高。",
    modeB: "模式 B · 共享密钥",
    modeBDesc: "整批文件共用一个密钥，便于归档。",
    modeC: "模式 C · 自定义密码",
    modeCDesc: "由密码派生密钥，可不生成密钥文件。",
    paramsRun: "参数与执行",
    paramsRunDesc: "系统会强制检查 PSNR。",
    customPassword: "自定义密码",
    customPasswordHint: "建议不少于十二个字符",
    embedStrength: "嵌入强度",
    strengthHint: "数值越高越抗压缩，但图像改变量也会增加。",
    frameParallelism: "视频编码并行度",
    frameParallelismHint: "仅影响视频抽帧后的水印嵌入，数值越高越占用 CPU 与磁盘 I/O。",
    decodeFrameParallelism: "视频解码并行度",
    decodeFrameParallelismHint: "这是当前设备的本地检测并行度，不依赖编码设备，也不会写入水印文件。",
    writeKey: "生成 .key 文件",
    writeKeyCustom: "关闭后仅凭密码解码",
    writeKeyRandom: "随机密钥模式必须生成密钥文件",
    startEncode: "开始编码",
    encoding: "正在本地处理…",
    encodeDone: "编码完成",
    output: "输出",
    evidenceManifest: "证据清单",
    otherFiles: "其余文件",
    selectDecodeMedia: "选择待检测媒体",
    selectDecodeMediaDesc: "批量解码应使用同一密钥或密码。",
    chooseWatermarked: "选择已加水印图片或视频",
    chooseMediaClick: "点击选择媒体",
    credentials: "提供解码凭据",
    credentialsDesc: "密钥文件和自定义密码任选其一。",
    chooseKeyFile: "选择 .key 文件",
    chooseKeyFileHint: "适用于模式 A / B / 已导出密钥的模式 C",
    orPassword: "或输入自定义密码",
    passwordModeC: "模式 C 密码",
    startDecode: "开始解码与检测",
    decoding: "正在逐帧检测…",
    detectResult: "检测结果",
    detectResultDesc: "显示水印文本与篡改判断。",
    noObviousTamper: "未发现明显篡改",
    suspiciousChange: "存在可疑变化",
    modified: "检测到修改",
    fingerprintDistance: "指纹距离",
    correctedCodewords: "纠正码字",
    validFrames: "有效帧",
    modifiedFrames: "修改帧",
    tamperRegions: "疑似篡改分区",
    syncRegistration: "同步配准",
    moreResults: "还有 {count} 个结果已完成。",
    decodeResultCount: "已显示全部 {count} 个结果。",
    waitingDecode: "等待解码结果。",
    importUnknown: "导入未知图片",
    importUnknownDesc: "无密钥扫描本软件水印头、C2PA/AI 元数据等痕迹。",
    chooseUnknown: "选择未知来源图片",
    unknownHint: "支持 PNG / JPEG / WebP / BMP / TIFF，可多选",
    scanWatermark: "扫描隐私水印",
    scanning: "正在扫描…",
    scanHint: "未检出时会返回非确定性结论；专有 AI 隐形水印可能需要原厂模型或密钥验证。",
    scanFeedback: "扫描反馈",
    scanFeedbackDesc: "如能读取明文元数据，会直接展示上下文。",
    foundTrace: "发现痕迹",
    notDetected: "未检出",
    imageSize: "图片尺寸",
    needKey: "需密钥读取正文",
    readable: "已提取可读内容",
    waitingScan: "等待扫描结果。",
    runtimeStatus: "运行状态",
    runtimeStatusDesc: "按当前平台读取内置 FFmpeg 资源。",
    refresh: "重新检测",
    resourceError: "资源异常",
    resourceErrorDetail: "未能读取内置 FFmpeg 资源",
    waitingCheck: "等待检测",
    waitingCheckDetail: "尚未读取运行时信息",
    missingBinary: "缺少二进制",
    missingBinaryDetail: "当前平台至少需要 ffmpeg 和 ffprobe",
    needsFix: "需要处理",
    needsFixDetail: "请重新生成 manifest 或检查文件是否被替换",
    runtimeOk: "运行时可用",
    runtimeOkDetail: "当前平台内置 FFmpeg 已通过校验",
    platform: "平台",
    ffmpegBuild: "FFmpeg 构建",
    utcCompileDate: "UTC 编译日期",
    undeclared: "未声明",
    licenseJudgement: "许可证判断",
    manifestTime: "清单时间",
    notGenerated: "未生成",
    sourceDesc: "来源说明",
    autoUpdate: "自动更新",
    packageSignature: "发布包签名",
    updateManifest: "更新清单",
    enabled: "已启用",
    disabled: "未启用",
    ffmpegResourceFailed: "未能读取 FFmpeg 资源",
    binaryHash: "二进制与哈希",
    binaryHashDesc: "视频功能执行前会强制校验 ffmpeg 和 ffprobe。",
    hashOk: "校验通过",
    hashNeedsFix: "需处理",
    versionLineMissing: "未能读取构建行",
    path: "路径",
    expectedSha: "期望 SHA-256",
    actualSha: "实际 SHA-256",
    notRead: "未读取",
    noCurrentBinary: "未读取到当前平台的 FFmpeg 二进制信息。",
    licenseBuild: "许可证与构建参数",
    licenseBuildDesc: "正式发布时需保留来源、许可证和源码获取说明。",
    buildParams: "构建参数",
    licenseOutput: "许可证输出",
    waitingFfmpeg: "等待 FFmpeg 信息。",
    failure: "处理失败",
    closeFailure: "关闭失败提醒",
    footer: "所有媒体、密码和密钥均在本机处理，不上传网络。",
    progressTitle: "处理进度",
    cancelTask: "取消任务",
    cancellingTask: "正在取消…",
    progressCancelled: "任务已取消",
    progressFailed: "任务已结束但未成功",
    clearKeyFile: "清除密钥",
    currentImport: "本次导入",
    clearImportedFiles: "清除导入",
    progressCurrentFile: "当前文件",
    progressCounter: "数量",
    progressQueued: "任务已进入处理队列",
    progressPreparing: "正在准备任务",
    progressPreparingFile: "正在准备当前文件",
    progressWritingKey: "正在写入密钥文件",
    progressProcessingImage: "正在处理图片",
    progressDecodingImage: "正在解码检测图片",
    progressScanningImage: "正在扫描图片",
    progressExtractingFrames: "正在抽取视频帧",
    progressProcessingFrames: "正在处理视频帧",
    progressCheckingFrames: "正在逐帧检测视频",
    progressMuxingVideo: "正在封装输出视频",
    progressSummarizingVideo: "正在汇总视频结果",
    progressWritingEvidence: "正在写入证据清单",
    progressCompletedFile: "当前文件已完成",
    progressCompleted: "任务已完成",
    selectedFiles: "已选择 {count} 个文件",
    importedFromContext: "已从右键菜单导入 {count} 个文件。",
    contextEncode: "右键菜单已导入到编码页，请选择输出目录并填写水印文本。",
    contextDecode: "右键菜单已导入到解码页，请选择密钥文件或输入密码。",
    contextScan: "右键菜单已导入到扫描页，已自动开始无密钥扫描。",
  },
  en: {
    appTitle: "Privacy Watermark Codec",
    eyebrow: "PRIVACY WATERMARK CODEC",
    subtitle: "Invisible frequency-domain watermark · encrypted payload · perceptual tamper detection · local processing",
    localOnly: "Local only",
    tabEncode: "Encode",
    tabDecode: "Decode",
    tabScan: "Unknown image scan",
    tabFfmpeg: "FFmpeg info",
    mediaOutput: "Media and output",
    mediaOutputDesc: "Single file, batch images, and common videos.",
    chooseMedia: "Choose images or videos",
    chooseMediaHint: "Open the system file picker",
    chooseOutput: "Choose output folder",
    chooseOutputHint: "All results will be written here",
    contentKey: "Content and key",
    contentKeyDesc: "Text is encrypted first, then embedded into image luminance frequency coefficients.",
    watermarkText: "Watermark text",
    watermarkPlaceholder: "Example: Copyright / Project ID / Contact",
    modeA: "Mode A · Independent key",
    modeADesc: "Generate one key per file for stronger isolation.",
    modeB: "Mode B · Shared key",
    modeBDesc: "Use one key for the whole batch for easier archiving.",
    modeC: "Mode C · Custom password",
    modeCDesc: "Derive the key from your password; key file export is optional.",
    paramsRun: "Parameters and run",
    paramsRunDesc: "PSNR is checked automatically.",
    customPassword: "Custom password",
    customPasswordHint: "At least twelve characters is recommended",
    embedStrength: "Embedding strength",
    strengthHint: "Higher values improve compression robustness but change pixels more.",
    frameParallelism: "Video encode parallelism",
    frameParallelismHint: "Applies after video frame extraction during embedding; higher values use more CPU and disk I/O.",
    decodeFrameParallelism: "Video decode parallelism",
    decodeFrameParallelismHint: "This is a local inspection setting for the current device; it is not embedded in the watermark file.",
    writeKey: "Write .key file",
    writeKeyCustom: "Disable this to decode only with the password",
    writeKeyRandom: "Random key modes must write a key file",
    startEncode: "Start encoding",
    encoding: "Processing locally…",
    encodeDone: "Encoding complete",
    output: "Output",
    evidenceManifest: "Evidence manifest",
    otherFiles: "Other files",
    selectDecodeMedia: "Select media to inspect",
    selectDecodeMediaDesc: "Batch decoding should use the same key or password.",
    chooseWatermarked: "Choose watermarked images or videos",
    chooseMediaClick: "Choose media files",
    credentials: "Provide credentials",
    credentialsDesc: "Use either a key file or a custom password.",
    chooseKeyFile: "Choose .key file",
    chooseKeyFileHint: "For Mode A / B / exported Mode C keys",
    orPassword: "Or enter a custom password",
    passwordModeC: "Mode C password",
    startDecode: "Decode and inspect",
    decoding: "Inspecting frame by frame…",
    detectResult: "Inspection result",
    detectResultDesc: "Shows watermark text and tamper status.",
    noObviousTamper: "No obvious tampering",
    suspiciousChange: "Suspicious change",
    modified: "Modification detected",
    fingerprintDistance: "Fingerprint distance",
    correctedCodewords: "Corrected codewords",
    validFrames: "Valid frames",
    modifiedFrames: "Modified frames",
    tamperRegions: "Suspicious regions",
    syncRegistration: "Sync registration",
    moreResults: "{count} more result(s) completed.",
    decodeResultCount: "Showing all {count} result(s).",
    waitingDecode: "Waiting for decode result.",
    importUnknown: "Import unknown images",
    importUnknownDesc: "Scan this app's watermark header, C2PA/AI metadata, and related traces without a key.",
    chooseUnknown: "Choose unknown images",
    unknownHint: "PNG / JPEG / WebP / BMP / TIFF, multi-select supported",
    scanWatermark: "Scan privacy watermark",
    scanning: "Scanning…",
    scanHint: "A negative result is not a guarantee; proprietary AI watermarks may require vendor models or keys.",
    scanFeedback: "Scan feedback",
    scanFeedbackDesc: "Readable plaintext metadata will be shown with context.",
    foundTrace: "Trace found",
    notDetected: "Not detected",
    imageSize: "Image size",
    needKey: "Key required for body",
    readable: "Readable content extracted",
    waitingScan: "Waiting for scan result.",
    runtimeStatus: "Runtime status",
    runtimeStatusDesc: "Read bundled FFmpeg resources for the current platform.",
    refresh: "Refresh",
    resourceError: "Resource error",
    resourceErrorDetail: "Failed to read bundled FFmpeg resources",
    waitingCheck: "Waiting",
    waitingCheckDetail: "Runtime info has not been loaded",
    missingBinary: "Missing binary",
    missingBinaryDetail: "This platform requires at least ffmpeg and ffprobe",
    needsFix: "Needs attention",
    needsFixDetail: "Regenerate manifest or check whether files were replaced",
    runtimeOk: "Runtime ready",
    runtimeOkDetail: "Bundled FFmpeg for this platform passed verification",
    platform: "Platform",
    ffmpegBuild: "FFmpeg build",
    utcCompileDate: "UTC compile date",
    undeclared: "Undeclared",
    licenseJudgement: "License judgment",
    manifestTime: "Manifest time",
    notGenerated: "Not generated",
    sourceDesc: "Source note",
    autoUpdate: "Automatic update",
    packageSignature: "Package signature",
    updateManifest: "Update manifest",
    enabled: "Enabled",
    disabled: "Disabled",
    ffmpegResourceFailed: "Failed to read FFmpeg resources",
    binaryHash: "Binaries and hashes",
    binaryHashDesc: "ffmpeg and ffprobe are verified before video tasks run.",
    hashOk: "Verified",
    hashNeedsFix: "Needs attention",
    versionLineMissing: "Build line unavailable",
    path: "Path",
    expectedSha: "Expected SHA-256",
    actualSha: "Actual SHA-256",
    notRead: "Not read",
    noCurrentBinary: "No FFmpeg binary info was loaded for the current platform.",
    licenseBuild: "License and build flags",
    licenseBuildDesc: "Keep source, license, and source-code offer notes for release builds.",
    buildParams: "Build flags",
    licenseOutput: "License output",
    waitingFfmpeg: "Waiting for FFmpeg info.",
    failure: "Failed",
    closeFailure: "Close failure alert",
    footer: "All media, passwords, and keys are processed locally and never uploaded.",
    progressTitle: "Progress",
    cancelTask: "Cancel task",
    cancellingTask: "Cancelling…",
    progressCancelled: "Task cancelled",
    progressFailed: "Task finished with errors",
    clearKeyFile: "Clear key",
    currentImport: "Current import",
    clearImportedFiles: "Clear import",
    progressCurrentFile: "Current file",
    progressCounter: "Count",
    progressQueued: "Task queued",
    progressPreparing: "Preparing task",
    progressPreparingFile: "Preparing current file",
    progressWritingKey: "Writing key file",
    progressProcessingImage: "Processing image",
    progressDecodingImage: "Decoding image",
    progressScanningImage: "Scanning image",
    progressExtractingFrames: "Extracting video frames",
    progressProcessingFrames: "Processing video frames",
    progressCheckingFrames: "Checking video frames",
    progressMuxingVideo: "Muxing output video",
    progressSummarizingVideo: "Summarizing video result",
    progressWritingEvidence: "Writing evidence manifest",
    progressCompletedFile: "Current file completed",
    progressCompleted: "Task completed",
    selectedFiles: "{count} file(s) selected",
    importedFromContext: "Imported {count} file(s) from the context menu.",
    contextEncode: "Files were imported into Encode. Choose an output folder and enter watermark text.",
    contextDecode: "Files were imported into Decode. Choose a key file or enter a password.",
    contextScan: "Files were imported into Scan. The keyless scan started automatically.",
  },
} as const;

const activeTab = ref<TabId>("encode");
const language = ref<Language>(detectLanguage());
const encodeInputs = ref<string[]>([]);
const decodeInputs = ref<string[]>([]);
const scanInputs = ref<string[]>([]);
const outputDir = ref("");
const watermarkText = ref("");
const keyMode = ref<KeyMode>("independent");
const customPassword = ref("");
const writeKeyFile = ref(true);
const strength = ref(8);
const frameParallelism = ref(1);
const decodeFrameParallelism = ref(1);
const decodeKeyFile = ref("");
const decodePassword = ref("");
const busy = ref(false);
const cancelling = ref(false);
const errorMessage = ref("");
const contextMessage = ref("");
const encodeResult = ref<EncodeResponse | null>(null);
const decodeResult = ref<DecodeResponse | null>(null);
const scanResult = ref<ScanResponse | null>(null);
const ffmpegInfo = ref<FfmpegRuntimeInfo | null>(null);
const releaseInfo = ref<ReleaseMetadata | null>(null);
const ffmpegError = ref("");
const launchBatch = ref<LaunchContext>({ files: [] });
const taskProgress = ref<TaskProgressEvent | null>(null);
const activeTaskId = ref("");
let launchTimer: number | undefined;
let unlistenLaunchContext: (() => void) | undefined;
let unlistenTaskProgress: (() => void) | undefined;

const canEncode = computed(
  () =>
    encodeInputs.value.length > 0 &&
    outputDir.value.length > 0 &&
    watermarkText.value.trim().length > 0 &&
    (keyMode.value !== "custom" || customPassword.value.length > 0),
);

const canDecode = computed(
  () =>
    decodeInputs.value.length > 0 &&
    (decodeKeyFile.value.length > 0 || decodePassword.value.length > 0),
);

const canScan = computed(() => scanInputs.value.length > 0);
const currentMessages = computed(() => messages[language.value]);
const progressPercent = computed(() => Math.round(Math.max(0, Math.min(taskProgress.value?.percent ?? 0, 100))));
const displayProgressMessage = computed(() => formatProgressMessage(taskProgress.value));
const displayErrorMessage = computed(() => localizeRuntimeMessage(errorMessage.value));
const ffmpegBinaries = computed<FfmpegBinaryInfo[]>(() => {
  if (!ffmpegInfo.value) return [];
  return [ffmpegInfo.value.ffmpeg, ffmpegInfo.value.ffprobe, ...ffmpegInfo.value.extraBinaries].filter(
    (item): item is FfmpegBinaryInfo => Boolean(item),
  );
});

const ffmpegStatus = computed(() => {
  if (ffmpegError.value) {
    return { label: t("resourceError"), className: "danger", detail: t("resourceErrorDetail") };
  }
  if (!ffmpegInfo.value) {
    return { label: t("waitingCheck"), className: "idle", detail: t("waitingCheckDetail") };
  }
  const required = [ffmpegInfo.value.ffmpeg, ffmpegInfo.value.ffprobe].filter(Boolean) as FfmpegBinaryInfo[];
  if (required.length < 2) {
    return { label: t("missingBinary"), className: "danger", detail: t("missingBinaryDetail") };
  }
  if (required.some((binary) => binary.error || !binary.hashOk)) {
    return { label: t("needsFix"), className: "warning", detail: t("needsFixDetail") };
  }

  // 只有执行用的 ffmpeg/ffprobe 都通过校验时才展示为可用，避免 ffplay 等非关键文件影响视频处理状态判断。
  // The runtime is marked usable only when executable ffmpeg/ffprobe pass verification, so optional ffplay does not block video processing status.
  return { label: t("runtimeOk"), className: "ok", detail: t("runtimeOkDetail") };
});

function detectLanguage(): Language {
  const saved = window.localStorage.getItem("pwc-language");
  if (saved === "zh" || saved === "en") return saved;
  return navigator.language.toLowerCase().startsWith("zh") ? "zh" : "en";
}

function setLanguage(nextLanguage: Language) {
  language.value = nextLanguage;
  window.localStorage.setItem("pwc-language", nextLanguage);
}

function selectTab(tab: TabId) {
  activeTab.value = tab;
  if (tab === "ffmpeg" && !ffmpegInfo.value && !ffmpegError.value) {
    void loadFfmpegInfo();
  }
}

function t(key: MessageKey, params?: Record<string, string | number>): string {
  let value: string = currentMessages.value[key] || messages.zh[key] || key;
  for (const [name, replacement] of Object.entries(params || {})) {
    value = value.replace(`{${name}}`, String(replacement));
  }
  return value;
}

function basename(path: string): string {
  return path.split(/[\\/]/).pop() || path;
}

function containsChinese(text: string): boolean {
  return /[\u4e00-\u9fff]/.test(text);
}

function formatProgressMessage(progress: TaskProgressEvent | null): string {
  if (!progress?.message) return "";
  if (language.value === "zh") return progress.message;

  const englishDetail = englishProgressMessage(progress);
  if (englishDetail) return englishDetail;

  return localizeRuntimeMessage(progress.message, progress.phase);
}

function englishProgressMessage(progress: TaskProgressEvent): string {
  const current = progress.current ?? 0;
  const total = progress.total ?? 0;
  const counter = total ? `${current}/${total}` : "";
  const message = progress.message || "";
  const elapsed = message.match(/已运行\s*(\d+)\s*秒/)?.[1];
  const generatedFrames = message.match(/已生成\s*(\d+)\s*帧/)?.[1];
  const stagePercent = message.match(/阶段进度\s*([0-9.]+)%/)?.[1];
  const elapsedPart = elapsed ? `, elapsed ${elapsed}s` : "";
  const stagePart = stagePercent ? `, stage ${stagePercent}%` : "";
  const generatedPart = generatedFrames ? `, ${generatedFrames} frame(s) generated` : "";

  switch (progress.phase) {
    case "preparing":
      return progress.task === "encode" ? "Preparing the encoding task" : progress.task === "decode" ? "Preparing the decode task" : "Preparing the scan task";
    case "writing-key":
      return "Writing the key file";
    case "preparing-file":
      return counter ? `Preparing file ${counter}` : "Preparing current file";
    case "processing-image":
      return counter ? `Embedding image watermark: ${counter}` : "Embedding image watermark";
    case "decoding-image":
      return counter ? `Decoding and inspecting image: ${counter}` : "Decoding and inspecting image";
    case "scanning-image":
      return counter ? `Scanning image: ${counter}` : "Scanning image";
    case "extracting-video-frames":
      return `Extracting video frames${stagePart}${generatedPart}${elapsedPart}`;
    case "processing-video-frames":
      return counter ? `Embedding video-frame watermark: ${counter}` : "Embedding video-frame watermark";
    case "checking-video-frames":
      return counter ? `Decoding and inspecting video frames: ${counter}` : "Decoding and inspecting video frames";
    case "muxing-video":
      return `Muxing output video${stagePart}${elapsedPart}`;
    case "summarizing-video":
      return "Summarizing video inspection results";
    case "writing-evidence":
      return "Writing the evidence manifest";
    case "completed-file":
      return "Current file completed";
    case "completed":
      return "Task completed";
    case "failed":
      return "Task finished with errors";
    case "cancelled":
      return "Task cancelled";
    default:
      return containsChinese(message) ? progressPhaseLabel(progress.phase) : message;
  }
}

function localizeRuntimeMessage(message: string, phase?: string): string {
  if (!message || language.value === "zh" || !containsChinese(message)) return message;

  const replacements: Array<[RegExp, string]> = [
    [/正在准备批量编码任务/g, "Preparing batch encoding task"],
    [/正在写入批次密钥文件/g, "Writing batch key file"],
    [/正在准备编码第\s*(\d+)\/(\d+)\s*个文件/g, "Preparing file $1/$2 for encoding"],
    [/正在嵌入图片水印[:：]\s*/g, "Embedding image watermark: "],
    [/第\s*(\d+)\/(\d+)\s*个文件编码完成/g, "File $1/$2 encoded"],
    [/正在写入证据清单[:：]\s*/g, "Writing evidence manifest: "],
    [/批量编码任务完成/g, "Batch encoding completed"],
    [/正在准备批量解码检测任务/g, "Preparing batch decode task"],
    [/正在准备检测第\s*(\d+)\/(\d+)\s*个文件/g, "Preparing file $1/$2 for inspection"],
    [/正在解码并检测图片[:：]\s*/g, "Decoding and inspecting image: "],
    [/批量解码检测任务完成/g, "Batch decode task completed"],
    [/正在准备未知来源图片扫描/g, "Preparing unknown-image scan"],
    [/正在扫描第\s*(\d+)\/(\d+)\s*张图片/g, "Scanning image $1/$2"],
    [/未知来源图片扫描完成/g, "Unknown-image scan completed"],
    [/正在使用 FFmpeg 抽取待检测视频帧/g, "Extracting frames from the video to inspect"],
    [/正在使用 FFmpeg 抽取视频帧/g, "Extracting video frames"],
    [/FFmpeg 正在抽取待检测视频帧，输出会先写入临时工作区/g, "FFmpeg is extracting frames to a temporary workspace"],
    [/FFmpeg 正在抽取视频帧，输出会先写入临时工作区/g, "FFmpeg is extracting frames to a temporary workspace"],
    [/视频帧已抽取，开始逐帧嵌入水印/g, "Frames extracted; embedding the watermark frame by frame"],
    [/视频帧已抽取，开始逐帧解码检测/g, "Frames extracted; decoding and inspecting frame by frame"],
    [/正在嵌入视频帧水印[:：]\s*/g, "Embedding video-frame watermark: "],
    [/正在逐帧解码检测[:：]\s*/g, "Decoding frames: "],
    [/正在重新封装视频并复制音轨/g, "Muxing the output video and copying audio"],
    [/FFmpeg 正在重新封装输出视频/g, "FFmpeg is muxing the output video"],
    [/正在汇总视频逐帧检测结果/g, "Summarizing per-frame video results"],
    [/当前视频编码完成/g, "Current video encoded"],
    [/当前视频解码检测完成/g, "Current video inspected"],
    [/阶段进度\s*(\d+)%/g, "stage progress $1%"],
    [/已运行\s*(\d+)\s*秒/g, "elapsed $1s"],
    [/已生成\s*(\d+)\s*帧/g, "$1 frame(s) generated"],
    [/任务已取消/g, "Task cancelled"],
    [/后台任务异常终止[:：]\s*/g, "Background task terminated unexpectedly: "],
    [/请至少选择一个待解码媒体文件/g, "Select at least one media file to decode"],
    [/请至少选择一个待编码媒体文件/g, "Select at least one media file to encode"],
    [/请至少选择一张待检测图片/g, "Select at least one image to scan"],
    [/输入文件不存在[:：]\s*/g, "Input file does not exist: "],
    [/不支持的文件类型[:：]\s*/g, "Unsupported file type: "],
    [/未知来源隐私水印扫描当前仅支持图片[:：]\s*/g, "Unknown-source watermark scanning currently supports images only: "],
    [/水印文本不能为空/g, "Watermark text cannot be empty"],
    [/嵌入强度必须位于\s*3\s*到\s*20\s*之间/g, "Embedding strength must be between 3 and 20"],
    [/当前参数得到的 PSNR 为\s*([0-9.]+)\s*dB，低于\s*([0-9.]+)\s*dB。请降低嵌入强度或缩短文本/g, "The current settings produced PSNR $1 dB, below $2 dB. Lower the embedding strength or shorten the text."],
    [/FFmpeg 命令执行失败（退出码\s*([^）]+)）[:：]\s*/g, "FFmpeg command failed (exit code $1): "],
    [/无法启动\s*([\s\S]*?)[:：]\s*Permission denied \(os error 13\)/g, "Unable to start $1: permission denied. Run npm run ffmpeg:manifest or chmod +x the bundled FFmpeg files."],
    [/无法执行\s*([\s\S]*?)[:：]\s*Permission denied \(os error 13\)/g, "Unable to execute $1: permission denied. Run npm run ffmpeg:manifest or chmod +x the bundled FFmpeg files."],
    [/无法启动\s*([\s\S]*?)[:：]/g, "Unable to start $1: "],
    [/无法执行\s*([\s\S]*?)[:：]/g, "Unable to execute $1: "],
    [/无法设置 FFmpeg 可执行权限\s*([^：:]+)[:：]\s*/g, "Unable to set executable permission on FFmpeg $1: "],
    [/无法读取 FFmpeg 文件权限\s*([^：:]+)[:：]\s*/g, "Unable to read FFmpeg file permissions $1: "],
    [/无法启动/g, "Unable to start"],
    [/无法执行/g, "Unable to execute"],
    [/等待/g, "Waiting for"],
    [/执行结果失败[:：]\s*/g, "result failed: "],
  ];

  let localized = message;
  for (const [pattern, replacement] of replacements) {
    localized = localized.replace(pattern, replacement);
  }
  // 如果仍含有中文，说明后端返回了未覆盖的新诊断。英文界面下保留原文会再次出现中英混排，因此用阶段标签兜底。
  // If Chinese remains, the backend returned a new diagnostic. In English mode we fall back to the phase label to avoid mixed-language UI.
  return containsChinese(localized) ? progressPhaseLabel(phase || taskProgress.value?.phase) : localized;
}


function previewNames(paths: string[], limit = 4): string[] {
  return paths.slice(0, limit).map(basename);
}

function restCount(paths: string[], limit = 4): number {
  return Math.max(0, paths.length - limit);
}

function clearEncodeInputs() {
  // 清除导入只影响当前待处理列表，不清除输出目录，避免用户重新选择导出位置。
  // Clearing imported media only resets the current processing list and keeps the output folder to avoid extra re-selection work.
  encodeInputs.value = [];
}

function clearDecodeInputs() {
  // 解码媒体与解码凭据是两个独立输入，清除媒体不应误删密钥或密码。
  // Decode media and credentials are independent inputs, so clearing media must not remove the key or password.
  decodeInputs.value = [];
}

function clearScanInputs() {
  // 未知图片扫描的导入列表独立维护，清除后保留历史结果，便于用户对照上一次反馈。
  // The unknown-image scan list is independent; results are kept after clearing so users can compare the previous feedback.
  scanInputs.value = [];
}

function updateViewportScale() {
  const viewport = window.visualViewport;
  const width = Math.round(viewport?.width ?? window.innerWidth);
  const height = Math.round(viewport?.height ?? window.innerHeight);
  const rawScale = Math.min(width / 1180, height / 820, 1);
  const uiScale = Math.max(0.72, Math.min(rawScale, 1));
  const density = height < 760 || width < 980 ? "compact" : "regular";

  // Tauri/WebView 已经把系统缩放和 DPI 折算进渲染尺寸，这里只保留内部缩放逻辑，不再把调试信息暴露给终端用户。
  // Tauri/WebView already applies system scaling and DPI to rendering, so only the internal UI scaling is kept and the debug readout is hidden from end users.
  document.documentElement.style.setProperty("--ui-scale", String(uiScale));
  document.documentElement.dataset.density = density;
}

function integrityLabel(status: string): string {
  if (status === "intact") return t("noObviousTamper");
  if (status === "uncertain") return t("suspiciousChange");
  return t("modified");
}

function detectionAccessLabel(detection: ScanDetection): string {
  return detection.needsKey ? t("needKey") : t("readable");
}


function createTaskId(task: TaskProgressKind): string {
  return `${task}-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

function beginTaskProgress(task: TaskProgressKind): string {
  const taskId = createTaskId(task);
  activeTaskId.value = taskId;
  taskProgress.value = {
    taskId,
    task,
    phase: "queued",
    message: t("progressQueued"),
    current: 0,
    total: 0,
    percent: 0,
  };
  return taskId;
}

function showProgress(task: TaskProgressKind): boolean {
  return taskProgress.value?.task === task;
}

async function cancelCurrentTask() {
  if (!activeTaskId.value || !busy.value || cancelling.value) return;
  cancelling.value = true;
  const request: CancelTaskRequest = { taskId: activeTaskId.value };
  if (taskProgress.value) {
    taskProgress.value = {
      ...taskProgress.value,
      phase: "cancelled",
      message: t("cancellingTask"),
    };
  }
  try {
    // 取消只提交任务编号，不携带密码、水印正文或媒体路径，避免取消请求暴露敏感上下文。
    // Cancellation submits only the task id, not passwords, watermark text, or media paths, so the request does not expose sensitive context.
    await invoke<void>("cancel_task", { request });
  } catch (error) {
    errorMessage.value = String(error);
    cancelling.value = false;
  }
}

function progressPhaseLabel(phase?: string): string {
  const labels: Record<string, MessageKey> = {
    queued: "progressQueued",
    preparing: "progressPreparing",
    "preparing-file": "progressPreparingFile",
    "writing-key": "progressWritingKey",
    "processing-image": "progressProcessingImage",
    "decoding-image": "progressDecodingImage",
    "scanning-image": "progressScanningImage",
    "extracting-video-frames": "progressExtractingFrames",
    "processing-video-frames": "progressProcessingFrames",
    "checking-video-frames": "progressCheckingFrames",
    "muxing-video": "progressMuxingVideo",
    "summarizing-video": "progressSummarizingVideo",
    "writing-evidence": "progressWritingEvidence",
    "completed-file": "progressCompletedFile",
    completed: "progressCompleted",
    cancelled: "progressCancelled",
    failed: "progressFailed",
  };
  return t(labels[phase || "queued"] || "progressPreparing");
}


function handleTaskError(error: unknown) {
  const message = String(error);
  if (message.includes("任务已取消") || message.toLowerCase().includes("cancel")) {
    if (taskProgress.value) {
      taskProgress.value = {
        ...taskProgress.value,
        phase: "cancelled",
        message: t("progressCancelled"),
      };
    }
    return;
  }
  errorMessage.value = message;
  if (taskProgress.value) {
    taskProgress.value = {
      ...taskProgress.value,
      phase: "failed",
      message,
      percent: 100,
    };
  }
}

async function chooseEncodeInputs() {
  const selected = await open({
    multiple: true,
    directory: false,
    filters: [
      {
        name: language.value === "zh" ? "图片与视频" : "Images and videos",
        extensions: ["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff", "mp4", "mov", "mkv", "avi", "webm"],
      },
    ],
  });
  if (selected) encodeInputs.value = Array.isArray(selected) ? selected : [selected];
}

async function chooseDecodeInputs() {
  const selected = await open({
    multiple: true,
    directory: false,
    filters: [
      {
        name: language.value === "zh" ? "已加水印的图片与视频" : "Watermarked images and videos",
        extensions: ["png", "jpg", "jpeg", "webp", "mp4", "mov", "mkv", "avi", "webm"],
      },
    ],
  });
  if (selected) decodeInputs.value = Array.isArray(selected) ? selected : [selected];
}

async function chooseScanInputs() {
  const selected = await open({
    multiple: true,
    directory: false,
    filters: [
      {
        name: language.value === "zh" ? "未知来源图片" : "Unknown images",
        extensions: ["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff"],
      },
    ],
  });
  if (selected) scanInputs.value = Array.isArray(selected) ? selected : [selected];
}

async function chooseOutputDir() {
  const selected = await open({ multiple: false, directory: true });
  if (typeof selected === "string") outputDir.value = selected;
}

async function chooseKeyFile() {
  const selected = await open({
    multiple: false,
    directory: false,
    filters: [{ name: language.value === "zh" ? "水印密钥" : "Watermark key", extensions: ["key", "json"] }],
  });
  if (typeof selected === "string") {
    decodeKeyFile.value = selected;
    decodePassword.value = "";
  }
}

function clearDecodeKeyFile() {
  decodeKeyFile.value = "";
}

watch(decodePassword, (value) => {
  if (value.length > 0 && decodeKeyFile.value.length > 0) {
    // 密码输入和 Key 文件互斥，避免界面同时显示两种凭据而用户无法判断实际使用哪一种。
    // Password input and key files are mutually exclusive so users do not see two credentials without knowing which one is used.
    decodeKeyFile.value = "";
  }
});


async function runEncode() {
  if (!canEncode.value || busy.value) return;
  busy.value = true;
  errorMessage.value = "";
  encodeResult.value = null;
  const taskId = beginTaskProgress("encode");

  const request: EncodeRequest = {
    inputPaths: encodeInputs.value,
    outputDir: outputDir.value,
    text: watermarkText.value,
    keyMode: keyMode.value,
    customPassword: keyMode.value === "custom" ? customPassword.value : undefined,
    writeKeyFile: writeKeyFile.value,
    strength: strength.value,
    frameParallelism: frameParallelism.value,
    taskId,
  };

  try {
    encodeResult.value = await invoke<EncodeResponse>("encode_media", { request });
  } catch (error) {
    handleTaskError(error);
  } finally {
    busy.value = false;
    cancelling.value = false;
  }
}

async function runDecode() {
  if (!canDecode.value || busy.value) return;
  busy.value = true;
  errorMessage.value = "";
  decodeResult.value = null;
  const taskId = beginTaskProgress("decode");

  const request: DecodeRequest = {
    inputPaths: decodeInputs.value,
    keyFile: decodeKeyFile.value || undefined,
    customPassword: decodePassword.value || undefined,
    frameParallelism: decodeFrameParallelism.value,
    taskId,
  };

  try {
    decodeResult.value = await invoke<DecodeResponse>("decode_media", { request });
  } catch (error) {
    handleTaskError(error);
  } finally {
    busy.value = false;
    cancelling.value = false;
  }
}

async function loadFfmpegInfo() {
  ffmpegError.value = "";
  try {
    // 许可证与哈希信息统一由后端返回，这样前端看到的路径和真正执行的视频二进制完全一致。
    // License and hash data come from the backend so the UI reflects the exact same binaries that will be executed for video tasks.
    ffmpegInfo.value = await invoke<FfmpegRuntimeInfo>("get_ffmpeg_info");
  } catch (error) {
    ffmpegInfo.value = null;
    ffmpegError.value = String(error);
  }
}

async function loadReleaseInfo() {
  try {
    releaseInfo.value = await invoke<ReleaseMetadata>("get_release_metadata");
  } catch {
    releaseInfo.value = null;
  }
}

async function runScan() {
  if (!canScan.value || busy.value) return;
  busy.value = true;
  errorMessage.value = "";
  scanResult.value = null;
  const taskId = beginTaskProgress("scan");

  const request: ScanRequest = {
    inputPaths: scanInputs.value,
    taskId,
  };

  try {
    scanResult.value = await invoke<ScanResponse>("scan_privacy_watermark", { request });
  } catch (error) {
    handleTaskError(error);
  } finally {
    busy.value = false;
    cancelling.value = false;
  }
}

function mergeLaunchContext(context: LaunchContext) {
  if (!context.action || !context.files?.length) return;

  const current = launchBatch.value;
  const actionChanged = current.action !== context.action;
  const files = actionChanged ? [] : current.files.slice();
  const seen = new Set(files.map((file) => file.toLocaleLowerCase()));

  for (const file of context.files) {
    const key = file.toLocaleLowerCase();
    if (!seen.has(key)) {
      seen.add(key);
      files.push(file);
    }
  }

  launchBatch.value = { action: context.action, files };
  if (launchTimer !== undefined) window.clearTimeout(launchTimer);

  // Windows 静态右键菜单在多选文件时可能按文件逐次启动程序，所以这里短暂等待并合并为一个批次。
  // Windows Explorer may start the static context-menu command once per selected file, so the app waits briefly and merges those launches into one batch.
  launchTimer = window.setTimeout(() => {
    void applyLaunchContext(launchBatch.value);
    launchBatch.value = { files: [] };
    launchTimer = undefined;
  }, 1200);
}

async function applyLaunchContext(context: LaunchContext) {
  if (!context.files?.length || !context.action) return;
  const files = context.files;
  contextMessage.value = t("importedFromContext", { count: files.length });

  if (context.action === "encode") {
    activeTab.value = "encode";
    encodeInputs.value = files;
    contextMessage.value = `${contextMessage.value} ${t("contextEncode")}`;
  } else if (context.action === "decode") {
    activeTab.value = "decode";
    decodeInputs.value = files;
    contextMessage.value = `${contextMessage.value} ${t("contextDecode")}`;
  } else if (context.action === "scan") {
    activeTab.value = "scan";
    scanInputs.value = files;
    contextMessage.value = `${contextMessage.value} ${t("contextScan")}`;
    await runScan();
  }
}

async function applyInitialLaunchContext() {
  try {
    const context = await invoke<LaunchContext>("get_launch_context");
    mergeLaunchContext(context);
  } catch (error) {
    errorMessage.value = String(error);
  }
}

onMounted(() => {
  updateViewportScale();
  void applyInitialLaunchContext();
  void loadReleaseInfo();
  void listen<LaunchContext>("pwc-launch-context", (event) => {
    mergeLaunchContext(event.payload);
  }).then((unlisten) => {
    unlistenLaunchContext = unlisten;
  });
  void listen<TaskProgressEvent>("pwc-task-progress", (event) => {
    if (!event.payload.taskId || event.payload.taskId === activeTaskId.value) {
      // 只接收当前任务的进度，避免上一次后台事件延迟到达后覆盖新任务状态。
      // Only the active task progress is accepted so delayed events from a previous task cannot overwrite the new task state.
      taskProgress.value = event.payload;
    }
  }).then((unlisten) => {
    unlistenTaskProgress = unlisten;
  });
  window.addEventListener("resize", updateViewportScale);
  window.visualViewport?.addEventListener("resize", updateViewportScale);
});

onBeforeUnmount(() => {
  if (launchTimer !== undefined) window.clearTimeout(launchTimer);
  unlistenLaunchContext?.();
  unlistenTaskProgress?.();
  window.removeEventListener("resize", updateViewportScale);
  window.visualViewport?.removeEventListener("resize", updateViewportScale);
});
</script>

<template>
  <main :class="['shell', `lang-${language}`]">
    <header class="hero">
      <div class="hero-copy hero-branding">
        <BrandMark />
        <div>
          <p class="eyebrow">{{ t("eyebrow") }}</p>
          <h1>{{ t("appTitle") }}</h1>
          <p class="subtitle">{{ t("subtitle") }}</p>
        </div>
      </div>
      <div class="hero-side">
        <div class="security-badge">
          <span class="badge-dot"></span>
          {{ t("localOnly") }}
        </div>
        <div class="language-switch" aria-label="Language switch">
          <button :class="{ active: language === 'zh' }" @click="setLanguage('zh')">中</button>
          <button :class="{ active: language === 'en' }" @click="setLanguage('en')">EN</button>
        </div>
      </div>
    </header>

    <nav class="tabs" aria-label="Function tabs">
      <button :class="{ active: activeTab === 'encode' }" @click="selectTab('encode')">{{ t("tabEncode") }}</button>
      <button :class="{ active: activeTab === 'decode' }" @click="selectTab('decode')">{{ t("tabDecode") }}</button>
      <button :class="{ active: activeTab === 'scan' }" @click="selectTab('scan')">{{ t("tabScan") }}</button>
      <button :class="{ active: activeTab === 'ffmpeg' }" @click="selectTab('ffmpeg')">{{ t("tabFfmpeg") }}</button>
    </nav>

    <div v-if="contextMessage" class="context-banner">
      <span>{{ contextMessage }}</span>
      <button type="button" @click="contextMessage = ''">×</button>
    </div>

    <section v-if="activeTab === 'encode'" class="panel panel-grid encode-grid">
      <article class="card">
        <div class="section-heading"><span>01</span><div><h2>{{ t("mediaOutput") }}</h2><p>{{ t("mediaOutputDesc") }}</p></div></div>
        <button class="file-picker" @click="chooseEncodeInputs"><strong>{{ t("chooseMedia") }}</strong><span>{{ encodeInputs.length ? t("selectedFiles", { count: encodeInputs.length }) : t("chooseMediaHint") }}</span></button>
        <div v-if="encodeInputs.length" class="import-summary"><div class="import-summary-head"><small>{{ t("currentImport") }}</small><button type="button" :disabled="busy" @click="clearEncodeInputs">{{ t("clearImportedFiles") }}</button></div><div class="file-list inline-list"><span v-for="name in previewNames(encodeInputs)" :key="name">{{ name }}</span><span v-if="restCount(encodeInputs)">+{{ restCount(encodeInputs) }}</span></div></div>
        <button class="file-picker" @click="chooseOutputDir"><strong>{{ t("chooseOutput") }}</strong><span>{{ outputDir || t("chooseOutputHint") }}</span></button>
      </article>

      <article class="card main-card">
        <div class="section-heading"><span>02</span><div><h2>{{ t("contentKey") }}</h2><p>{{ t("contentKeyDesc") }}</p></div></div>
        <label class="field"><span>{{ t("watermarkText") }}</span><textarea v-model="watermarkText" maxlength="800" :placeholder="t('watermarkPlaceholder')"></textarea><small>{{ watermarkText.length }}/800</small></label>
        <div class="key-cards">
          <label :class="['key-card', { selected: keyMode === 'independent' }]"><input v-model="keyMode" type="radio" value="independent" /><strong>{{ t("modeA") }}</strong><span>{{ t("modeADesc") }}</span></label>
          <label :class="['key-card', { selected: keyMode === 'shared' }]"><input v-model="keyMode" type="radio" value="shared" /><strong>{{ t("modeB") }}</strong><span>{{ t("modeBDesc") }}</span></label>
          <label :class="['key-card', { selected: keyMode === 'custom' }]"><input v-model="keyMode" type="radio" value="custom" /><strong>{{ t("modeC") }}</strong><span>{{ t("modeCDesc") }}</span></label>
        </div>
      </article>

      <article class="card action-card">
        <div class="section-heading"><span>03</span><div><h2>{{ t("paramsRun") }}</h2><p>{{ t("paramsRunDesc") }}</p></div></div>
        <label v-if="keyMode === 'custom'" class="field"><span>{{ t("customPassword") }}</span><input v-model="customPassword" type="password" autocomplete="new-password" :placeholder="t('customPasswordHint')" /></label>
        <label class="field range-field"><span>{{ t("embedStrength") }}: {{ strength }}</span><input v-model.number="strength" type="range" min="5" max="14" step="1" /><small>{{ t("strengthHint") }}</small></label>
        <label class="field range-field"><span>{{ t("frameParallelism") }}: {{ frameParallelism }}</span><input v-model.number="frameParallelism" type="range" min="1" max="8" step="1" /><small>{{ t("frameParallelismHint") }}</small></label>
        <label class="toggle-row"><input v-model="writeKeyFile" type="checkbox" :disabled="keyMode !== 'custom'" /><span><strong>{{ t("writeKey") }}</strong><small>{{ keyMode === "custom" ? t("writeKeyCustom") : t("writeKeyRandom") }}</small></span></label>
        <div class="task-actions">
          <button class="primary" :disabled="!canEncode || busy" @click="runEncode">{{ busy ? t("encoding") : t("startEncode") }}</button>
          <button v-if="showProgress('encode') && busy && activeTaskId === taskProgress?.taskId" class="secondary-button cancel-button" type="button" :disabled="cancelling" @click="cancelCurrentTask">{{ cancelling ? t("cancellingTask") : t("cancelTask") }}</button>
        </div>
        <div v-if="showProgress('encode')" class="progress-card"><div class="progress-top"><strong>{{ t("progressTitle") }}</strong><span>{{ progressPercent }}%</span></div><div class="progress-track"><div class="progress-fill" :style="{ width: `${progressPercent}%` }"></div></div><p>{{ progressPhaseLabel(taskProgress?.phase) }}</p><small v-if="taskProgress?.currentPath">{{ t("progressCurrentFile") }}: {{ basename(taskProgress.currentPath) }}</small><small v-if="taskProgress?.total">{{ t("progressCounter") }}: {{ taskProgress.current }}/{{ taskProgress.total }}</small><small v-if="displayProgressMessage">{{ displayProgressMessage }}</small></div>
        <div v-if="encodeResult" class="result-card success"><h3>{{ t("encodeDone") }}</h3><p>{{ t("output") }}: {{ encodeResult.outputRoot }}</p><p v-if="encodeResult.manifestPath">{{ t("evidenceManifest") }}: {{ encodeResult.manifestPath }}</p><div class="result-items"><article v-for="item in encodeResult.items.slice(0, 3)" :key="item.outputPath"><strong>{{ basename(item.outputPath) }}</strong><span>{{ item.mediaType === "image" ? `PSNR ${item.psnr?.toFixed(2) ?? "-"} dB` : `${item.frameCount ?? 0}` }}</span></article><article v-if="encodeResult.items.length > 3"><strong>{{ t("otherFiles") }}</strong><span>+{{ encodeResult.items.length - 3 }}</span></article></div></div>
      </article>
    </section>

    <section v-else-if="activeTab === 'decode'" class="panel panel-grid decode-grid">
      <article class="card"><div class="section-heading"><span>01</span><div><h2>{{ t("selectDecodeMedia") }}</h2><p>{{ t("selectDecodeMediaDesc") }}</p></div></div><button class="file-picker tall" @click="chooseDecodeInputs"><strong>{{ t("chooseWatermarked") }}</strong><span>{{ decodeInputs.length ? t("selectedFiles", { count: decodeInputs.length }) : t("chooseMediaClick") }}</span></button><div v-if="decodeInputs.length" class="import-summary"><div class="import-summary-head"><small>{{ t("currentImport") }}</small><button type="button" :disabled="busy" @click="clearDecodeInputs">{{ t("clearImportedFiles") }}</button></div><div class="file-list inline-list"><span v-for="name in previewNames(decodeInputs)" :key="name">{{ name }}</span><span v-if="restCount(decodeInputs)">+{{ restCount(decodeInputs) }}</span></div></div></article>
      <article class="card action-card"><div class="section-heading"><span>02</span><div><h2>{{ t("credentials") }}</h2><p>{{ t("credentialsDesc") }}</p></div></div><div class="credential-picker"><button class="file-picker" @click="chooseKeyFile"><strong>{{ t("chooseKeyFile") }}</strong><span>{{ decodeKeyFile ? basename(decodeKeyFile) : t("chooseKeyFileHint") }}</span></button><button v-if="decodeKeyFile" class="secondary-button clear-key-button" type="button" @click="clearDecodeKeyFile">{{ t("clearKeyFile") }}</button></div><label class="field"><span>{{ t("orPassword") }}</span><input v-model="decodePassword" type="password" autocomplete="current-password" :placeholder="t('passwordModeC')" /></label><label class="field range-field"><span>{{ t("decodeFrameParallelism") }}: {{ decodeFrameParallelism }}</span><input v-model.number="decodeFrameParallelism" type="range" min="1" max="8" step="1" /><small>{{ t("decodeFrameParallelismHint") }}</small></label><div class="task-actions"><button class="primary" :disabled="!canDecode || busy" @click="runDecode">{{ busy ? t("decoding") : t("startDecode") }}</button><button v-if="showProgress('decode') && busy && activeTaskId === taskProgress?.taskId" class="secondary-button cancel-button" type="button" :disabled="cancelling" @click="cancelCurrentTask">{{ cancelling ? t("cancellingTask") : t("cancelTask") }}</button></div><div v-if="showProgress('decode')" class="progress-card"><div class="progress-top"><strong>{{ t("progressTitle") }}</strong><span>{{ progressPercent }}%</span></div><div class="progress-track"><div class="progress-fill" :style="{ width: `${progressPercent}%` }"></div></div><p>{{ progressPhaseLabel(taskProgress?.phase) }}</p><small v-if="taskProgress?.currentPath">{{ t("progressCurrentFile") }}: {{ basename(taskProgress.currentPath) }}</small><small v-if="taskProgress?.total">{{ t("progressCounter") }}: {{ taskProgress.current }}/{{ taskProgress.total }}</small><small v-if="displayProgressMessage">{{ displayProgressMessage }}</small></div></article>
      <article class="card result-panel"><div class="section-heading"><span>03</span><div><h2>{{ t("detectResult") }}</h2><p>{{ t("detectResultDesc") }}</p></div></div><div v-if="decodeResult" class="result-card success decode-results"><p class="result-count">{{ t("decodeResultCount", { count: decodeResult.items.length }) }}</p><article v-for="(item, index) in decodeResult.items" :key="item.inputPath" class="decode-item"><div class="decode-title"><strong>{{ index + 1 }}. {{ basename(item.inputPath) }}</strong><span :class="['integrity', item.integrity]">{{ integrityLabel(item.integrity) }}</span></div><p class="watermark-output">{{ item.text }}</p><small class="path-line">{{ item.inputPath }}</small><small v-if="item.mediaType === 'image'">{{ t("fingerprintDistance") }} {{ item.fingerprintDistance ?? "-" }} · {{ t("correctedCodewords") }} {{ item.correctedCodewords }}</small><small v-else>{{ t("validFrames") }} {{ item.validFrames ?? 0 }}/{{ item.frameCount ?? 0 }} · {{ t("modifiedFrames") }} {{ item.modifiedFrames ?? 0 }}</small><small v-if="item.syncRegistration">{{ t("syncRegistration") }} {{ item.syncRegistration.rotationDegrees }}° / ×{{ item.syncRegistration.scale.toFixed(2) }} / {{ item.syncRegistration.score.toFixed(2) }}</small><small v-if="item.tamperRegions.length">{{ t("tamperRegions") }} {{ item.tamperRegions.length }}</small></article></div><p v-else class="empty-state">{{ t("waitingDecode") }}</p></article>
    </section>

    <section v-else-if="activeTab === 'scan'" class="panel panel-grid scan-grid">
      <article class="card action-card"><div class="section-heading"><span>01</span><div><h2>{{ t("importUnknown") }}</h2><p>{{ t("importUnknownDesc") }}</p></div></div><button class="file-picker tall" @click="chooseScanInputs"><strong>{{ t("chooseUnknown") }}</strong><span>{{ scanInputs.length ? t("selectedFiles", { count: scanInputs.length }) : t("unknownHint") }}</span></button><div v-if="scanInputs.length" class="import-summary"><div class="import-summary-head"><small>{{ t("currentImport") }}</small><button type="button" :disabled="busy" @click="clearScanInputs">{{ t("clearImportedFiles") }}</button></div><div class="file-list inline-list"><span v-for="name in previewNames(scanInputs)" :key="name">{{ name }}</span><span v-if="restCount(scanInputs)">+{{ restCount(scanInputs) }}</span></div></div><div class="task-actions"><button class="primary" :disabled="!canScan || busy" @click="runScan">{{ busy ? t("scanning") : t("scanWatermark") }}</button><button v-if="showProgress('scan') && busy && activeTaskId === taskProgress?.taskId" class="secondary-button cancel-button" type="button" :disabled="cancelling" @click="cancelCurrentTask">{{ cancelling ? t("cancellingTask") : t("cancelTask") }}</button></div><div v-if="showProgress('scan')" class="progress-card"><div class="progress-top"><strong>{{ t("progressTitle") }}</strong><span>{{ progressPercent }}%</span></div><div class="progress-track"><div class="progress-fill" :style="{ width: `${progressPercent}%` }"></div></div><p>{{ progressPhaseLabel(taskProgress?.phase) }}</p><small v-if="taskProgress?.currentPath">{{ t("progressCurrentFile") }}: {{ basename(taskProgress.currentPath) }}</small><small v-if="taskProgress?.total">{{ t("progressCounter") }}: {{ taskProgress.current }}/{{ taskProgress.total }}</small><small v-if="displayProgressMessage">{{ displayProgressMessage }}</small></div><p class="hint-text">{{ t("scanHint") }}</p></article>
      <article class="card result-panel wide-result"><div class="section-heading"><span>02</span><div><h2>{{ t("scanFeedback") }}</h2><p>{{ t("scanFeedbackDesc") }}</p></div></div><div v-if="scanResult" class="scan-results"><article v-for="item in scanResult.items" :key="item.inputPath" class="scan-item"><div class="decode-title"><strong>{{ basename(item.inputPath) }}</strong><span :class="['scan-status', item.status]">{{ item.status === "detected" ? t("foundTrace") : t("notDetected") }}</span></div><p class="scan-summary">{{ item.summary }}</p><small v-if="item.width && item.height">{{ t("imageSize") }} {{ item.width }}×{{ item.height }}</small><div v-if="item.detections.length" class="detection-list"><article v-for="detection in item.detections" :key="`${detection.detector}-${detection.label}`"><div><strong>{{ detection.label }}</strong><span>{{ detection.confidence }} · {{ detectionAccessLabel(detection) }}</span></div><p>{{ detection.content }}</p></article></div></article></div><p v-else class="empty-state">{{ t("waitingScan") }}</p></article>
    </section>

    <section v-else class="panel panel-grid ffmpeg-grid">
      <article class="card runtime-card"><div class="section-heading"><span>FF</span><div><h2>{{ t("runtimeStatus") }}</h2><p>{{ t("runtimeStatusDesc") }}</p></div></div><div :class="['runtime-overview', ffmpegStatus.className]"><div><strong>{{ ffmpegStatus.label }}</strong><span>{{ ffmpegStatus.detail }}</span></div><button class="secondary-button" type="button" @click="loadFfmpegInfo">{{ t("refresh") }}</button></div><div v-if="ffmpegInfo" class="ffmpeg-meta status-grid"><p><strong>{{ t("platform") }}</strong><span>{{ ffmpegInfo.platform }}</span></p><p><strong>{{ t("ffmpegBuild") }}</strong><span>{{ ffmpegInfo.version }}</span></p><p><strong>{{ t("utcCompileDate") }}</strong><span>{{ ffmpegInfo.utcCompileDate || t("undeclared") }}</span></p><p><strong>{{ t("licenseJudgement") }}</strong><span>{{ ffmpegInfo.buildLicense }}</span></p><p><strong>{{ t("manifestTime") }}</strong><span>{{ ffmpegInfo.generatedAt || t("notGenerated") }}</span></p><p><strong>{{ t("sourceDesc") }}</strong><span>{{ ffmpegInfo.source }}</span></p></div><div v-if="releaseInfo" class="ffmpeg-meta status-grid"><p><strong>{{ t("autoUpdate") }}</strong><span>{{ releaseInfo.automaticUpdate ? t("enabled") : t("disabled") }}</span></p><p><strong>{{ t("updateManifest") }}</strong><span>{{ releaseInfo.manifestUrl || t("notGenerated") }}</span></p><p><strong>{{ t("packageSignature") }}</strong><span>{{ releaseInfo.packageSigning.signature || t("notGenerated") }}</span></p></div><div v-else-if="ffmpegError" class="status-card error"><strong>{{ t("ffmpegResourceFailed") }}</strong><p>{{ ffmpegError }}</p></div></article>
      <article class="card result-panel"><div class="section-heading"><span>SHA</span><div><h2>{{ t("binaryHash") }}</h2><p>{{ t("binaryHashDesc") }}</p></div></div><div v-if="ffmpegBinaries.length" class="binary-list"><article v-for="binary in ffmpegBinaries" :key="binary.name" class="binary-card"><div class="decode-title"><strong>{{ binary.name }}</strong><span :class="['scan-status', binary.hashOk ? 'not_detected' : 'detected']">{{ binary.hashOk ? t("hashOk") : t("hashNeedsFix") }}</span></div><p class="mono">{{ binary.versionLine || t("versionLineMissing") }}</p><small>{{ t("path") }}: {{ binary.path }}</small><small>{{ t("expectedSha") }}: {{ binary.expectedSha256 || t("notGenerated") }}</small><small>{{ t("actualSha") }}: {{ binary.actualSha256 || t("notRead") }}</small><small v-if="binary.error" class="error-text binary-error">{{ binary.error }}</small></article></div><p v-else class="empty-state">{{ t("noCurrentBinary") }}</p></article>
      <article class="card result-panel"><div class="section-heading"><span>LIC</span><div><h2>{{ t("licenseBuild") }}</h2><p>{{ t("licenseBuildDesc") }}</p></div></div><div v-if="ffmpegInfo" class="license-box"><h3>{{ t("buildParams") }}</h3><p class="mono">{{ ffmpegInfo.buildConfigure }}</p><h3>{{ t("licenseOutput") }}</h3><pre>{{ ffmpegInfo.licenseText }}</pre></div><p v-else class="empty-state">{{ t("waitingFfmpeg") }}</p></article>
    </section>

    <div v-if="errorMessage" class="error-box" role="alert">
      <div class="error-box-head">
        <strong>{{ t("failure") }}</strong>
        <button type="button" :aria-label="t('closeFailure')" @click="errorMessage = ''">×</button>
      </div>
      <p>{{ displayErrorMessage }}</p>
    </div>

    <footer>
      <span>{{ t("footer") }}</span>
    </footer>
  </main>
</template>
