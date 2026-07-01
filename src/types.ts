export type KeyMode = "independent" | "shared" | "custom";

export interface EncodeRequest {
  inputPaths: string[];
  outputDir: string;
  text: string;
  keyMode: KeyMode;
  customPassword?: string;
  writeKeyFile: boolean;
  strength: number;
}

export interface EncodeItemResult {
  inputPath: string;
  outputPath: string;
  keyPath?: string;
  mediaType: "image" | "video";
  psnr?: number;
  frameCount?: number;
}

export interface EncodeResponse {
  outputRoot: string;
  items: EncodeItemResult[];
  sharedKeyPath?: string;
}

export interface DecodeRequest {
  inputPaths: string[];
  keyFile?: string;
  customPassword?: string;
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
}

export interface DecodeResponse {
  items: DecodeItemResult[];
}

export interface ScanRequest {
  inputPaths: string[];
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

export interface LaunchContext {
  action?: "encode" | "decode" | "scan" | string;
  files: string[];
}
