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
  customPassword?: string;
  frameParallelism?: number;
  taskId?: string;
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
  score: number;
}

export interface DecodeItemResult {
  inputPath: string;
  mediaType: "image" | "video";
  text: string;
  integrity: "intact" | "uncertain" | "modified";
  fingerprintDistance?: number;
  correctedCodewords: number;
  frameCount?: number;
  validFrames?: number;
  modifiedFrames?: number;
  tamperRegions: TamperRegion[];
  syncRegistration?: SyncRegistration;
}

export interface DecodeResponse {
  items: DecodeItemResult[];
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
}

export interface ScanResponse {
  items: ScanItemResult[];
}

export interface CancelTaskRequest {
  taskId: string;
}

export type TaskProgressKind = "encode" | "decode" | "scan";

export interface TaskProgressEvent {
  taskId?: string;
  task: TaskProgressKind;
  phase: string;
  message: string;
  current: number;
  total: number;
  percent: number;
  currentPath?: string;
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
