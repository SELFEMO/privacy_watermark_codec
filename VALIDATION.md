# 水印恢复与界面优化验证记录

## 交付结果与环境

本次应用发布元数据统一为 0.2.12，保留构建、依赖与密钥/数据协议必需字段。完整实现已经写入项目文件，未生成 diff / patch 文件。原有品牌、中英文、三种密钥与图片/视频入口保留。

在 macOS ARM64、macOS 27.0.1、Rust 1.95.0、Node 26.12.0 上验证。耗时为本机调试构建中已优化的核心代码的单次观测，不是所有设备的时间保证。图片标准恢复最多 10 秒，深入恢复最多 60 秒；候选耗尽可以提前返回。读取文件和无法中断的外部调用可能带来少量检查点延迟。

用户提供的 9 张水印图片全部通过完整头校验及正文认证；未加水印的原始 JPG 没有误提取出正文。另一次逐字一致性核对 9/9 通过：所有变换副本的正文与主水印图完全相同，比较只在内存中完成，不记录正文或其哈希；见 [一致性记录](validation/corpus-check.txt)。独立同步导频的独立样本没有通过恢复门槛，按计划不默认启用。

## 真实样本逐张结果

所有副本均在用户提供的同一目录，文件名为 ZQH_Momi_20250323_watermarked - 副本 (编号).png；主水印图为 ZQH_Momi_20250323_watermarked.png。原图为 ZQH_Momi_20250323.jpg。使用原有配套密钥，不更换密钥、不重新编码这些历史图片。

“头通过”表示完整头 CRC 通过；“认证通过”表示纠错后的载荷校验及 AEAD 认证通过。候选列统计诊断 API 的公开候选/进度事件，不包含内部像素相位、正文强度及纠错子集枚举，不能理解为总运算次数。旧代码没有请求内截止时间，基线诊断在进度回调达到约 10 秒时取消，因此旧失败时间可能超过 10 秒；新代码在内部循环中检查截止时间。

| 文件 | 尺寸 | 变换诊断 | 修改前秒 / 结果 | 修改后秒 | 头 | 正文认证 | 事件数前 → 后 | 内容完整性 |
| --- | --- | --- | --- | ---: | --- | --- | --- | --- |
| 副本 (2) | 2109×2812 | 裁边 | 10.638 / 预算内未成功 | 0.344 | 通过 | 通过 | 1898 → 1 | 已修改 |
| 副本 (3) | 2728×3637 | 180° 旋转 | 9.029 / 成功 | 0.453 | 通过 | 通过 | 2565 → 4 | 已修改 |
| 副本 (4) | 2728×3637 | 水平镜像 | 10.916 / 预算内未成功 | 0.519 | 通过 | 通过 | 2506 → 5 | 已修改 |
| 副本 (5) | 2728×3637 | 垂直镜像 | 11.008 / 预算内未成功 | 0.697 | 通过 | 通过 | 2506 → 8 | 已修改 |
| 副本 (6) | 2557×3410 | 约 −2.923° 旋转并裁边 | 10.001 / 预算内未成功 | 1.803 | 通过 | 通过 | 3482 → 9 | 已修改 |
| 副本 (7) | 2855×3637 | 约 2° 水平剪切及白边 | 10.001 / 预算内未成功 | 0.941 | 通过 | 通过 | 2896 → 9 | 已修改 |
| 副本 (8) | 2728×3637 | 局部遮挡 | 0.189 / 成功 | 0.159 | 通过 | 通过 | 1 → 1 | 已修改 |
| 副本 (9) | 2426×3033 | 裁边 | 2.442 / 成功 | 0.502 | 通过 | 通过 | 1 → 1 | 已修改 |
| 主水印图 | 2728×3637 | 未变换 | 0.231 / 成功 | 0.210 | 通过 | 通过 | 1 → 1 | 未发现明显修改 |

主水印图从 0.231 秒到 0.210 秒；180° 副本从 9.029 秒到 0.453 秒；两个原来预算内失败的重点样本，旋转裁边副本为 1.803 秒，剪切白边副本为 0.941 秒。全部 9 张提取耗时为 0.159–1.803 秒。

