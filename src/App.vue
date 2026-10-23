<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";
import BrandMark from "./components/BrandMark.vue";
import { decodeEventSlot, decodeCountDelta } from "./taskState";
import ImportedFiles from "./components/ImportedFiles.vue";
import ResultPager from "./components/ResultPager.vue";
import TaskProgressPanel from "./components/TaskProgressPanel.vue";
import type {
  CancelTaskRequest,
  DecodeItemEvent,
  DecodeItemResult,
  DecodeRequest,
  DecodeResponse,
  DropPathResolution,
  EncodeItemResult,
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
  TimelineAnomalyRange,
} from "./types";
import { clampFailurePercent } from "./progressDisplay";

type TabId = "encode" | "decode" | "scan" | "ffmpeg";
type Language = "zh" | "en";
type DropTarget = "encode-inputs" | "output-directory" | "decode-inputs" | "key-file" | "scan-inputs";

type MessageKey = keyof typeof messages.zh;

const ENCODE_MEDIA_EXTENSIONS = ["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff", "mp4", "mov", "mkv", "avi", "webm"];
const DECODE_MEDIA_EXTENSIONS = ["png", "jpg", "jpeg", "webp", "mp4", "mov", "mkv", "avi", "webm"];
const SCAN_IMAGE_EXTENSIONS = ["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff"];
const KEY_FILE_EXTENSIONS = ["key", "json"];
const RESULT_PAGE_SIZE = 50;

