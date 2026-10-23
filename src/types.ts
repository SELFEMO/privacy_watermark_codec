export type KeyMode = "independent" | "shared" | "custom";

export interface EncodeRequest {
  inputPaths: string[];
  outputDir: string;
  text: string;
  keyMode: KeyMode;
  customPassword?: string;
  writeKeyFile: boolean;
  strength: number;
  frameParallelism?: number;
  taskId?: string;
}

export interface EncodeItemResult {
  inputPath: string;
  outputPath: string;
  keyPath?: string;
  manifestPath?: string;
  mediaType: "image" | "video";
  psnr?: number;
  frameCount?: number;
  succeeded?: boolean;
  error?: string;
}

export interface EncodeResponse {
  outputRoot: string;
  items: EncodeItemResult[];
  sharedKeyPath?: string;
  manifestPath?: string;
}

export interface DecodeRequest {
  inputPaths: string[];
  keyFile?: string;
  keyFiles?: string[];
  customPassword?: string;
  frameParallelism?: number;
  taskId?: string;
  recoveryMode?: "standard" | "deep";
}

export interface TamperRegion {
  index: number;
  x: number;
  y: number;
  width: number;
  height: number;
  distance: number;
  status: "intact" | "uncertain" | "modified" | string;
}

export interface SyncRegistration {
  rotationDegrees: number;
  scale: number;
  translationX: number;
  translationY: number;
  score: number;
  shearX?: number;
  mirrored?: boolean;
}


export type TimelineAnomalyKind =
  | "insertion"
  | "deletion"
  | "replacement"
  | "duplicate"
  | "out_of_order";

export interface TimelineAnomalyRange {
  kind: TimelineAnomalyKind;
  physicalStartFrame?: number;
  physicalEndFrame?: number;
  expectedStartFrame?: number;
  expectedEndFrame?: number;
  count: number;
}

export interface DecodeItemResult {
  inputPath: string;
  succeeded: boolean;
  error?: string;
  errorCode?: "header_not_found" | "recovery_timeout" | "authentication_failed" | "payload_corrupted" | "cancelled" | "invalid_input" | "other";
  mediaType: "image" | "video";
  text: string;
  integrity: "intact" | "uncertain" | "modified";
  fingerprintDistance?: number;
  correctedCodewords: number;
  frameCount?: number;
  expectedFrameCount?: number;
  validFrames?: number;
  modifiedFrames?: number;
  missingFrames?: number;
  insertedFrames?: number;
  deletedFrames?: number;
  duplicateFrames?: number;
  outOfOrderFrames?: number;
  replacedFrames?: number;
  unreadableFrames?: number;
  transformedFrames?: number;
  timelineAnomalies: TimelineAnomalyRange[];
  tamperRegions: TamperRegion[];
  syncRegistration?: SyncRegistration;
}

export interface DecodeResponse {
  items: DecodeItemResult[];
}

export interface DecodeItemEvent {
  taskId?: string;
  current: number;
  total: number;
  item: DecodeItemResult;
}

export interface ScanRequest {
  inputPaths: string[];
  taskId?: string;
}

export interface ScanDetection {
  detector: string;
  label: string;
  content: string;
  confidence: "high" | "medium" | "low" | string;
  needsKey: boolean;
}

export interface ScanItemResult {
  inputPath: string;
  status: "detected" | "not_detected";
  summary: string;
  detections: ScanDetection[];
  width?: number;
  height?: number;
  error?: string;
}

export interface ScanResponse {
  items: ScanItemResult[];
}

export interface CancelTaskRequest {
  taskId: string;
}

export type TaskProgressKind = "encode" | "decode" | "scan";
export type TaskOutcome = "running" | "completed" | "completed_with_errors" | "failed" | "cancelled";
export type ProgressUnitKind = "file" | "frame" | "geometry";

export interface TaskProgressEvent {
  taskId?: string;
  task: TaskProgressKind;
  phase: string;
  message: string;
  /** 批次：已处理完的文件序号（1-based 显示用 fileIndex+1） */
  current: number;
  total: number;
  percent: number;
  currentPath?: string;
  outcome?: TaskOutcome;
  fileFraction?: number;
  filesSucceeded?: number;
  filesFailed?: number;
  unitKind?: ProgressUnitKind;
  unitCurrent?: number;
  unitTotal?: number;
  unitLabel?: string; // 例如 "+2.00° ×0.956 · 近角度"
}

export interface FfmpegBinaryInfo {
  name: string;
  path: string;
  expectedSha256: string;
  actualSha256: string;
  hashOk: boolean;
  versionLine: string;
  error?: string;
}

export interface FfmpegRuntimeInfo {
  platform: string;
  version: string;
  source: string;
  buildLicense: string;
  buildConfigure: string;
  generatedAt?: string;
  utcCompileDate?: string;
  ffmpeg?: FfmpegBinaryInfo;
  ffprobe?: FfmpegBinaryInfo;
  extraBinaries: FfmpegBinaryInfo[];
  licenseText: string;
}

export interface ReleasePackageSigningMetadata {
  signatureAlgorithm: string;
  signer: string;
  signature: string;
  artifactSha256: string;
  manifestSha256: string;
}

export interface ReleaseMetadata {
  automaticUpdate: boolean;
  manifestUrl: string;
  packageSigning: ReleasePackageSigningMetadata;
}

export interface LaunchContext {
  action?: "encode" | "decode" | "scan" | string;
  files: string[];
}

export interface DropPathResolution {
  files: string[];
  directories: string[];
  ignoredCount: number;
  truncated: boolean;
}