未加水印的 2728×3637 JPG 用时 7.768 秒、48 个公开候选事件，返回“未检测到通过校验的水印头”，没有返回正文，也没有把相关高分当作水印存在。原图仅参与变换诊断与测试，正式解码接口只接收待解图片及凭据。

副本 (7) 经过原图辅助诊断更符合水平剪切：横坐标含约 tan(2°) 倍的纵坐标项，而非纯相似旋转。这是此前单纯扩充旋转角度仍无法解决的设计缺口。新解码在有直线补边证据时单独恢复该模型，并报告 shearX；不宣称支持任意透视或非刚性形变。

脱敏逐文件记录：[修改前](validation/baseline.json)、[最终结果](validation/real-samples.json)。记录仅含文件名、尺寸、耗时、状态和事件数，不含私有密钥或水印正文。

## 实现要点

- 删除固定角度/倍率的强制优先入口、借用其他候选分数的路径与分数保底；精化候选去重、排除已尝试项之后才执行验证限额。
- 白边拟合保留方向和可信度；支持只有两个白角的情况，不通过先裁边丢掉偏移。联合处理旋转、内容尺度、画布、浮点平移、像素相位与正/负块原点。
- 用多个分散窗口的块周期频谱提供独立几何线索。单个密钥的公开盐可辅助评分；密码路径不需要预先知道盐，独立裁边测试通过。
- 任意像素起点直接求 Haar 方向差，不创建每块小图；请求内缓存系数观测和派生密钥。头、旧格式、正文、BCH、FFT、变换物化和镜像分支共用截止时间与取消信号。
- 相关分只用于排序和微调；完整头校验与正文认证决定成功。无头、正文损坏、凭据错误、超时和取消保持不同错误语义。
- 新编码默认使用金字塔载体加 BCH 码字交织，降低连续裁剪集中消除一个码字的风险。旧 DCT、一级 Haar、旧金字塔和现有密钥继续读取；旧应用不保证能读取新的交织配置。
- 独立周期导频通过显式配置区分，仅核心实验 API 可请求，前端默认编码不启用。输出实测 PSNR 必须严格高于 40 dB，等于 40 dB 也拒绝。
- 文件哈希流式读取，FFmpeg 资源分块还原，目录导入在后台线程执行；FFmpeg 优先读取自身进度、目录扫描回退限频。开发/构建保留增量缓存，显式清理命令保留。
- 视频正常帧仍逐帧检测，失败帧共享整段几何恢复预算；封装回读通过一次 FFmpeg 顺序解码选择去重的首、中、尾帧。
- 公共任务区独立于页签；先显示正在取消，后台退出后显示已取消。结果使用稳定索引、每页 50 条，深入恢复只更新原槽位；文件可展开、分页与逐项删除，高级参数收起。
- 窄英文布局使用单列并由外层面板滚动，避免卡片只剩标题而裁掉表单；保留缩放后的无边框窗口控制按钮。

## 自动测试与构建

| 检查 | 结果 | 范围 |
| --- | --- | --- |
| cargo test --workspace --offline -- --test-threads=2 | 113 通过、0 失败、1 可选诊断忽略 | 核心 65、图片集成 29、Tauri 19；图片攻击集约 18.60 秒 |
| npm run test:frontend | 2 通过 | 1000 条乱序及重复事件、计数、稳定槽位、单项更新及过期任务拒绝 |
| npm run build | 通过 | Vue/TypeScript 类型检查与 Vite 生产构建 |
| cargo build --workspace --offline | 通过 | macOS ARM64 原生可执行文件编译链接 |
| cargo build -p watermark-core --examples --offline | 通过 | 可复现诊断入口 |
| npm run test:ui | 通过 | Edge 无头浏览器、模拟 Tauri 桥接；下述详细交互与布局 |
| 修改文件空白检查 | 通过 | 无新增空白错误；保留原 watermark.rs 的 CRLF 风格 |

几何回归使用仓库固定密文、固定测试密钥和固定攻击参数，覆盖直角、7°、17.3°、41.7°、71.26°、137.36°、白边、放大、对称裁边、去白边、镜像组合与不整齐分辨率恢复。单元测试还验证历史 DCT / Haar 配置和严格质量门槛。未使用反复生成随机水印直到成功的重试。固定数据及哈希见 [夹具说明](crates/watermark-core/tests/fixtures/README.md)。