const messages = {
  zh: {
    appTitle: "图隐私水印编解码器",
    eyebrow: "PRIVACY WATERMARK CODEC",
    subtitle: "不可见频域水印 · 加密载荷 · 感知篡改检测 · 本地离线处理",
    uiZoomLabel: "界面缩放",
    uiZoomOut: "缩小界面",
    uiZoomIn: "放大界面",
    uiZoomReset: "重置",
    tabEncode: "编码水印",
    tabDecode: "解码检测",
    tabScan: "未知图片扫描",
    tabFfmpeg: "FFmpeg 信息",
    mediaOutput: "媒体与输出",
    mediaOutputDesc: "单张、批量图片与常见视频。",
    chooseMedia: "选择图片或视频",
    chooseMediaHint: "点击选择，或将图片、视频或文件夹拖到此处",
    chooseOutput: "选择输出文件夹",
    chooseOutputHint: "点击选择，或将输出文件夹拖到此处",
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
    customPasswordHint: "必须不少于十二个字符",
    embedStrength: "嵌入强度",
    strengthHint: "视频不再偷偷提高到 16；PSNR<40 时逐级降到 4。仍不足请缩短文本或换画面。数值越高越抗压缩，但图像改变量也会增加。",
    frameParallelism: "视频编码并行度",
    frameParallelismHint: "仅影响视频抽帧后的水印嵌入，数值越高越占用 CPU 与磁盘 I/O。",
    decodeFrameParallelism: "视频解码并行度",
    decodeFrameParallelismHint: "这是当前设备的本地检测并行度，不依赖编码设备，也不会写入水印文件。",
    writeKey: "生成 .key 文件",
    writeKeyCustom: "关闭后仅凭密码解码",
    writeKeyRandom: "随机密钥模式必须生成密钥文件",
    keyFileWarning: ".key = 能解出水印的完整密钥，勿随图转发",
    startEncode: "开始编码",
    encoding: "正在本地处理…",
    encodeDone: "编码完成",
    output: "输出",
    evidenceManifest: "证据清单",
    otherFiles: "其余文件",
    selectDecodeMedia: "选择待检测媒体",
    selectDecodeMediaDesc: "支持共享密钥、同一密码，或为独立密钥批量选择多个 .key 文件。",
    chooseWatermarked: "选择已加水印图片或视频",
    chooseMediaClick: "点击选择，或将媒体文件或文件夹拖到此处",
    credentials: "提供解码凭据",
    credentialsDesc: "可选择一个或多个密钥文件；自定义密码与密钥文件互斥。",
    chooseKeyFile: "选择一个或多个 .key 文件",
    chooseKeyFileHint: "点击多选，或拖入 .key / JSON 密钥文件",
    orPassword: "或输入自定义密码",
    passwordModeC: "模式 C 密码",
    startDecode: "开始解码与检测",
    decoding: "正在逐帧检测…",
    detectResult: "检测结果",
    detectResultDesc: "显示水印文本与篡改判断。",
    noObviousTamper: "未发现明显篡改",
    suspiciousChange: "存在可疑变化",
    modified: "检测到修改",
    fingerprintDistance: "持钥校验码",
    correctedCodewords: "纠正码字",
    validFrames: "有效帧",
    modifiedFrames: "内容修改帧",
    missingFrames: "原始缺失帧（删除+替换）",
    insertedFrames: "插入帧",
    deletedFrames: "删除帧",
    duplicateFrames: "重复帧",
    outOfOrderFrames: "错序帧",
    replacedFrames: "替换帧",
    unreadableFrames: "未匹配/不可读帧",
    transformedFrames: "几何变换帧",
    timelineAnomalies: "时间轴异常区间",
    timelineActual: "实际",
    timelineExpected: "原始",
    timelineInsertion: "插入",
    timelineDeletion: "删除",
    timelineReplacement: "替换",
    timelineDuplicate: "重复",
    timelineOutOfOrder: "错序",
    timelineMore: "另有 {count} 个区间",
    expectedFrames: "声明总帧",
    tamperRegions: "疑似篡改分区",
    syncRegistration: "同步配准",
    moreResults: "还有 {count} 个结果已完成。",
    decodeResultCount: "已显示全部 {count} 个结果。",
    decodeStreamingCount: "已完成 {count}/{total}，结果会在每个文件处理结束后立即显示。",
    decodeFailed: "此文件未能完成解码",
    waitingDecode: "等待解码结果。",
    importUnknown: "导入未知图片",
    importUnknownDesc: "无密钥扫描本软件水印头、C2PA/AI 元数据等痕迹。",
    chooseUnknown: "选择未知来源图片",
    unknownHint: "支持多选；点击选择，或将图片或文件夹拖到此处",
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
    clearKeyFile: "清除全部密钥",
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
    progressRegisteringImage: "正在搜索图片几何配准",
    progressVerifyingImage: "正在完整验证校正图",
    progressScanningImage: "正在扫描图片",
    progressExtractingFrames: "正在抽取视频帧",
    progressProcessingFrames: "正在处理视频帧",
    progressCheckingFrames: "正在逐帧检测视频",
    progressRegisteringVideoFrames: "正在恢复视频帧几何同步",
    progressMuxingVideo: "正在封装输出视频",
    progressSummarizingVideo: "正在汇总视频结果",
    progressWritingEvidence: "正在写入证据清单",
    progressCompletedFile: "当前文件已完成",
    progressCompleted: "任务已完成",
    progressCompletedWithErrors: "任务已结束，部分文件失败",
    progressFiles: "文件",
    progressFrames: "帧",
    progressGeometry: "几何",
    progressSucceeded: "成功",
    progressFailedCount: "失败",
    selectedFiles: "已选择 {count} 个文件",
    dropRelease: "松开鼠标即可导入",
    dropAppendHint: "可继续拖入追加文件",
    dropImported: "已通过拖放导入 {accepted} 个文件。",
    dropImportedIgnored: "已导入 {accepted} 个文件，忽略 {ignored} 个不匹配项目。",
    dropNothingAccepted: "拖入内容与此选择区域不匹配。",
    dropAlreadyImported: "拖入文件已在当前列表中，无需重复添加。",
    dropOutputSet: "已将输出目录设置为：{name}",
    dropKeySet: "已载入密钥文件：{name}",
    dropKeysSet: "已载入 {count} 个密钥文件。",
    dropTruncated: "导入文件过多，已限制为前 {count} 个文件。",
    importedFromContext: "已从右键菜单导入 {count} 个文件。",
    contextEncode: "右键菜单已导入到编码页，请选择输出目录并填写水印文本。",
    contextDecode: "右键菜单已导入到解码页，请选择一个或多个密钥文件，或输入密码。",
    contextScan: "右键菜单已导入到扫描页，已自动开始无密钥扫描。",
    windowMinimize: "最小化",
    windowMaximize: "最大化",
    windowRestore: "还原窗口",
    windowClose: "关闭",
    advancedOptions: "高级设置",
    removeFile: "移除此文件",
    previousPage: "上一页",
    nextPage: "下一页",
    resultPages: "结果分页",
    importingFiles: "正在读取导入文件…",
    runningTask: "当前任务",
    returnToTask: "查看任务",
    recoveryHint: "默认几何恢复最多 10 秒；未识别的图片可单独尝试深入恢复。",
    deepRecovery: "深入恢复（最多 60 秒）",
    deepRecoveryRunning: "正在深入恢复…",
    recoveryTimedOut: "恢复已达到时间限制，可尝试深入恢复或检查原始水印图片。",
    recoveryNotFound: "未检测到有效水印，可尝试深入恢复旋转或缩放后的图片。",
    recoveryKeyMismatch: "密钥无法通过验证，请检查密钥文件或密码。",
    recoveryInvalidInput: "无法读取此媒体，请检查文件和格式。",
    recoveryPayloadCorrupted: "已识别水印结构，但正文未通过认证；可尝试深入恢复。",
    recoveryOtherError: "此文件处理失败，请查看诊断详情。",
    diagnosticDetails: "诊断详情",
    resultDetails: "检测详情",
    mirroredTransform: "已检测到镜像",
    shearTransform: "水平斜切",
    resultsSummary: "共 {count} 个结果 · 成功 {succeeded} · 失败 {failed}",
    taskElapsed: "已用 {seconds} 秒",
    scanFailed: "扫描失败",
    refreshing: "正在检测…",
  },
  en: {
    appTitle: "Privacy Watermark Codec",
    eyebrow: "PRIVACY WATERMARK CODEC",
    subtitle: "Invisible frequency-domain watermark · encrypted payload · perceptual tamper detection · local processing",
    uiZoomLabel: "UI zoom",
    uiZoomOut: "Zoom out",
    uiZoomIn: "Zoom in",
    uiZoomReset: "Reset",
    tabEncode: "Encode",
    tabDecode: "Decode",
    tabScan: "Unknown image scan",
    tabFfmpeg: "FFmpeg info",
    mediaOutput: "Media and output",
    mediaOutputDesc: "Single file, batch images, and common videos.",
    chooseMedia: "Choose images or videos",
    chooseMediaHint: "Click to choose, or drop images, videos, or folders here",
    chooseOutput: "Choose output folder",
    chooseOutputHint: "Click to choose, or drop the output folder here",
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
    customPasswordHint: "At least twelve characters are required",
    embedStrength: "Embedding strength",
    strengthHint: "Video strength is no longer silently raised to 16; if PSNR is below 40 it steps down to 4. If that is still not enough, shorten the text or pick a different clip. Higher values improve compression robustness but change pixels more.",
    frameParallelism: "Video encode parallelism",
    frameParallelismHint: "Applies after video frame extraction during embedding; higher values use more CPU and disk I/O.",
    decodeFrameParallelism: "Video decode parallelism",
    decodeFrameParallelismHint: "This is a local inspection setting for the current device; it is not embedded in the watermark file.",
    writeKey: "Write .key file",
    writeKeyCustom: "Disable this to decode only with the password",
    writeKeyRandom: "Random key modes must write a key file",
    keyFileWarning: ".key files are the full decryption secret; do not forward them with the media",
    startEncode: "Start encoding",
    encoding: "Processing locally…",
    encodeDone: "Encoding complete",
    output: "Output",
    evidenceManifest: "Evidence manifest",
    otherFiles: "Other files",
    selectDecodeMedia: "Select media to inspect",
    selectDecodeMediaDesc: "Use one shared key/password, or select multiple .key files for independently keyed media.",
    chooseWatermarked: "Choose watermarked images or videos",
    chooseMediaClick: "Click to choose, or drop media files or folders here",
    credentials: "Provide credentials",
    credentialsDesc: "Choose one or more key files, or use a mutually exclusive custom password.",
    chooseKeyFile: "Choose one or more .key files",
    chooseKeyFileHint: "Click for multiple selection, or drop .key / JSON key files here",
    orPassword: "Or enter a custom password",
    passwordModeC: "Mode C password",
    startDecode: "Decode and inspect",
    decoding: "Inspecting frame by frame…",
    detectResult: "Inspection result",
    detectResultDesc: "Shows watermark text and tamper status.",
    noObviousTamper: "No obvious tampering",
    suspiciousChange: "Suspicious change",
    modified: "Modification detected",
    fingerprintDistance: "Key-bound check code",
    correctedCodewords: "Corrected codewords",
    validFrames: "Valid frames",
    modifiedFrames: "Content-modified frames",
    missingFrames: "Missing originals (deleted + replaced)",
    insertedFrames: "Inserted frames",
    deletedFrames: "Deleted frames",
    duplicateFrames: "Duplicate frames",
    outOfOrderFrames: "Out-of-order frames",
    replacedFrames: "Replaced frames",
    unreadableFrames: "Unmatched/unreadable frames",
    transformedFrames: "Geometry-transformed frames",
    timelineAnomalies: "Timeline anomaly ranges",
    timelineActual: "actual",
    timelineExpected: "original",
    timelineInsertion: "insertion",
    timelineDeletion: "deletion",
    timelineReplacement: "replacement",
    timelineDuplicate: "duplicate",
    timelineOutOfOrder: "out of order",
    timelineMore: "{count} more range(s)",
    expectedFrames: "Declared total frames",
    tamperRegions: "Suspicious regions",
    syncRegistration: "Sync registration",
    moreResults: "{count} more result(s) completed.",
    decodeResultCount: "Showing all {count} result(s).",
    decodeStreamingCount: "Completed {count}/{total}. Each result appears as soon as its file finishes.",
    decodeFailed: "This file could not be decoded",
    waitingDecode: "Waiting for decode result.",
    importUnknown: "Import unknown images",
    importUnknownDesc: "Scan this app's watermark header, C2PA/AI metadata, and related traces without a key.",
    chooseUnknown: "Choose unknown images",
    unknownHint: "Multiple selection supported; click or drop images or folders here",
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
    clearKeyFile: "Clear all keys",
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
    progressRegisteringImage: "Searching image geometry registration",
    progressVerifyingImage: "Fully verifying a corrected image",
    progressScanningImage: "Scanning image",
    progressExtractingFrames: "Extracting video frames",
    progressProcessingFrames: "Processing video frames",
    progressCheckingFrames: "Checking video frames",
    progressRegisteringVideoFrames: "Recovering video-frame geometry",
    progressMuxingVideo: "Muxing output video",
    progressSummarizingVideo: "Summarizing video result",
    progressWritingEvidence: "Writing evidence manifest",
    progressCompletedFile: "Current file completed",
    progressCompleted: "Task completed",
    progressCompletedWithErrors: "Task finished with some files failing",
    progressFiles: "Files",
    progressFrames: "Frames",
    progressGeometry: "Geometry",
    progressSucceeded: "Succeeded",
    progressFailedCount: "Failed",
    selectedFiles: "{count} file(s) selected",
    dropRelease: "Release to import",
    dropAppendHint: "Drop more files to append",
    dropImported: "Imported {accepted} file(s) by drag and drop.",
    dropImportedIgnored: "Imported {accepted} file(s); ignored {ignored} unmatched item(s).",
    dropNothingAccepted: "The dropped content does not match this selection area.",
    dropAlreadyImported: "The dropped files are already in the current list.",
    dropOutputSet: "Output folder set to: {name}",
    dropKeySet: "Key file loaded: {name}",
    dropKeysSet: "Loaded {count} key files.",
    dropTruncated: "Too many files were dropped; only the first {count} files were imported.",
    importedFromContext: "Imported {count} file(s) from the context menu.",
    contextEncode: "Files were imported into Encode. Choose an output folder and enter watermark text.",
    contextDecode: "Files were imported into Decode. Choose one or more key files, or enter a password.",
    contextScan: "Files were imported into Scan. The keyless scan started automatically.",
    windowMinimize: "Minimize",
    windowMaximize: "Maximize",
    windowRestore: "Restore window",
    windowClose: "Close",
    advancedOptions: "Advanced settings",
    removeFile: "Remove file",
    previousPage: "Previous",
    nextPage: "Next",
    resultPages: "Result pages",
    importingFiles: "Reading imported files…",
    runningTask: "Active task",
    returnToTask: "View task",
    recoveryHint: "Standard geometry recovery runs for up to 10 seconds. Unrecognized images can be retried individually with deep recovery.",
    deepRecovery: "Deep recovery (up to 60 seconds)",
    deepRecoveryRunning: "Running deep recovery…",
    recoveryTimedOut: "Recovery reached its time limit. Try deep recovery or check the original watermarked image.",
    recoveryNotFound: "No valid watermark was found. Try deep recovery for a rotated or resized image.",
    recoveryKeyMismatch: "The key could not be authenticated. Check your key files or password.",
    recoveryInvalidInput: "This media could not be read. Check the file and its format.",
    recoveryPayloadCorrupted: "A watermark structure was found, but its payload could not be authenticated. Try deep recovery.",
    recoveryOtherError: "This file could not be processed. See the diagnostic details.",
    diagnosticDetails: "Diagnostic details",
    resultDetails: "Inspection details",
    mirroredTransform: "Mirror detected",
    shearTransform: "Horizontal shear",
    resultsSummary: "{count} results · {succeeded} succeeded · {failed} failed",
    taskElapsed: "Elapsed {seconds}s",
    scanFailed: "Scan failed",
    refreshing: "Checking…",
  },
} as const;

const UI_ZOOM_STORAGE_KEY = "pwc-ui-zoom";
const LEGACY_DEFAULT_UI_ZOOM = 1.12;
// 默认界面缩放保持 100%，避免清晰度增强和应用内缩放叠加后把三栏内容挤出卡片。
// The default UI zoom stays at 100% so readability improvements do not multiply with app zoom and push three-column content outside cards.
const DEFAULT_UI_ZOOM = 1;
const MIN_UI_ZOOM = 1;
const MAX_UI_ZOOM = 1.35;
const UI_ZOOM_STEP = 0.05;