剩余的 ignored 项是需要外部 PWC_REAL_ATTACK_DIR 的诊断工具，不是被跳过的几何成功标准；真实用户样本已经另行逐张执行。测试并发限制为 2，避免多个带墙钟预算的几何请求争用 CPU；不将默认无限并发下的超时当成同一设备单图耗时。生产编码继续使用系统安全随机数，单元测试专用随机分支不进入生产构建。

预算/取消测试在头扫描开始后触发取消，并分别进入正文观测、BCH、FFT 和变换物化循环后触发取消。标准 10 秒 / 深入 60 秒的最大预算及不随阶段重置的截止时间有独立断言。另对独立样本 0.4° / 1.01× 组合实际执行深入恢复：约 30.533 秒后候选耗尽，仍无有效头；没有伪造成功，也没有强行等待满 60 秒。[实际深入恢复记录](validation/deep-budget.txt)。

界面测试包括 1000 条逆序结果、50 条 DOM 卡片、逐项删除与清空、跨页签任务显示、取消前后的状态、取消后迟到进度、旧任务迟到失败，以及清空导入队列后对原结果深入恢复。测试发送 135% 缩放请求，在等效 830×563 逻辑视口及更窄的 560×380 视口检查窗口按钮、单列、可滚动正文和表单可达性。它不替代各平台原生 WebView / DPI 验收。

界面截图使用模拟数据：[中文](validation/ui-zh.png)、[英文缩放](validation/ui-en-zoom.png)。

## 补充媒体攻击与视频回读

使用固定 1024×768 合成密文，所有攻击只变换同一份夹具；JPEG 由 Pillow 默认色度采样生成，视频使用本机现有 FFmpeg 二进制。详细结果：[补充记录](validation/additional-results.json)。

| 固定攻击 | 提取 / 正文认证 | 秒 | 指纹判断 |
| --- | --- | ---: | --- |
| JPEG quality 90 | 通过 | 0.034 | 未发现明显修改 |
| JPEG quality 85 | 通过 | 0.023 | 不确定 |
| 左/上/右/下裁去 13/29/32/17 像素 | 通过 | 0.141 | 已修改 |
| 左上四分之一替换为不透明黑色 | 通过 | 0.021 | 已修改 |
| 盐匹配但派生密钥错误 | 正确拒绝解密 | 0.017 | 不生成正文 |
| 5 帧 H.264 lossless / yuv444p，回读首、中、尾 | 三帧全部通过 | 0.022 / 0.022 / 0.021 | 三帧均不确定 |

视频烟雾测试验证实际 FFmpeg 封装、单次 select 抽取和核心回读，未通过 Tauri 原生界面运行完整视频批次。时间轴插入、删除、替换、重复、错序及早期取消由后端单元测试覆盖。

感知完整性与水印认证是不同结论。平滑渐变中的近乎相等亮度比较会因量化改变，可能让未攻击图的局部分区呈不确定；没有为了让测试显示完整而提高容忍阈值。成功提取不证明画面未改。

## 独立同步导频门槛

开发图片为用户原图的实验编码，独立图片为 scikit-image 官方数据集的 astronaut（512×512）；[官方数据说明](https://scikit-image.org/docs/stable/api/skimage.data.html#skimage.data.astronaut) 和 [数据来源](https://raw.githubusercontent.com/scikit-image/scikit-image/v0.24.0/skimage/data/astronaut.png)。私有开发图片及实验密钥均只留在被忽略的本地诊断目录，未加入交付文件。

导频周期为 128 像素，四个非共线分量，块内共同整数亮度偏移幅度系数 0.85；从四个窗口拟合角度/尺度和相位。开发图实测输出 PSNR 40.321149 dB，独立图 40.091498 dB。两者未变换时均能完整提取。

攻击固定为 Pillow bicubic 重采样、保持原比例扩展白画布后叠加下列内容放大。1.7° 另做白边包围盒裁边与 JPEG quality 85。始终复用最初那份密文，不为失败项重编码。这里的成功仅表示整个解码流程认证通过，不能归因于导频单独贡献。

| 攻击 | 开发图 | 独立图 |
| --- | --- | --- |
| 未变换 | 成功 0.213 秒 | 成功 0.017 秒 |
| 0.4° / 1.01× | 成功 4.770 秒 | 无有效头 2.493 秒 |
| 1.7° / 1.03× | 成功 4.955 秒 | 无有效头 3.863 秒 |
| 上项再裁白边 | 成功 4.742 秒 | 无有效头 3.876 秒 |
| 上项旋转后 JPEG 85 | 10 秒超时 | 无有效头 1.728 秒 |
| −5.6° / 1.07× | 成功 4.988 秒 | 无有效头 3.100 秒 |
| 12.8° / 1.10× | 成功 6.081 秒 | 无有效头 4.598 秒 |

质量和未变换提取通过，但独立旋转组合失败，因此启用门槛未通过，默认关闭。详细记录：[导频实验](validation/pilot-results.json)。这不是“所有旋转都已解决”的结论，尤其不能把开发样本上的改善外推到所有尺寸和纹理。

## 复现入口

先下载依赖，再使用离线命令。不要先清理整个 target，避免丢掉编译缓存。主检查：

~~~sh
cargo test --workspace --offline -- --test-threads=2
npm run test:frontend
npm run build
cargo build --workspace --offline
cargo build -p watermark-core --examples --offline
~~~

真实图片诊断仅打印阶段、耗时、文本长度和状态，不打印正文：

~~~sh
target/debug/examples/inspect_image IMAGE KEY 10
target/debug/examples/inspect_image IMAGE KEY 60
target/debug/examples/inspect_corpus SAMPLE_DIRECTORY KEY REFERENCE_WATERMARKED_IMAGE
python3 scripts/benchmark-recovery.py SAMPLE_DIRECTORY KEY --json target/diagnostics/repeated-samples.json
~~~

界面回归需要运行 Vite，另提供 Playwright 安装和浏览器；它们不是应用运行依赖。PWC_PLAYWRIGHT_ROOT 可指向含 Playwright 的 node_modules 对应 package.json 所在目录，PWC_BROWSER_EXECUTABLE 可指向本机 Chromium / Edge 可执行文件；未指定时使用普通 Node 模块解析及 Playwright 的默认 Chromium。PWC_UI_URL 默认 http://127.0.0.1:1420。

~~~sh
npm run dev
# 在另一个终端配置可选环境变量后运行：
npm run test:ui
~~~

## 能力边界与未验证范围

- 标准/深入预算是上限，水印信息已被裁切、重采样、压缩消除时可以失败。通过的固定样本不代表任意图片或任意参数都保证成功。
- 补边水平剪切恢复需要直线边界证据；不支持一般透视、局部扭曲及社交平台的所有处理链。
- 未在本次环境编译或运行 Windows、Linux、Intel Mac 包，也未生成安装包或进行签名、公证、发布。
- 浏览器模拟测试没有替代原生拖放、真实窗口操作、系统 DPI、多种 WebView 以及整段视频界面工作流的实机验证。
- 已存在的本地 FFmpeg 二进制变更原样保留，本次没有替换其内容。测试仅使用该现有二进制；macOS ARM64 的 FFmpeg 和 ffprobe 实际文件均与清单 SHA-256 匹配。
- 用户原图、水印图、变换副本及配套密钥未加入仓库；这里只保留脱敏结果记录、合成测试夹具及模拟界面截图。

## 完整交付文件

以下为本次修改或新增的完整文件，可直接打开和覆盖使用；不需要手工合并示意片段。现有 FFmpeg 二进制的先前本地变更不计入清单。

- [Cargo.lock](Cargo.lock)
- [PROJECT_OVERVIEW.md](PROJECT_OVERVIEW.md)
- [README.md](README.md)
- [README.zh-CN.md](README.zh-CN.md)
- [VALIDATION.md](VALIDATION.md)
- [crates/watermark-core/Cargo.toml](crates/watermark-core/Cargo.toml)
- [crates/watermark-core/examples/generate_fixtures.rs](crates/watermark-core/examples/generate_fixtures.rs)
- [crates/watermark-core/examples/inspect_corpus.rs](crates/watermark-core/examples/inspect_corpus.rs)
- [crates/watermark-core/examples/inspect_image.rs](crates/watermark-core/examples/inspect_image.rs)
- [crates/watermark-core/examples/sync_probe.rs](crates/watermark-core/examples/sync_probe.rs)
- [crates/watermark-core/src/bch.rs](crates/watermark-core/src/bch.rs)
- [crates/watermark-core/src/crypto.rs](crates/watermark-core/src/crypto.rs)
- [crates/watermark-core/src/dwt_qim.rs](crates/watermark-core/src/dwt_qim.rs)
- [crates/watermark-core/src/error.rs](crates/watermark-core/src/error.rs)
- [crates/watermark-core/src/lib.rs](crates/watermark-core/src/lib.rs)
- [crates/watermark-core/src/payload.rs](crates/watermark-core/src/payload.rs)
- [crates/watermark-core/src/pilot.rs](crates/watermark-core/src/pilot.rs)
- [crates/watermark-core/src/recovery.rs](crates/watermark-core/src/recovery.rs)
- [crates/watermark-core/src/spectral.rs](crates/watermark-core/src/spectral.rs)
- [crates/watermark-core/src/sync.rs](crates/watermark-core/src/sync.rs)
- [crates/watermark-core/src/watermark.rs](crates/watermark-core/src/watermark.rs)
- [crates/watermark-core/tests/fixtures/1024x768.png](crates/watermark-core/tests/fixtures/1024x768.png)
- [crates/watermark-core/tests/fixtures/1920x1080.png](crates/watermark-core/tests/fixtures/1920x1080.png)
- [crates/watermark-core/tests/fixtures/640x480.png](crates/watermark-core/tests/fixtures/640x480.png)
- [crates/watermark-core/tests/fixtures/960x1280.png](crates/watermark-core/tests/fixtures/960x1280.png)
- [crates/watermark-core/tests/fixtures/README.md](crates/watermark-core/tests/fixtures/README.md)
- [crates/watermark-core/tests/roundtrip.rs](crates/watermark-core/tests/roundtrip.rs)
- [package-lock.json](package-lock.json)
- [package.json](package.json)
- [scripts/benchmark-recovery.py](scripts/benchmark-recovery.py)
- [scripts/build-release.mjs](scripts/build-release.mjs)
- [scripts/frontend-state.test.mjs](scripts/frontend-state.test.mjs)
- [scripts/tauri-cli.mjs](scripts/tauri-cli.mjs)
- [scripts/tauri-dev.mjs](scripts/tauri-dev.mjs)
- [scripts/ui-check.mjs](scripts/ui-check.mjs)
- [src-tauri/Cargo.toml](src-tauri/Cargo.toml)
- [src-tauri/src/cancellation.rs](src-tauri/src/cancellation.rs)
- [src-tauri/src/commands.rs](src-tauri/src/commands.rs)
- [src-tauri/src/drop_paths.rs](src-tauri/src/drop_paths.rs)
- [src-tauri/src/evidence.rs](src-tauri/src/evidence.rs)
- [src-tauri/src/ffmpeg.rs](src-tauri/src/ffmpeg.rs)
- [src-tauri/src/models.rs](src-tauri/src/models.rs)
- [src-tauri/src/video.rs](src-tauri/src/video.rs)
- [src-tauri/tauri.conf.json](src-tauri/tauri.conf.json)
- [src/App.vue](src/App.vue)
- [src/components/ImportedFiles.vue](src/components/ImportedFiles.vue)
- [src/components/ResultPager.vue](src/components/ResultPager.vue)
- [src/components/TaskProgressPanel.vue](src/components/TaskProgressPanel.vue)
- [src/styles.css](src/styles.css)
- [src/taskState.ts](src/taskState.ts)
- [src/types.ts](src/types.ts)
- [validation/additional-results.json](validation/additional-results.json)
- [validation/baseline.json](validation/baseline.json)
- [validation/corpus-check.txt](validation/corpus-check.txt)
- [validation/deep-budget.txt](validation/deep-budget.txt)
- [validation/pilot-results.json](validation/pilot-results.json)
- [validation/real-samples.json](validation/real-samples.json)
- [validation/ui-en-zoom.png](validation/ui-en-zoom.png)
- [validation/ui-zh.png](validation/ui-zh.png)


## Windows 构建入口验证（2026-10-23）

本节记录本次 Windows 构建修复，应用发布元数据统一为 0.2.13。前述 macOS、算法与媒体样本记录作为历史验证保留，不视为本轮重新执行。

原始 npm 日志的参数包含字面量 clean;，镜像地址也带尾随分号。错误发生在 npm 查找脚本时，尚未执行现有清理逻辑。新增 desktop:build:clean 和 desktop:dev:clean，通过 Node 直接启动 npm_execpath 指向的 npm CLI，按参数数组依次执行完整清理、npmmirror 安装和原有桌面入口；前一步失败即停止并保留退出码。预览模式不执行这些步骤，额外分隔符之后的参数归目标命令所有。

本机环境：Windows x64（系统内核 10.0.26300）、Node 22.18.0、npm 11.21.1、Cargo 1.96.1，使用本机 MSVC 工具链。实际检查结果：

- npm run test:workflow：12 项通过。覆盖开发/构建顺序、三阶段失败停止与退出码、中文及空格路径、特殊字符参数、环境继承、缺失或无效 npm CLI、预览无副作用、真实子进程启动错误，以及 SIGINT / SIGTERM 处理与后代进程退出。信号用隔离父进程触发同一处理器验证，不宣称手动进行控制台按键验收。
- npm run test:frontend：2 项通过，保留乱序事件、重试槽位与过期任务检查。
- npm run build：在完整流程内通过 Vue/TypeScript 检查与 Vite 生产构建。
- CMD 和 PowerShell 的 npm run desktop:build:clean -- --dry-run：通过。Windows PowerShell 的本机执行策略阻止 npm.ps1，使用 npm.cmd run desktop:build:clean -- --dry-run 通过；未修改系统执行策略。
- cargo metadata --offline --locked --no-deps --format-version 1：通过，两个工作区包均为目标应用版本。
- npm run desktop:build:clean：最终完整执行成功，退出码为 0；确实执行了全量清理、镜像安装、前端构建、Rust 原生编译、NSIS 打包和规范命名产物复制。Rust release 编译输出记录为 2 分 09 秒，此数值不包含安装、前端构建和 NSIS 打包耗时。
- 六个清单/锁文件中的八处自有版本值一致；npm 第三方锁定条目、依赖声明，以及 Cargo 锁文件中的第三方包内容均无变化。
- 构建结束后 Tauri 配置逐字节核对通过，仅保留批准的应用版本更新。FFmpeg 二进制路径及哈希未变；生成清单中的平台信息保留在构建输出，源码清单已恢复到构建前内容。

首次受限环境构建在访问 crates.io 时失败，本地离线缓存缺少 rustfft。补齐 Cargo.lock 锁定的依赖并允许必要打包工具下载后，从完整清理入口重新执行成功。没有以网络失败代替构建通过，也没有调整依赖版本来规避问题。

产物为 [Windows x64 NSIS 安装包](release/privacy-watermark-codec-windows-x64.exe)，文件大小 169,479,760 字节，SHA-256：37A9BD873C2A55AD64F0DD1B8E0B0460AFA25D864024E37FD1335C74A54EA103。原生程序及安装包的 ProductVersion 和 FileVersion 均实测为目标应用版本。规范命名产物位于被 Git 忽略的 release 目录，链接指向本地文件，不表示已上传发布。

本轮验收覆盖编译和打包，没有安装或启动应用，也没有重新执行此前媒体样本、全平台原生界面或算法攻击回归。

完整修改文件均已写入原路径，可直接打开并覆盖使用：

- [npm 清单](package.json)
- [npm 锁文件](package-lock.json)
- [Cargo 锁文件](Cargo.lock)
- [桌面包清单](src-tauri/Cargo.toml)
- [核心库清单](crates/watermark-core/Cargo.toml)
- [Tauri 配置](src-tauri/tauri.conf.json)
- [统一桌面工作流完整源码](scripts/desktop-workflow.mjs)
- [工作流回归测试完整源码](scripts/desktop-workflow.test.mjs)
- [英文说明](README.md)
- [中文说明](README.zh-CN.md)
- [开发工作流技能](.cursor/skills/pwc-dev-workflow/SKILL.md)
- [本验证记录](VALIDATION.md)