const activeTab = ref<TabId>("encode");
const language = ref<Language>(detectLanguage());
const uiZoom = ref(readStoredUiZoom());
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
const decodeKeyFiles = ref<string[]>([]);
const isWindowMaximized = ref(false);
const decodePassword = ref("");
const busy = ref(false);
const cancelling = ref(false);
const importing = ref(false);
const ffmpegLoading = ref(false);
const activeTaskKind = ref<TaskProgressKind | null>(null);
const deepRecoveryRunning = ref(false);
const taskElapsedSeconds = ref(0);
const encodePage = ref(1);
const decodePage = ref(1);
const scanPage = ref(1);
const decodeCompleted = ref(0);
const decodeSucceeded = ref(0);
let decodeTaskSlots: number[] = [];
let decodeTaskPaths: string[] = [];
let taskClock: number | undefined;
let pendingProgress: TaskProgressEvent | undefined;
let progressAnimationFrame: number | undefined;
let eventListenersReady: Promise<void> = Promise.resolve();
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
const activeDropTarget = ref<DropTarget | null>(null);
let launchTimer: number | undefined;
let contextMessageTimer: number | undefined;
let unlistenLaunchContext: (() => void) | undefined;
let unlistenTaskProgress: (() => void) | undefined;
let unlistenDecodeItem: (() => void) | undefined;
let unlistenScaleChanged: (() => void) | undefined;
let unlistenDragDrop: (() => void) | undefined;

const inputLocked = computed(() => busy.value || importing.value);
const encodeHasVideo = computed(() => encodeInputs.value.some(isVideoPath));
const decodeHasVideo = computed(() => decodeInputs.value.some(isVideoPath));
const visibleEncodeItems = computed(() => encodeResult.value?.items.slice((encodePage.value - 1) * RESULT_PAGE_SIZE, encodePage.value * RESULT_PAGE_SIZE) || []);
const visibleScanItems = computed(() => scanResult.value?.items.slice((scanPage.value - 1) * RESULT_PAGE_SIZE, scanPage.value * RESULT_PAGE_SIZE) || []);
const visibleDecodeItems = computed(() => {
  const start = (decodePage.value - 1) * RESULT_PAGE_SIZE;
  // 分页只展开当前页，流式事件即使乱序到达也不会把空槽或上千条详情送进渲染树。
  // Pagination expands only the current page, keeping empty slots and thousands of details out of the render tree even when streaming events arrive out of order.
  return decodeResult.value?.items.slice(start, start + RESULT_PAGE_SIZE).flatMap((item, offset) => item ? [{ item, index: start + offset }] : []) || [];
});
const decodeSummary = computed(() => t("resultsSummary", {
  count: decodeCompleted.value,
  succeeded: decodeSucceeded.value,
  failed: decodeCompleted.value - decodeSucceeded.value,
}));
const activeTaskLabel = computed(() => {
  if (deepRecoveryRunning.value) return t("deepRecoveryRunning");
  if (activeTaskKind.value === "encode") return t("tabEncode");
  if (activeTaskKind.value === "scan") return t("tabScan");
  return t("tabDecode");
});
const progressPanelProps = computed(() => ({
  percent: progressPercent.value,
  title: t("progressTitle"),
  status: cancelling.value ? t("cancellingTask") : outcomeLabel(taskProgress.value?.outcome, taskProgress.value?.phase),
  message: displayProgressMessage.value,
  filesLabel: t("progressFiles"),
  succeededLabel: t("progressSucceeded"),
  failedLabel: t("progressFailedCount"),
  unitLabel: unitKindLabel(taskProgress.value?.unitKind),
}));

function isVideoPath(path: string): boolean {
  return /\.(mp4|mov|mkv|avi|webm)$/i.test(path);
}

function isTaskRunning(task: TaskProgressKind): boolean {
  return busy.value && activeTaskKind.value === task;
}

watch(contextMessage, (message) => {
  if (contextMessageTimer !== undefined) {
    window.clearTimeout(contextMessageTimer);
    contextMessageTimer = undefined;
  }
  if (!message) return;
  // 操作提示使用短暂浮层而不是占据主布局，是为了让文件导入反馈可见但不挤压三栏工作区。
  // Operation feedback uses a short-lived overlay instead of layout space so imports remain visible without pushing down the three-column workspace.
  contextMessageTimer = window.setTimeout(() => {
    contextMessage.value = "";
    contextMessageTimer = undefined;
  }, 3600);
});

function unicodeScalarCount(value: string): number {
  return Array.from(value).length;
}

const canEncode = computed(
  () =>
    encodeInputs.value.length > 0 &&
    outputDir.value.length > 0 &&
    watermarkText.value.trim().length > 0 &&
    (keyMode.value !== "custom" || unicodeScalarCount(customPassword.value) >= 12),
);

const hasDecodeCredentials = computed(() => decodeKeyFiles.value.length > 0 || unicodeScalarCount(decodePassword.value) >= 12);
const canDecode = computed(() => decodeInputs.value.length > 0 && hasDecodeCredentials.value);

const canScan = computed(() => scanInputs.value.length > 0);
const currentMessages = computed(() => messages[language.value]);
const progressPercent = computed(() =>
  clampFailurePercent(
    taskProgress.value?.percent,
    taskProgress.value?.percent,
    taskProgress.value?.outcome ?? taskProgress.value?.phase ?? "running",
  ),
);
const displayProgressMessage = computed(() => formatProgressMessage(taskProgress.value));
const encodeProgressExtra = computed(() => {
  const phase = taskProgress.value?.phase;
  if (!phase || !displayProgressMessage.value) return "";
  if (
    [
      "preparing",
      "processing-image",
      "processing-video-frames",
      "checking-video-frames",
      "decoding-image",
      "completed",
      "completed-file",
      "completed_with_errors",
      "failed",
      "cancelled",
    ].includes(phase)
  ) {
    return "";
  }
  return displayProgressMessage.value;
});
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

function normalizeUiZoom(value: number): number {
  return Math.round(Math.max(MIN_UI_ZOOM, Math.min(value, MAX_UI_ZOOM)) * 100) / 100;
}

function readStoredUiZoom(): number {
  const rawValue = window.localStorage.getItem(UI_ZOOM_STORAGE_KEY);
  const saved = Number(rawValue);
  if (Number.isFinite(saved) && Math.abs(saved - LEGACY_DEFAULT_UI_ZOOM) < 0.001) {
    // 早期清晰度增强包曾把 112% 当作默认值；升级时把这个历史默认迁移回 100%，避免旧默认继续造成内容裁切。
    // An earlier readability build used 112% as the default; migrating that historical default back to 100% prevents old settings from continuing to clip content.
    window.localStorage.setItem(UI_ZOOM_STORAGE_KEY, String(DEFAULT_UI_ZOOM));
    return DEFAULT_UI_ZOOM;
  }
  return Number.isFinite(saved) && saved > 0 ? normalizeUiZoom(saved) : DEFAULT_UI_ZOOM;
}

async function applyUiZoom(): Promise<void> {
  const zoom = normalizeUiZoom(uiZoom.value);
  uiZoom.value = zoom;

  // Linux 高 DPI 或分数缩放环境下，单纯把文字写大仍可能被 WebView/XWayland 二次缩放；直接设置 WebView 缩放可以让页面先以更大的逻辑尺寸排版，降低文字被放大后发虚的感知。
  // On Linux HiDPI or fractional-scaling setups, increasing font sizes alone can still be resampled by WebView/XWayland; setting WebView zoom lets the page lay out at a larger logical size first, reducing perceived text blur after scaling.
  try {
    await getCurrentWebview().setZoom(zoom);
  } catch {
    // 如果运行环境暂时拒绝 WebView 缩放，前端字号和对比度增强仍然生效，避免缩放失败反过来阻断应用启动。
    // If the runtime rejects WebView zoom, the stronger font sizes and contrast still apply, preventing a zoom failure from blocking app startup.
    document.documentElement.dataset.webviewZoom = "unavailable";
  }
}

function setUiZoom(nextZoom: number) {
  const zoom = normalizeUiZoom(nextZoom);
  uiZoom.value = zoom;
  window.localStorage.setItem(UI_ZOOM_STORAGE_KEY, String(zoom));
  void applyUiZoom();
}

function resetUiZoom() {
  setUiZoom(DEFAULT_UI_ZOOM);
}

async function syncDisplayScaleInfo(): Promise<void> {
  const scaleFactor = await getCurrentWindow().scaleFactor();

  // 这里只把系统缩放因子记录到 DOM 属性中供 CSS/调试判断使用，不再参与布局计算，避免系统 DPI 与应用内部缩放互相叠加。
  // The system scale factor is only exposed as a DOM attribute for CSS/debugging and is not mixed into layout math, preventing system DPI and app zoom from multiplying each other.
  document.documentElement.dataset.scaleFactor = scaleFactor.toFixed(2);
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

function encodeItemStatus(item: EncodeItemResult): string {
  if (item.succeeded === false) {
    return item.error || t("progressFailed");
  }
  if (item.mediaType === "image") {
    return `PSNR ${item.psnr?.toFixed(2) ?? "-"} dB`;
  }
  return `${item.frameCount ?? 0}`;
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
  const geometry = message.match(/候选\s*(\d+)\/(\d+)，旋转\s*([+\-0-9.]+)°，缩放\s*×([0-9.]+)/);
  const verify = message.match(/完整验证第\s*(\d+)\/(\d+)\s*张校正图，旋转\s*([+\-0-9.]+)°，缩放\s*×([0-9.]+)/);
  const videoVerify = message.match(/完整验证第\s*(\d+)\s*帧第\s*(\d+)\/(\d+)\s*张校正图，旋转\s*([+\-0-9.]+)°，缩放\s*×([0-9.]+)/);
  const geometryWorkers = message.match(/CPU\s*(\d+)\s*线程/)?.[1];
  // 英文界面直接从结构化数值重建几何进度，是因为后端阶段标签会持续扩展，逐个翻译中文阶段名既容易遗漏，也会让新增算法在英文界面回退成中文。
  // The English UI rebuilds geometry progress from structured numbers because backend stage labels keep evolving; translating every Chinese stage name is easy to miss and would make new algorithms fall back to Chinese.
  const geometryDetail = geometry
    ? `candidate ${geometry[1]}/${geometry[2]}, rotation ${geometry[3]}°, scale ×${geometry[4]}${geometryWorkers ? `, CPU ${geometryWorkers} threads` : ""}`
    : "";
  const verifyDetail = verify
    ? `${verify[1]}/${verify[2]}, rotation ${verify[3]}°, scale ×${verify[4]}`
    : "";
  const videoVerifyDetail = videoVerify
    ? `frame ${videoVerify[1]}, corrected image ${videoVerify[2]}/${videoVerify[3]}, rotation ${videoVerify[4]}°, scale ×${videoVerify[5]}`
    : "";

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
    case "registering-image":
      return geometryDetail ? `Searching image registration: ${geometryDetail}` : localizeRuntimeMessage(message, progress.phase);
    case "verifying-image":
      return verifyDetail ? `Scoring finished, fully verifying corrected image ${verifyDetail}` : localizeRuntimeMessage(message, progress.phase);
    case "scanning-image":
      return counter ? `Scanning image: ${counter}` : "Scanning image";
    case "extracting-video-frames":
      return `Extracting video frames${stagePart}${generatedPart}${elapsedPart}`;
    case "processing-video-frames":
      return counter ? `Embedding video-frame watermark: ${counter}` : "Embedding video-frame watermark";
    case "checking-video-frames":
      return counter ? `Decoding and inspecting video frames: ${counter}` : "Decoding and inspecting video frames";
    case "registering-video-frames":
      if (videoVerifyDetail) {
        return `Scoring finished, fully verifying ${videoVerifyDetail}`;
      }
      return geometryDetail ? `Recovering video-frame geometry: ${geometryDetail}` : localizeRuntimeMessage(message, progress.phase);
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
    case "completed_with_errors":
      return "Task finished with some files failing";
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
    [/正在搜索图片几何配准[:：]候选\s*(\d+)\/(\d+)，旋转\s*([+\-0-9.]+)°，缩放\s*×([0-9.]+)/g, "Searching image registration: candidate $1/$2, rotation $3°, scale ×$4"],
    [/评分已结束，正在完整验证第\s*(\d+)\/(\d+)\s*张校正图，旋转\s*([+\-0-9.]+)°，缩放\s*×([0-9.]+)/g, "Scoring finished, fully verifying corrected image $1/$2, rotation $3°, scale ×$4"],
    [/评分已结束，正在完整验证第\s*(\d+)\s*帧第\s*(\d+)\/(\d+)\s*张校正图，旋转\s*([+\-0-9.]+)°，缩放\s*×([0-9.]+)/g, "Scoring finished, fully verifying frame $1 corrected image $2/$3, rotation $4°, scale ×$5"],
    [/正在恢复第\s*(\d+)\s*帧的几何同步[:：]候选\s*(\d+)\/(\d+)，旋转\s*([+\-0-9.]+)°，缩放\s*×([0-9.]+)/g, "Recovering geometry for frame $1: candidate $2/$3, rotation $4°, scale ×$5"],
    [/未检测到有效水印头；图片可能没有使用本软件编码，或旋转后铺满裁切\/严重压缩破坏了块周期。无白边的小角度旋转铺满裁切可能对不齐网格；旧一级高频在旋转插值后也可能无法恢复/g, "No valid watermark header was found. The image may not have been encoded with this app, or a rotate-then-fill-crop / heavy compression broke the block period. A small-angle fill-crop without white borders can miss the grid; a legacy first-level high-frequency encoding may also fail after rotation interpolation"],
    [/批量解码检测任务完成/g, "Batch decode task completed"],
    [/批量解码检测任务结束，部分文件失败/g, "Batch decode finished with some files failing"],
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

function timelineAnomalyKindLabel(kind: TimelineAnomalyRange["kind"]): string {
  const labels: Record<TimelineAnomalyRange["kind"], MessageKey> = {
    insertion: "timelineInsertion",
    deletion: "timelineDeletion",
    replacement: "timelineReplacement",
    duplicate: "timelineDuplicate",
    out_of_order: "timelineOutOfOrder",
  };
  return t(labels[kind]);
}

function timelineFrameRange(start?: number, end?: number): string {
  if (!start) return "-";
  return end && end !== start ? `${start}-${end}` : String(start);
}

function timelineAnomalySummary(ranges: TimelineAnomalyRange[]): string {
  const visible = ranges.slice(0, 8).map((range) => {
    const kind = timelineAnomalyKindLabel(range.kind);
    const actual = range.physicalStartFrame
      ? `${t("timelineActual")} ${timelineFrameRange(range.physicalStartFrame, range.physicalEndFrame)}`
      : "";
    const expected = range.expectedStartFrame
      ? `${t("timelineExpected")} ${timelineFrameRange(range.expectedStartFrame, range.expectedEndFrame)}`
      : "";
    return `${kind} ${[actual, expected].filter(Boolean).join(" / ")}`.trim();
  });
  if (ranges.length > visible.length) {
    visible.push(t("timelineMore", { count: ranges.length - visible.length }));
  }
  return visible.join(" · ");
}

function detectionAccessLabel(detection: ScanDetection): string {
  return detection.needsKey ? t("needKey") : t("readable");
}


function createTaskId(task: TaskProgressKind): string {
  return `${task}-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

function beginTaskProgress(task: TaskProgressKind, total: number): string {
  const taskId = createTaskId(task);
  activeTaskKind.value = task;
  activeTaskId.value = taskId;
  pendingProgress = undefined;
  if (progressAnimationFrame !== undefined) window.cancelAnimationFrame(progressAnimationFrame);
  progressAnimationFrame = undefined;
  taskElapsedSeconds.value = 0;
  const startedAt = performance.now();
  if (taskClock !== undefined) window.clearInterval(taskClock);
  taskClock = window.setInterval(() => {
    taskElapsedSeconds.value = Math.floor((performance.now() - startedAt) / 1000);
  }, 1000);
  taskProgress.value = {
    taskId,
    task,
    phase: "queued",
    message: t("progressQueued"),
    current: 0,
    total,
    percent: 0,
    outcome: "running",
  };
  return taskId;
}

function finishTask() {
  // 命令结束后丢弃尚未刷新的运行事件，避免最后一帧把完成或取消状态重新覆盖成运行中。
  // Queued running events are discarded after the command settles so a final animation frame cannot overwrite completion or cancellation with a running state.
  pendingProgress = undefined;
  if (progressAnimationFrame !== undefined) window.cancelAnimationFrame(progressAnimationFrame);
  progressAnimationFrame = undefined;
  if (taskClock !== undefined) window.clearInterval(taskClock);
  taskClock = undefined;
  busy.value = false;
  cancelling.value = false;
  deepRecoveryRunning.value = false;
}

function completeTaskProgress(total: number, failed: number) {
  if (!taskProgress.value) return;
  const outcome = failed ? "completed_with_errors" : "completed";
  taskProgress.value = {
    ...taskProgress.value,
    outcome,
    phase: outcome,
    message: t(failed ? "progressCompletedWithErrors" : "progressCompleted"),
    current: total,
    total,
    percent: 100,
    filesSucceeded: total - failed,
    filesFailed: failed,
    unitCurrent: undefined,
    unitTotal: undefined,
    unitLabel: undefined,
  };
}

function acceptTaskProgress(progress: TaskProgressEvent) {
  if (!busy.value || progress.taskId !== activeTaskId.value || progress.task !== activeTaskKind.value) return;
  // 同一绘制帧只应用最新进度，避免候选评分的密集事件反复触发整页计算；终态则立即显示。
  // Only the latest progress in a paint frame is applied, avoiding repeated page work from dense candidate events; terminal states are displayed immediately.
  pendingProgress = progress;
  const flush = () => {
    progressAnimationFrame = undefined;
    const incoming = pendingProgress;
    pendingProgress = undefined;
    if (!incoming || incoming.taskId !== activeTaskId.value) return;
    taskProgress.value = {
      ...incoming,
      percent: clampFailurePercent(taskProgress.value?.percent, incoming.percent, incoming.outcome ?? incoming.phase),
    };
  };
  if (progress.outcome && progress.outcome !== "running") {
    if (progressAnimationFrame !== undefined) window.cancelAnimationFrame(progressAnimationFrame);
    flush();
  } else if (progressAnimationFrame === undefined) {
    progressAnimationFrame = window.requestAnimationFrame(flush);
  }
}

function showProgress(task: TaskProgressKind): boolean {
  return taskProgress.value?.task === task;
}

async function cancelCurrentTask() {
  if (!activeTaskId.value || !busy.value || cancelling.value) return;
  cancelling.value = true;
  const request: CancelTaskRequest = { taskId: activeTaskId.value };
  // 取消请求不等于后台已经退出；独立显示等待状态才能保留真实进度并阻止重复启动。
  // A cancellation request does not mean the worker has exited; a separate waiting state preserves real progress and prevents a duplicate start.
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
    "registering-image": "progressRegisteringImage",
    "verifying-image": "progressVerifyingImage",
    "scanning-image": "progressScanningImage",
    "extracting-video-frames": "progressExtractingFrames",
    "processing-video-frames": "progressProcessingFrames",
    "checking-video-frames": "progressCheckingFrames",
    "registering-video-frames": "progressRegisteringVideoFrames",
    "muxing-video": "progressMuxingVideo",
    "summarizing-video": "progressSummarizingVideo",
    "writing-evidence": "progressWritingEvidence",
    "completed-file": "progressCompletedFile",
    completed: "progressCompleted",
    completed_with_errors: "progressCompletedWithErrors",
    cancelled: "progressCancelled",
    failed: "progressFailed",
  };
  return t(labels[phase || "queued"] || "progressPreparing");
}

function outcomeLabel(outcome?: string, phase?: string): string {
  if (outcome === "completed_with_errors") {
    return t("progressCompletedWithErrors");
  }
  if (outcome === "failed") {
    return t("progressFailed");
  }
  if (outcome === "cancelled") {
    return t("progressCancelled");
  }
  if (outcome === "completed") {
    return t("progressCompleted");
  }
  return progressPhaseLabel(phase);
}

function unitKindLabel(kind?: string): string {
  if (kind === "frame") {
    return t("progressFrames");
  }
  if (kind === "geometry") {
    return t("progressGeometry");
  }
  return t("progressFiles");
}


function handleTaskError(error: unknown) {
  const message = String(error);
  if (message.includes("任务已取消") || message.toLowerCase().includes("cancel")) {
    if (taskProgress.value) {
      taskProgress.value = {
        ...taskProgress.value,
        phase: "cancelled",
        message: t("progressCancelled"),
        outcome: "cancelled",
        percent: clampFailurePercent(taskProgress.value.percent, taskProgress.value.percent, "cancelled"),
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
      outcome: "failed",
      percent: clampFailurePercent(taskProgress.value.percent, taskProgress.value.percent, "failed"),
    };
  }
}

function pickerHint(paths: string[], emptyHint: MessageKey): string {
  return paths.length
    ? `${t("selectedFiles", { count: paths.length })} · ${t("dropAppendHint")}`
    : t(emptyHint);
}

function mergeUniquePaths(current: string[], incoming: string[]): string[] {
  const merged = current.slice();
  // 只有 Windows 的文件系统路径按大小写去重，其它平台必须保留同名大小写不同的独立文件。
  // Only Windows filesystem paths are deduplicated without case; other platforms must retain distinct files whose names differ by case.
  const identity = (path: string) => navigator.platform.startsWith("Win") ? path.toLowerCase() : path;
  const seen = new Set(current.map(identity));
  for (const path of incoming) {
    const key = identity(path);
    if (!seen.has(key)) {
      seen.add(key);
      merged.push(path);
    }
  }
  return merged;
}

function dropTargetAtPosition(position: { x: number; y: number }): DropTarget | null {
  const scaleFactor = Number(document.documentElement.dataset.scaleFactor || window.devicePixelRatio || 1);
  const candidatePoints = [
    { x: position.x / Math.max(scaleFactor, 0.1), y: position.y / Math.max(scaleFactor, 0.1) },
    { x: position.x, y: position.y },
  ];

  // Tauri 在桌面端报告物理坐标，但不同 WebView/缩放组合可能已经转换为逻辑坐标；依次尝试两种坐标可避免高 DPI 下高亮错位。
  // Tauri normally reports physical coordinates, while some WebView/zoom combinations already expose logical coordinates; trying both prevents misplaced highlights on HiDPI displays.
  for (const point of candidatePoints) {
    const element = document.elementsFromPoint(point.x, point.y)
      .map((item) => item.closest<HTMLElement>("[data-drop-target]"))
      .find((item): item is HTMLElement => Boolean(item));
    const target = element?.dataset.dropTarget as DropTarget | undefined;
    if (target) return target;
  }
  return null;
}

async function resolveDroppedPaths(
  paths: string[],
  allowedExtensions: string[],
  expandDirectories: boolean,
): Promise<DropPathResolution> {
  return invoke<DropPathResolution>("resolve_drop_paths", {
    request: { paths, allowedExtensions, expandDirectories },
  });
}

function showDropImportFeedback(accepted: number, ignored: number, truncated: boolean) {
  if (accepted === 0) {
    errorMessage.value = t("dropNothingAccepted");
    return;
  }
  contextMessage.value = ignored > 0
    ? t("dropImportedIgnored", { accepted, ignored })
    : t("dropImported", { accepted });
  if (truncated) {
    contextMessage.value = `${contextMessage.value} ${t("dropTruncated", { count: accepted })}`;
  }
}

async function applyDroppedPaths(target: DropTarget, paths: string[]) {
  if (inputLocked.value || paths.length === 0) return;
  importing.value = true;
  try {
    await importDroppedPaths(target, paths);
  } finally {
    importing.value = false;
  }
}

async function importDroppedPaths(target: DropTarget, paths: string[]) {
  errorMessage.value = "";

  if (target === "output-directory") {
    const resolved = await resolveDroppedPaths(paths, [], false);
    const directory = resolved.directories[0];
    if (!directory) {
      errorMessage.value = t("dropNothingAccepted");
      return;
    }
    outputDir.value = directory;
    contextMessage.value = t("dropOutputSet", { name: basename(directory) });
    return;
  }

  if (target === "key-file") {
    const resolved = await resolveDroppedPaths(paths, KEY_FILE_EXTENSIONS, false);
    if (resolved.files.length === 0) {
      errorMessage.value = t("dropNothingAccepted");
      return;
    }
    const previousCount = decodeKeyFiles.value.length;
    decodeKeyFiles.value = mergeUniquePaths(decodeKeyFiles.value, resolved.files);
    const addedCount = decodeKeyFiles.value.length - previousCount;
    decodePassword.value = "";
    // 密钥拖放采用追加而非覆盖，是为了让独立密钥模式的多张图片可在一次任务中自动按 salt 匹配。
    // Key-file drops append instead of replacing so independently keyed images can be matched by salt in one batch task.
    contextMessage.value = addedCount === 1
      ? t("dropKeySet", { name: basename(resolved.files[0]) })
      : t("dropKeysSet", { count: addedCount });
    return;
  }

  const policy = target === "encode-inputs"
    ? { extensions: ENCODE_MEDIA_EXTENSIONS, destination: encodeInputs }
    : target === "decode-inputs"
      ? { extensions: DECODE_MEDIA_EXTENSIONS, destination: decodeInputs }
      : { extensions: SCAN_IMAGE_EXTENSIONS, destination: scanInputs };
  const resolved = await resolveDroppedPaths(paths, policy.extensions, true);
  const previousCount = policy.destination.value.length;
  policy.destination.value = mergeUniquePaths(policy.destination.value, resolved.files);
  const addedCount = policy.destination.value.length - previousCount;
  if (addedCount === 0 && resolved.files.length > 0) {
    contextMessage.value = t("dropAlreadyImported");
    return;
  }
  showDropImportFeedback(addedCount, resolved.ignoredCount, resolved.truncated);
}

async function registerNativeDragDrop() {
  // 使用 Tauri 原生拖放事件才能取得真实文件系统路径；浏览器 File 对象不会可靠暴露桌面绝对路径，无法交给 Rust 后端处理。
  // Tauri's native drag-drop event is required to obtain real filesystem paths; browser File objects do not reliably expose desktop absolute paths for the Rust backend.
  unlistenDragDrop = await getCurrentWebview().onDragDropEvent((event) => {
    const payload = event.payload;
    if (inputLocked.value) {
      activeDropTarget.value = null;
      return;
    }
    if (payload.type === "over") {
      activeDropTarget.value = dropTargetAtPosition(payload.position);
      return;
    }
    if (payload.type === "drop") {
      const target = dropTargetAtPosition(payload.position) || activeDropTarget.value;
      activeDropTarget.value = null;
      if (target) {
        void applyDroppedPaths(target, payload.paths).catch((error) => {
          errorMessage.value = String(error);
        });
      }
      return;
    }
    activeDropTarget.value = null;
  });
}

async function chooseEncodeInputs() {
  const selected = await open({
    multiple: true,
    directory: false,
    filters: [
      {
        name: language.value === "zh" ? "图片与视频" : "Images and videos",
        extensions: ENCODE_MEDIA_EXTENSIONS,
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
        extensions: DECODE_MEDIA_EXTENSIONS,
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
        extensions: SCAN_IMAGE_EXTENSIONS,
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
    multiple: true,
    directory: false,
    filters: [{ name: language.value === "zh" ? "水印密钥" : "Watermark keys", extensions: KEY_FILE_EXTENSIONS }],
  });
  if (selected) {
    const paths = Array.isArray(selected) ? selected : [selected];
    decodeKeyFiles.value = mergeUniquePaths(decodeKeyFiles.value, paths);
    decodePassword.value = "";
  }
}

function clearDecodeKeyFile() {
  decodeKeyFiles.value = [];
}

watch(decodePassword, (value) => {
  if (value.length > 0 && decodeKeyFiles.value.length > 0) {
    // 密码输入和 Key 文件互斥，避免界面同时显示两种凭据而用户无法判断实际使用哪一种。
    // Password input and key files are mutually exclusive so users do not see two credentials without knowing which one is used.
    decodeKeyFiles.value = [];
  }
});


async function minimizeWindow() {
  await getCurrentWindow().minimize();
}

async function toggleWindowMaximize() {
  const window = getCurrentWindow();
  await window.toggleMaximize();
  isWindowMaximized.value = await window.isMaximized();
}

async function closeWindow() {
  await getCurrentWindow().close();
}

async function syncWindowState() {
  isWindowMaximized.value = await getCurrentWindow().isMaximized();
}

async function runEncode() {
  if (!canEncode.value || inputLocked.value) return;
  busy.value = true;
  errorMessage.value = "";
  encodeResult.value = null;
  await eventListenersReady;
  const taskId = beginTaskProgress("encode", encodeInputs.value.length);

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
    encodePage.value = 1;
    completeTaskProgress(encodeResult.value.items.length, encodeResult.value.items.filter((item) => item.succeeded === false).length);
  } catch (error) {
    handleTaskError(error);
  } finally {
    finishTask();
  }
}

function mergeStreamingDecodeItem(event: DecodeItemEvent) {
  if (event.taskId !== activeTaskId.value || !busy.value || !decodeResult.value) return;
  const index = decodeEventSlot(event, activeTaskId.value, decodeTaskPaths, decodeTaskSlots);
  if (index === undefined) return;
  const previous = decodeResult.value.items[index];
  // 原位更新稳定槽位，单项重试仍映射原批次位置，避免列表复制和错误覆盖首项。
  // Update stable slots in place; single-item retries retain their batch position without copying the list or overwriting the first item.
  const delta = decodeCountDelta(previous, event.item);
  decodeCompleted.value += delta.completed;
  decodeSucceeded.value += delta.succeeded;
  decodeResult.value.items[index] = event.item;
}

function decodeErrorLabel(item: DecodeItemResult): string {
  if (item.errorCode === "recovery_timeout") return t("recoveryTimedOut");
  if (item.errorCode === "header_not_found") return t("recoveryNotFound");
  if (item.errorCode === "payload_corrupted") return t("recoveryPayloadCorrupted");
  if (item.errorCode === "invalid_input") return t("recoveryInvalidInput");
  if (item.errorCode === "authentication_failed") return t("recoveryKeyMismatch");
  return item.error || t("recoveryOtherError");
}

async function retryDeepRecovery(index: number) {
  const item = decodeResult.value?.items[index];
  if (!item || inputLocked.value || !hasDecodeCredentials.value) return;
  busy.value = true;
  deepRecoveryRunning.value = true;
  decodeTaskSlots = [index];
  decodeTaskPaths = [item.inputPath];
  await eventListenersReady;
  const taskId = beginTaskProgress("decode", 1);
  try {
    const response = await invoke<DecodeResponse>("decode_media", { request: {
      inputPaths: [item.inputPath], keyFiles: decodeKeyFiles.value.length ? decodeKeyFiles.value : undefined,
      customPassword: decodePassword.value || undefined, frameParallelism: decodeFrameParallelism.value,
      recoveryMode: "deep", taskId,
    } satisfies DecodeRequest });
    if (response.items[0]) mergeStreamingDecodeItem({ taskId, current: 1, total: 1, item: response.items[0] });
    completeTaskProgress(1, response.items.filter((value) => !value.succeeded).length);
  } catch (error) { handleTaskError(error); }
  finally { finishTask(); }
}

async function runDecode() {
  if (!canDecode.value || inputLocked.value) return;
  busy.value = true;
  errorMessage.value = "";
  decodeResult.value = { items: [] };
  decodeCompleted.value = 0;
  decodeSucceeded.value = 0;
  decodePage.value = 1;
  decodeTaskSlots = decodeInputs.value.map((_, index) => index);
  decodeTaskPaths = decodeInputs.value.slice();
  await eventListenersReady;
  const taskId = beginTaskProgress("decode", decodeInputs.value.length);

  const request: DecodeRequest = {
    inputPaths: decodeInputs.value,
    keyFiles: decodeKeyFiles.value.length ? decodeKeyFiles.value : undefined,
    customPassword: decodePassword.value || undefined,
    frameParallelism: decodeFrameParallelism.value,
    taskId,
  };

  try {
    const response = await invoke<DecodeResponse>("decode_media", { request });
    if (activeTaskId.value === taskId) {
      // 最终响应作为权威快照覆盖流式列表，是为了补齐极端情况下漏到前端的事件，同时保持后端输入顺序。
      // The final response replaces the streamed list as an authoritative snapshot, recovering any event missed by the frontend while preserving backend input order.
      decodeResult.value = response;
      decodeCompleted.value = response.items.length;
      decodeSucceeded.value = response.items.filter((item) => item.succeeded).length;
      completeTaskProgress(response.items.length, response.items.length - decodeSucceeded.value);
    }
  } catch (error) {
    handleTaskError(error);
  } finally {
    finishTask();
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
  if (!canScan.value || inputLocked.value) return;
  busy.value = true;
  errorMessage.value = "";
  scanResult.value = null;
  await eventListenersReady;
  const taskId = beginTaskProgress("scan", scanInputs.value.length);

  const request: ScanRequest = {
    inputPaths: scanInputs.value,
    taskId,
  };

  try {
    scanResult.value = await invoke<ScanResponse>("scan_privacy_watermark", { request });
    scanPage.value = 1;
    completeTaskProgress(scanResult.value.items.length, scanResult.value.items.filter((item) => item.error).length);
  } catch (error) {
    handleTaskError(error);
  } finally {
    finishTask();
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
  void applyUiZoom();
  void syncDisplayScaleInfo();
  void syncWindowState();
  void getCurrentWindow().onScaleChanged(({ payload }) => {
    document.documentElement.dataset.scaleFactor = payload.scaleFactor.toFixed(2);
    void applyUiZoom();
  }).then((unlisten) => {
    unlistenScaleChanged = unlisten;
  });
  void applyInitialLaunchContext();
  void loadReleaseInfo();
  void registerNativeDragDrop().catch((error) => {
    errorMessage.value = String(error);
  });
  void listen<LaunchContext>("pwc-launch-context", (event) => {
    mergeLaunchContext(event.payload);
  }).then((unlisten) => {
    unlistenLaunchContext = unlisten;
  });
  eventListenersReady = Promise.all([
    listen<DecodeItemEvent>("pwc-decode-item", (event) => mergeStreamingDecodeItem(event.payload)).then((unlisten) => { unlistenDecodeItem = unlisten; }),
    listen<TaskProgressEvent>("pwc-task-progress", (event) => acceptTaskProgress(event.payload)).then((unlisten) => { unlistenTaskProgress = unlisten; }),
  ]).then(() => undefined).catch(() => {
    // 实时事件不可用时仍允许命令返回最终结果，避免监听注册失败把任务永久锁在准备状态。
    // Final command results remain available if live events fail, preventing listener registration from locking tasks in preparation forever.
  });
  window.addEventListener("resize", updateViewportScale);
  window.visualViewport?.addEventListener("resize", updateViewportScale);
});

onBeforeUnmount(() => {
  finishTask();
  if (launchTimer !== undefined) window.clearTimeout(launchTimer);
  if (contextMessageTimer !== undefined) window.clearTimeout(contextMessageTimer);
  unlistenLaunchContext?.();
  unlistenTaskProgress?.();
  unlistenDecodeItem?.();
  unlistenScaleChanged?.();
  unlistenDragDrop?.();
  window.removeEventListener("resize", updateViewportScale);
  window.visualViewport?.removeEventListener("resize", updateViewportScale);
});
</script>

<template>
  <main :class="['shell', `lang-${language}`]">
    <header class="hero" data-tauri-drag-region>
      <div class="hero-copy hero-branding" data-tauri-drag-region>
        <BrandMark />
        <div>
          <p class="eyebrow">{{ t("eyebrow") }}</p>
          <h1>{{ t("appTitle") }}</h1>
          <p class="subtitle">{{ t("subtitle") }}</p>
        </div>
      </div>
      <div class="hero-side">
        <div class="window-controls" aria-label="Window controls">
          <button class="minimize-window" type="button" :aria-label="t('windowMinimize')" :title="t('windowMinimize')" @click="minimizeWindow"><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3.5 8h9" /></svg></button>
          <button type="button" :aria-label="isWindowMaximized ? t('windowRestore') : t('windowMaximize')" :title="isWindowMaximized ? t('windowRestore') : t('windowMaximize')" @click="toggleWindowMaximize"><svg v-if="isWindowMaximized" viewBox="0 0 16 16" aria-hidden="true"><rect x="3.5" y="5.5" width="7" height="7" rx="1" /><path d="M5.5 5.5V4.5a1 1 0 0 1 1-1h5a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1h-1" /></svg><svg v-else viewBox="0 0 16 16" aria-hidden="true"><rect x="3.5" y="3.5" width="9" height="9" rx="1.2" /></svg></button>
          <button class="close-window" type="button" :aria-label="t('windowClose')" :title="t('windowClose')" @click="closeWindow"><svg viewBox="0 0 16 16" aria-hidden="true"><path d="m4 4 8 8M12 4l-8 8" /></svg></button>
        </div>
        <div class="hero-preferences">
          <div class="language-switch" aria-label="Language switch">
            <button :class="{ active: language === 'zh' }" @click="setLanguage('zh')">中</button>
            <button :class="{ active: language === 'en' }" @click="setLanguage('en')">EN</button>
          </div>
          <div class="zoom-control" :aria-label="t('uiZoomLabel')">
            <span>{{ t("uiZoomLabel") }} {{ Math.round(uiZoom * 100) }}%</span>
            <button type="button" :aria-label="t('uiZoomOut')" :disabled="uiZoom <= MIN_UI_ZOOM" @click="setUiZoom(uiZoom - UI_ZOOM_STEP)">−</button>
            <button type="button" @click="resetUiZoom">{{ t("uiZoomReset") }}</button>
            <button type="button" :aria-label="t('uiZoomIn')" :disabled="uiZoom >= MAX_UI_ZOOM" @click="setUiZoom(uiZoom + UI_ZOOM_STEP)">+</button>
          </div>
        </div>
      </div>
    </header>

    <nav class="tabs" aria-label="Function tabs">
      <button :class="{ active: activeTab === 'encode' }" @click="selectTab('encode')">{{ t("tabEncode") }}</button>
      <button :class="{ active: activeTab === 'decode' }" @click="selectTab('decode')">{{ t("tabDecode") }}</button>
      <button :class="{ active: activeTab === 'scan' }" @click="selectTab('scan')">{{ t("tabScan") }}</button>
      <button :class="{ active: activeTab === 'ffmpeg' }" @click="selectTab('ffmpeg')">{{ t("tabFfmpeg") }}</button>
    </nav>

    <section v-if="taskProgress" class="global-task-bar" :aria-label="activeTaskLabel">
      <div class="global-task-heading"><strong>{{ activeTaskLabel }}</strong><span>{{ taskElapsedSeconds }}s</span><button v-if="busy" type="button" class="secondary-button" :disabled="cancelling" @click="cancelCurrentTask">{{ cancelling ? t("cancellingTask") : t("cancelTask") }}</button></div>
      <TaskProgressPanel :progress="taskProgress" v-bind="progressPanelProps" />
    </section>
    <p v-if="activeTab === 'decode'" class="recovery-hint">{{ t("recoveryHint") }}</p>
    <p v-if="importing" role="status">{{ t("importingFiles") }}</p>
    <Teleport to="body">
      <Transition name="context-toast">
        <div v-if="contextMessage" class="context-banner" role="status" aria-live="polite">
          <span>{{ contextMessage }}</span>
          <button type="button" :aria-label="t('windowClose')" @click="contextMessage = ''">×</button>
        </div>
      </Transition>
    </Teleport>

    <section v-if="activeTab === 'encode'" class="panel panel-grid encode-grid">
      <article class="card">
        <div class="section-heading"><span>01</span><div><h2>{{ t("mediaOutput") }}</h2><p>{{ t("mediaOutputDesc") }}</p></div></div>
        <button :class="['file-picker', { 'drag-active': activeDropTarget === 'encode-inputs' }]" type="button" data-drop-target="encode-inputs" :disabled="inputLocked" @click="chooseEncodeInputs"><strong>{{ t("chooseMedia") }}</strong><span>{{ activeDropTarget === "encode-inputs" ? t("dropRelease") : pickerHint(encodeInputs, "chooseMediaHint") }}</span></button>
        <ImportedFiles v-model:paths="encodeInputs" :disabled="inputLocked" :title="t('currentImport')" :clear-label="t('clearImportedFiles')" :remove-label="t('removeFile')" :previous-label="t('previousPage')" :next-label="t('nextPage')" />
        <button :class="['file-picker', { 'drag-active': activeDropTarget === 'output-directory' }]" type="button" data-drop-target="output-directory" :disabled="inputLocked" @click="chooseOutputDir"><strong>{{ t("chooseOutput") }}</strong><span>{{ activeDropTarget === "output-directory" ? t("dropRelease") : (outputDir || t("chooseOutputHint")) }}</span></button>
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
        <details class="advanced-settings"><summary>{{ t("advancedOptions") }}</summary><label class="field range-field"><span>{{ t("embedStrength") }}: {{ strength }}</span><input v-model.number="strength" type="range" min="3" max="20" step="1" /><small>{{ t("strengthHint") }}</small></label>
        <label v-if="encodeHasVideo" class="field range-field"><span>{{ t("frameParallelism") }}: {{ frameParallelism }}</span><input v-model.number="frameParallelism" type="range" min="1" max="8" step="1" /><small>{{ t("frameParallelismHint") }}</small></label></details>
        <label class="toggle-row"><input v-model="writeKeyFile" type="checkbox" :disabled="keyMode !== 'custom'" /><span><strong>{{ t("writeKey") }}</strong><small>{{ keyMode === "custom" ? t("writeKeyCustom") : t("writeKeyRandom") }}</small></span></label>
        <small class="field-warning">{{ t("keyFileWarning") }}</small>
        <div class="task-actions">
          <button class="primary" :disabled="!canEncode || inputLocked" @click="runEncode">{{ isTaskRunning('encode') ? t("encoding") : t("startEncode") }}</button>
          <button v-if="showProgress('encode') && busy && activeTaskId === taskProgress?.taskId" class="secondary-button cancel-button" type="button" :disabled="cancelling" @click="cancelCurrentTask">{{ cancelling ? t("cancellingTask") : t("cancelTask") }}</button>
        </div>
        <div v-if="encodeResult" :class="['result-card', encodeResult.items.some((item) => item.succeeded === false) ? '' : 'success']">
          <h3>{{ encodeResult.items.some((item) => item.succeeded === false) ? t("progressCompletedWithErrors") : t("encodeDone") }}</h3>
          <p>{{ t("output") }}: {{ encodeResult.outputRoot }}</p>
          <p v-if="encodeResult.manifestPath">{{ t("evidenceManifest") }}: {{ encodeResult.manifestPath }}</p>
          <p class="field-warning">{{ t("keyFileWarning") }}</p>
          <ResultPager v-model:page="encodePage" :total="encodeResult.items.length" :page-size="RESULT_PAGE_SIZE" :previous-label="t('previousPage')" :next-label="t('nextPage')" :label="t('detectResult')" /><div class="result-items">
            <article v-for="item in visibleEncodeItems" :key="item.outputPath || item.inputPath">
              <strong>{{ basename(item.outputPath || item.inputPath) }}</strong>
              <span>{{ encodeItemStatus(item) }}</span>
              <small class="path-line">{{ item.outputPath || item.inputPath }}</small>
            </article>
          </div>
        </div>
      </article>
    </section>

    <section v-else-if="activeTab === 'decode'" class="panel panel-grid decode-grid">
      <article class="card"><div class="section-heading"><span>01</span><div><h2>{{ t("selectDecodeMedia") }}</h2><p>{{ t("selectDecodeMediaDesc") }}</p></div></div><button :class="['file-picker tall', { 'drag-active': activeDropTarget === 'decode-inputs' }]" type="button" data-drop-target="decode-inputs" :disabled="inputLocked" @click="chooseDecodeInputs"><strong>{{ t("chooseWatermarked") }}</strong><span>{{ activeDropTarget === "decode-inputs" ? t("dropRelease") : pickerHint(decodeInputs, "chooseMediaClick") }}</span></button><ImportedFiles v-model:paths="decodeInputs" :disabled="inputLocked" :title="t('currentImport')" :clear-label="t('clearImportedFiles')" :remove-label="t('removeFile')" :previous-label="t('previousPage')" :next-label="t('nextPage')" /></article>
      <article class="card action-card">
        <div class="section-heading"><span>02</span><div><h2>{{ t("credentials") }}</h2><p>{{ t("credentialsDesc") }}</p></div></div>
        <div class="credential-picker">
          <button :class="['file-picker', { 'drag-active': activeDropTarget === 'key-file' }]" type="button" data-drop-target="key-file" :disabled="inputLocked" @click="chooseKeyFile">
            <strong>{{ t("chooseKeyFile") }}</strong>
            <span>{{ activeDropTarget === "key-file" ? t("dropRelease") : pickerHint(decodeKeyFiles, "chooseKeyFileHint") }}</span>
          </button>
          <div v-if="decodeKeyFiles.length" class="import-summary key-import-summary">
            <div class="file-list inline-list"><span v-for="name in previewNames(decodeKeyFiles)" :key="name">{{ name }}</span><span v-if="restCount(decodeKeyFiles)">+{{ restCount(decodeKeyFiles) }}</span></div>
          </div>
          <button v-if="decodeKeyFiles.length" class="secondary-button clear-key-button" type="button" @click="clearDecodeKeyFile">{{ t("clearKeyFile") }}</button>
        </div>
        <label class="field"><span>{{ t("orPassword") }}</span><input v-model="decodePassword" type="password" autocomplete="current-password" :placeholder="t('passwordModeC')" /></label>
        <details v-if="decodeHasVideo" class="advanced-settings"><summary>{{ t("advancedOptions") }}</summary><label class="field range-field"><span>{{ t("decodeFrameParallelism") }}: {{ decodeFrameParallelism }}</span><input v-model.number="decodeFrameParallelism" type="range" min="1" max="8" step="1" /><small>{{ t("decodeFrameParallelismHint") }}</small></label></details>
        <div class="task-actions"><button class="primary" :disabled="!canDecode || inputLocked" @click="runDecode">{{ isTaskRunning('decode') ? t("decoding") : t("startDecode") }}</button><button v-if="showProgress('decode') && busy && activeTaskId === taskProgress?.taskId" class="secondary-button cancel-button" type="button" :disabled="cancelling" @click="cancelCurrentTask">{{ cancelling ? t("cancellingTask") : t("cancelTask") }}</button></div>
              </article>
      <article class="card result-panel">
        <div class="section-heading"><span>03</span><div><h2>{{ t("detectResult") }}</h2><p>{{ t("detectResultDesc") }}</p></div></div>
        <div v-if="decodeResult" class="result-card success decode-results">
          <p class="result-count">{{ decodeSummary }}</p>
          <ResultPager v-model:page="decodePage" :total="decodeResult.items.length" :page-size="RESULT_PAGE_SIZE" :previous-label="t('previousPage')" :next-label="t('nextPage')" :label="t('detectResult')" />
          <article v-for="{ item, index } in visibleDecodeItems" :key="`${item.inputPath}-${index}`" :class="['decode-item', { failed: !item.succeeded }]">
            <div class="decode-title"><strong>{{ index + 1 }}. {{ basename(item.inputPath) }}</strong><span v-if="item.succeeded" :class="['integrity', item.integrity]">{{ integrityLabel(item.integrity) }}</span><span v-else class="integrity failed">{{ t("decodeFailed") }}</span></div>
            <p v-if="item.succeeded" class="watermark-output">{{ item.text }}</p>
            <template v-else><p class="decode-error">{{ decodeErrorLabel(item) }}</p><button v-if="item.errorCode === 'recovery_timeout' || item.errorCode === 'header_not_found' || item.errorCode === 'payload_corrupted'" class="secondary-button" type="button" :disabled="busy" @click="retryDeepRecovery(index)">{{ t("deepRecovery") }}</button></template>
            <details class="result-details"><summary>{{ t("resultDetails") }}</summary><small class="path-line">{{ item.inputPath }}</small><p v-if="item.error">{{ item.error }}</p>
            <small v-if="item.succeeded && item.mediaType === 'image'">{{ t("fingerprintDistance") }} {{ item.fingerprintDistance ?? "-" }} · {{ t("correctedCodewords") }} {{ item.correctedCodewords }}</small>
            <small v-else-if="item.succeeded">{{ t("validFrames") }} {{ item.validFrames ?? 0 }}/{{ item.frameCount ?? 0 }} · {{ t("expectedFrames") }} {{ item.expectedFrameCount ?? "-" }} · {{ t("modifiedFrames") }} {{ item.modifiedFrames ?? 0 }} · {{ t("missingFrames") }} {{ item.missingFrames ?? 0 }} · {{ t("insertedFrames") }} {{ item.insertedFrames ?? 0 }} · {{ t("deletedFrames") }} {{ item.deletedFrames ?? 0 }} · {{ t("replacedFrames") }} {{ item.replacedFrames ?? 0 }} · {{ t("duplicateFrames") }} {{ item.duplicateFrames ?? 0 }} · {{ t("outOfOrderFrames") }} {{ item.outOfOrderFrames ?? 0 }} · {{ t("unreadableFrames") }} {{ item.unreadableFrames ?? 0 }} · {{ t("transformedFrames") }} {{ item.transformedFrames ?? 0 }}</small>
            <small v-if="item.succeeded && item.timelineAnomalies?.length" class="timeline-summary">{{ t("timelineAnomalies") }}: {{ timelineAnomalySummary(item.timelineAnomalies) }}</small>
            <small v-if="item.succeeded && item.syncRegistration">{{ t("syncRegistration") }} {{ item.syncRegistration.rotationDegrees.toFixed(2) }}° / ×{{ item.syncRegistration.scale.toFixed(3) }} / Δ{{ item.syncRegistration.translationX }},{{ item.syncRegistration.translationY }} / {{ item.syncRegistration.score.toFixed(2) }}</small>
            <small v-if="item.syncRegistration?.mirrored">{{ t("mirroredTransform") }}</small><small v-if="item.syncRegistration?.shearX">{{ t("shearTransform") }} {{ (Math.atan(item.syncRegistration.shearX) * 180 / Math.PI).toFixed(2) }}°</small>
            <small v-if="item.succeeded && item.tamperRegions.length">{{ t("tamperRegions") }} {{ item.tamperRegions.length }}</small></details>
          </article>
        </div>
        <p v-else class="empty-state">{{ t("waitingDecode") }}</p>
      </article>
    </section>

    <section v-else-if="activeTab === 'scan'" class="panel panel-grid scan-grid">
      <article class="card action-card"><div class="section-heading"><span>01</span><div><h2>{{ t("importUnknown") }}</h2><p>{{ t("importUnknownDesc") }}</p></div></div><button :class="['file-picker tall', { 'drag-active': activeDropTarget === 'scan-inputs' }]" type="button" data-drop-target="scan-inputs" :disabled="inputLocked" @click="chooseScanInputs"><strong>{{ t("chooseUnknown") }}</strong><span>{{ activeDropTarget === "scan-inputs" ? t("dropRelease") : pickerHint(scanInputs, "unknownHint") }}</span></button><ImportedFiles v-model:paths="scanInputs" :disabled="inputLocked" :title="t('currentImport')" :clear-label="t('clearImportedFiles')" :remove-label="t('removeFile')" :previous-label="t('previousPage')" :next-label="t('nextPage')" /><div class="task-actions"><button class="primary" :disabled="!canScan || inputLocked" @click="runScan">{{ isTaskRunning('scan') ? t("scanning") : t("scanWatermark") }}</button><button v-if="showProgress('scan') && busy && activeTaskId === taskProgress?.taskId" class="secondary-button cancel-button" type="button" :disabled="cancelling" @click="cancelCurrentTask">{{ cancelling ? t("cancellingTask") : t("cancelTask") }}</button></div><p class="hint-text">{{ t("scanHint") }}</p></article>
      <article class="card result-panel wide-result"><div class="section-heading"><span>02</span><div><h2>{{ t("scanFeedback") }}</h2><p>{{ t("scanFeedbackDesc") }}</p></div></div><div v-if="scanResult" class="scan-results"><ResultPager v-model:page="scanPage" :total="scanResult.items.length" :page-size="RESULT_PAGE_SIZE" :previous-label="t('previousPage')" :next-label="t('nextPage')" :label="t('detectResult')" /><article v-for="item in visibleScanItems" :key="item.inputPath" class="scan-item"><div class="decode-title"><strong>{{ basename(item.inputPath) }}</strong><span :class="['scan-status', item.error ? 'failed' : item.status]">{{ item.error ? t("failure") : item.status === "detected" ? t("foundTrace") : t("notDetected") }}</span></div><p class="scan-summary">{{ item.summary }}</p><small v-if="item.width && item.height">{{ t("imageSize") }} {{ item.width }}×{{ item.height }}</small><div v-if="item.detections.length" class="detection-list"><article v-for="detection in item.detections" :key="`${detection.detector}-${detection.label}`"><div><strong>{{ detection.label }}</strong><span>{{ detection.confidence }} · {{ detectionAccessLabel(detection) }}</span></div><p>{{ detection.content }}</p></article></div></article></div><p v-else class="empty-state">{{ t("waitingScan") }}</p></article>
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
