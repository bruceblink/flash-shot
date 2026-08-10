# Windows 手工验收记录

本记录用于验证自动化测试不能覆盖的 Windows 桌面行为。它不是发布声明：只有每项都填入
实际环境、日期和证据后，状态才能由“待执行”改为“通过”。

## 状态定义

| 状态 | 含义 |
| --- | --- |
| 待执行 | 尚未在要求的真实硬件和桌面会话中验证。 |
| 通过 | 步骤完成，结果符合预期，且已保留截图、视频或机器可读报告路径。 |
| 失败 | 实际结果不符合预期；必须记录复现条件和问题链接。 |
| 阻塞 | 设备、显示器、FFmpeg 或权限条件缺失，不能据此声称功能通过。 |
| 暂缓 | 当前开发范围明确不执行；保留步骤和通过条件，恢复范围后必须重新验收。 |

## 执行前记录

每次验收先复制此段，并填写真实信息：

```text
日期：
Flash Shot 提交：
Windows 版本与内部版本：
GPU 与驱动版本：
显示器（物理分辨率、排列、缩放）：
FFmpeg 版本与 ddagrab/gdigrab 支持：
证据目录：
```

开始前关闭已有 Flash Shot 实例，使用 `cargo run --release` 或同一提交构建的发布包启动。
全局快捷键已在设置中配置为默认值时，区域截图使用 `Ctrl+Shift+Print Screen`。

## 矩阵

| 场景 | 状态 | 操作 | 通过条件 | 证据 |
| --- | --- | --- | --- | --- |
| 单显示器 100% | 待执行 | 触发区域截图，拖动选区，键盘微调；依次复制、保存、Pin、取消。 | 覆盖层只有一组可操作工具栏；复制和保存的像素尺寸等于选区物理尺寸；取消不留下窗口。 | |
| 负坐标双屏 | 暂缓 | 将副屏放在主屏左侧，跨屏拖动选区并在两屏边缘调整大小。 | 选区和放大镜不跳变；每个显示器只显示自己的覆盖层操作区；导出结果没有偏移或裁切错误。 | 用户于 2026-08-10 暂缓双屏范围；恢复时执行。 |
| 混合 DPI 双屏 | 暂缓 | 将两个显示器设置为不同缩放，例如 100% 与 150% 或 200%；跨屏截图、保存并核对像素尺寸。 | 光标、选区、窗口智能识别和导出均以物理像素对齐；无重复或缺失的工具栏。 | 用户于 2026-08-10 暂缓双屏范围；恢复时执行。 |
| 窄选区与最小设置窗 | 待执行 | 将设置窗口缩小到最小可用尺寸；在屏幕边缘创建窄选区，展开次级操作与标注面板。 | 文本不截断，控件不重叠；主工具栏保持可点击，次级菜单在可用一侧展开。 | |
| 多 Pin 生命周期 | 待执行 | 连续创建至少三张 Pin，分别移动、缩放、调透明度、复制、保存、关闭；期间再次截图。 | 各 Pin 独立响应，关闭一个不影响其余窗口，主应用可继续进入截图覆盖层。 | |
| OCR、翻译与滚动 | 待执行 | 在含文字的选区运行 OCR；在翻译服务可用时运行翻译并模拟一次失败后重试；执行滚动后自动捕获。 | OCR/翻译结果可复制；失败时保留原选区并显示匹配的重试操作；自动滚动等待目标重绘后只追加一帧。 | |
| FFmpeg 录屏 | 待执行 | 使用支持 `ddagrab` 或 `gdigrab` 的 FFmpeg，分别启动显示器、窗口、区域录制，暂停、恢复并停止。 | 产生可播放 MP4；状态、时长和保存路径正确；停止后无遗留 FFmpeg 子进程。 | |

## 自动证据

自动门禁不能替代上述交互矩阵，但每次手工验收都应在同一提交上附带：

```powershell
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --lib
cargo run --release --bin capture-stress -- --output target/capture-stress.json
cargo run --release --bin annotation-stress -- --iterations 30
cargo run --release --bin windows-acceptance-probe -- --output target/windows-acceptance-environment.json
cargo run --release --bin recognition-acceptance -- --output target/ui-acceptance/recognition-acceptance.json
# 可选依赖门禁：只有在本机明确要求 OCR/翻译就绪时才使用；未安装或未配置会以非零退出。
cargo run --release --bin recognition-acceptance -- --require-ocr --output target/ui-acceptance/recognition-acceptance-required-ocr.json
cargo run --release --bin recognition-acceptance -- --require-translation --output target/ui-acceptance/recognition-acceptance-required-translation.json
# 可选真实链路门禁：对含文字的项目截图执行完整 PNG -> Tesseract OCR，只写入文本长度等元数据。
cargo run --release --bin recognition-acceptance -- --require-ocr --ocr-image target/ui-acceptance/settings-current-dark-520x640.png --output target/ui-acceptance/recognition-acceptance-ocr-fixture.json
# 可选真实翻译链路：只有配置 HTTPS 端点时执行；只写入翻译结果长度等元数据，不保存原文或译文。
cargo run --release --bin recognition-acceptance -- --require-translation --translation-text "Flash Shot" --output target/ui-acceptance/recognition-acceptance-translation-exercise.json
cargo run --release --bin settings-ui-acceptance -- light 980 760 target/ui-acceptance/settings-light-980x760.png
cargo run --release --bin settings-ui-acceptance -- light 520 640 target/ui-acceptance/settings-light-520x640.png
cargo run --release --bin settings-ui-acceptance -- dark 980 760 target/ui-acceptance/settings-dark-980x760.png
cargo run --release --bin settings-ui-acceptance -- dark 520 640 target/ui-acceptance/recording-settings-520x640.png 1500 1000 1.0 record
# Record 页生命周期视觉复核：最后的 state 参数只注入 UI 状态，不会启动 FFmpeg。
cargo run --release --bin settings-ui-acceptance -- light 520 640 target/ui-acceptance/recording-ui-paused.png 3000 0 1.0 record 0 paused
# 翻译服务测试忙状态：不发起网络请求，仅检查按钮与状态栏的可读性。
cargo run --release --bin settings-ui-acceptance -- light 520 1200 target/ui-acceptance/translation-service-testing.png 3000 0 1.0 capture 0 idle translation-testing
cargo run --release --bin scroll-acceptance -- --output target/ui-acceptance/scroll-acceptance.json
# 在实际 150%/200% Windows 缩放环境执行，最后一个参数会校验窗口 DPI
cargo run --release --bin settings-ui-acceptance -- light 520 640 target/ui-acceptance/settings-scale-150.png 1500 0 1.5
cargo run --release --bin settings-ui-acceptance -- light 520 640 target/ui-acceptance/settings-scale-200.png 1500 0 2.0
# 双屏范围恢复后可选择零基显示器索引；这会把窗口放到指定显示器并保留其 DPI 证据
cargo run --release --bin settings-ui-acceptance -- light 520 640 target/ui-acceptance/settings-display-1-scale.png 1500 0 1.5 capture 1
```

`settings-ui-acceptance` 为每个 PNG 写入同名 JSON，其中包含物理窗口边界、Windows DPI
和缩放比例。提供最后一个 `expected-scale` 参数时，命令只有在 `scale_match` 为 `true`
时才成功；150%/200% 验收必须保留 `scale_factor` 为 `1.5` 或 `2.0` 的对应证据。
设置页探针应串行执行：截图 worker 捕获窗口所在的桌面物理区域，多个验收窗口重叠时，
后启动的进程可能截到前一个窗口，不能把并行输出当作独立主题或页面证据。
提供最后一个 `display-index` 参数时，探针会将窗口放到指定的零基显示器，并在同名 JSON
中保留该窗口实际观测到的 `dpi`、物理边界和 `scale_factor`；索引不存在时命令失败，避免
把另一块显示器的截图误记为目标 DPI 证据。
提供其后的 `idle|starting|recording|paused|stopping` 参数时，探针会在不创建 FFmpeg 子进程
的情况下固定 Record 页生命周期外观；截图前会把目标窗口置前，避免桌面区域捕获被其他窗口遮挡。

将 `capture-stress.json`、标注压力输出以及手工截图/视频放入记录中的证据目录。不要提交
机器专属的 `target` 输出；在问题或发布验收单中引用它们即可。

## 当前记录

| 日期 | 提交 | 自动门禁 | 手工矩阵 | 说明 |
| --- | --- | --- | --- | --- |
| 2026-08-02 | `88ed32e` | 313 项库测试、Clippy、格式检查、100 次连续截图和 4K 标注压力测试通过；Release 启动 p95 为 264.24ms（上限 500ms），快捷键到覆盖层 p95 为 96.38ms（上限 100ms）。 | 待执行 | 当前机器只提供单显示器压力测试证据；负坐标双屏、混合 DPI 和 FFmpeg 实机结果尚未记录。 |
| 2026-08-09 | `23d4c31` | 324 项库测试、严格 Clippy、格式检查和工作区全目标编译通过；`windows-acceptance-probe` 输出保存为 `target/windows-acceptance-environment-20260809.json`。 | 待执行 | 当前会话探测到一块 2560x1440、100% 显示器，FFmpeg 不可用；因此未把双屏、混合 DPI、录屏或完整单屏交互矩阵标为通过。 |
| 2026-08-09 | `32096da` | 326 项库测试、严格 Clippy、格式检查和设置页宽窄截图复核通过；用户级 FFmpeg 探测结果保存为 `target/windows-acceptance-environment-20260809-ffmpeg.json`。 | 待执行 | Capture/Library/Record/App 导航在 980x760 和 520x640 窗口中无截断或重叠，证据位于 `target/ui-acceptance/settings-p1-task-navigation-*.png`。FFmpeg 已探测到 `gdigrab`，但 MP4 录制矩阵尚未执行。 |
| 2026-08-09 | `c4e90f4` | 326 项库测试、严格 Clippy、格式检查和真实覆盖层截图复核通过。 | 待执行 | Undo/Redo 在无历史时保持固定低对比禁用态，Select/Mark 选中反馈清楚，主动作与样式控件未越出 2560x1440 屏幕；证据位于 `target/ui-acceptance/annotation-p1-stable-history-controls-*.png`。 |
| 2026-08-09 | `2c966f7` | 300 个真实 PNG 与 300 条历史索引完成调试构建重启验收；默认页面仅渲染 5 条预览。 | 待执行 | 隐藏窗口 998ms 内就绪，Library 自动打开后工作集约 68 MiB，数秒后保持稳定；截图显示 `Showing 5 of 300 captures`，证据位于 `target/ui-acceptance/history-p1-300-*.png`。该记录关闭历史长列表 P1 差距，不替代 Release 性能门禁。 |
| 2026-08-09 | `a7c9edb` | 327 项库测试、严格 Clippy、格式检查和全目标编译通过；`target/windows-acceptance-environment-20260809-ffmpeg9.json` 识别 FFmpeg 9.0 与 `gdigrab`。 | 待执行 | 受限 640x360 桌面区域已录制为 2 秒 H.264 MP4，并由 `ffprobe` 验证后删除临时内容。此冒烟测试证明本机 FFmpeg 可生成 MP4；Flash Shot 应用内显示器、窗口、区域录制及暂停/恢复/停止矩阵仍待执行。 |
| 2026-08-09 | `cfeec74` | 331 项库测试、严格 Clippy、格式检查和全目标编译通过。 | 待执行 | 在 2560x1440、100% 显示器上创建 Arrow 后从 Layers 选中它；Selected 组保持 Delete/Duplicate/Arrange，展开 Arrange 后 Rotate 90 与全部层级命令仍清晰可点，面板翻到选区上方且没有遮挡标注。证据位于 `target/ui-acceptance/annotation-p1-selection-context-20260809.png` 与 `target/ui-acceptance/annotation-p1-arrange-context-20260809.png`。这关闭标注高密度上下文的 P1 差距，不替代窄选区或混合 DPI 的 P0 验收。 |
| 2026-08-10 | `e71230a` | 332 项库测试、严格 Clippy、格式检查和浅色主题语义色对比度门禁通过；环境探测保存为 `target/windows-acceptance-environment-20260810.json`。 | 待执行 | 浅色主题的 muted、accent、success 前景色均调整到在 background/panel 上至少 4.5:1。本机会话仍只有一块 2560x1440、100% 显示器，未执行 150%/200% 或新的浅色主题截图，因此设置页 P1 实机项保持待执行。 |
| 2026-08-10 | `c3dd07a` | Release 原生设置页截图探针在深色/浅色与 980x760/520x640 组合运行成功；332 项库测试、严格 Clippy、格式检查和全目标编译通过。 | 待执行 | 深色 980x760、浅色 980x760 和浅色 520x640 均无文字截断、控件重叠或导航跳动，证据位于 `target/ui-acceptance/settings-p1-dark-980x760-20260810.png`、`target/ui-acceptance/settings-p1-light-980x760-20260810.png` 与 `target/ui-acceptance/settings-p1-light-520x640-20260810.png`。深浅主题 100% 项通过；本机没有 150%/200% 环境，设置页 P1 仍不整体标为通过。 |
| 2026-08-10 | `79fbd77` | 333 项库测试、严格 Clippy、格式检查和 `recording-acceptance` 验收探针通过；显示器、窗口、区域三种目标均生成 H.264 MP4，并由 `ffprobe` 验证时长、编码和尺寸；暂停/恢复/停止事件均观察到，最终进度帧分别为 31、36、29，录制结束后无 FFmpeg 子进程。 | 待执行 | 生产录屏后端证据位于 `target/ui-acceptance/recording-p2-display-20260810-progress.json`、`target/ui-acceptance/recording-p2-window-20260810-progress.json` 与 `target/ui-acceptance/recording-p2-region-20260810-progress.json`；完整 UI 手工矩阵仍待执行。窗口验收使用可见的 `Flash Shot` 设置窗口，截图证据为 `target/ui-acceptance/recording-window-settings-progress.png`。 |
| 2026-08-10 | `88724b1` | 当前提交的 Release `recording-acceptance` 探针完成显示器、区域、窗口三种目标；三份 MP4 均由 `ffprobe` 验证可读，暂停/恢复事件均观测到，最终进度帧分别为 32、29、32。 | 待执行 | 报告位于 `target/ui-acceptance/recording-p2-current-display.json`、`target/ui-acceptance/recording-p2-current-region.json` 与 `target/ui-acceptance/recording-p2-current-window.json`；三次运行结束后没有残留 FFmpeg/FFprobe 进程。应用内完整录屏 UI 手工矩阵仍待执行。 |
| 2026-08-10 | `7f27ca4` | 334 项库测试、严格 Clippy、格式检查和工作区全目标编译通过；滚动控制器新增未修饰 `Escape` 取消回归测试。 | 待执行 | 滚动截图已有首帧、手动/辅助追加、重叠校验、失败重试和拼接导出链路；控制窗口获得焦点后按 `Escape` 会取消会话并清理临时状态。滚动与 OCR/翻译的真实 UI 手工矩阵仍待执行。 |
| 2026-08-10 | `68aad53` | 334 项库测试、严格 Clippy、格式检查、全目标编译和 Release 原生截图探针通过。 | 待执行 | `settings-p1-scale-metadata-520x640-20260810.png` 的同名 JSON 报告 DPI 96、缩放 1.0，并记录窗口物理边界。探针现已能为未来 150%/200% 运行保留可审计的缩放证据；当前机器没有该环境，设置页 P1 实机项仍待执行。 |
| 2026-08-10 | `52cdb90` | 当前提交的 Release 原生设置页截图探针在深色 980x760 与浅色 520x640 窗口成功运行。 | 待执行 | 人工检查 `target/ui-acceptance/settings-p1-current-dark-980x760.png` 与 `target/ui-acceptance/settings-p1-current-light-520x640.png`：100% DPI 下未发现文字截断、按钮重叠或底部状态栏遮挡；浅色窄窗切换为顶部导航并保持主要动作可达。150%/200% 缩放仍待真实 Windows 环境验收。 |
| 2026-08-10 | `3ae9a2e` | `settings-ui-acceptance` 新增 `expected-scale` 守卫与 6 项探针测试；334 项库测试、严格 Clippy、格式检查和全目标编译通过。 | 待执行 | 当前 100% 环境要求 `1.5` 时命令按预期失败，并在 JSON 中记录 `scale_factor: 1.0`、`scale_match: false`；未来 150%/200% 运行必须以 `scale_match: true` 作为机器可读门禁。 |
| 2026-08-10 | `f5ac5e4` | 334 项库测试、严格 Clippy、格式检查和全目标编译通过；OCR、翻译和二维码请求统一推进 operation generation。 | 待执行 | 新识别请求会清理旧结果，晚到的异步结果不会覆盖当前请求；OCR/翻译真实外部环境和失败后 UI 重试仍待手工矩阵。 |
| 2026-08-10 | `edb5e48` | 334 项库测试、严格 Clippy、格式检查和全目标编译通过；`scroll-acceptance` 确定性滚动拼接验收探针通过。 | 待执行 | 6 帧合成视口使用 90 像素重叠，输出 96x630，5 个重叠区均匹配，像素校验和为 `2267123376996061824`；报告写入 `target/ui-acceptance/scroll-acceptance-20260810.json`。这补充滚动算法的机器可读证据，不替代 OCR/翻译/滚动真实 UI 手工矩阵。 |
| 2026-08-10 | `6c09c39` | 334 项库测试、严格 Clippy、格式检查、全目标编译通过；Release 便携包构建、SHA-256、manifest 校验和启动冒烟通过。 | 待执行 | 首次启动遇到不可用的历史目录时会自动回退到托管目录或应用数据目录，并清除失效的首选路径；`FlashShot-0.1.0-windows-x86_64.zip` 实测保持运行 5 秒。真实签名与干净 Windows profile 仍待执行。 |
| 2026-08-10 | `6c09c39` | Release `scroll-acceptance` 确定性滚动拼接验收探针通过。 | 待执行 | 6 帧合成视口使用 90 像素重叠，输出 96x630，5 个重叠区均匹配，像素校验和为 `2267123376996061824`；当前报告写入 `target/ui-acceptance/scroll-acceptance-20260810-current.json`。这证明滚动合成链路可用，不替代滚动真实 UI 手工矩阵。 |
| 2026-08-10 | `P1-420x420` | Release 原生设置页截图探针在深色/浅色最小 `420x420` 窗口成功运行；设置探针 6 项单测通过。 | 待执行 | `settings-p1-min-dark-420x420-20260810.png` 与 `settings-p1-min-light-420x420-20260810.png` 均显示紧凑导航、主要动作和固定状态栏；内容区保持可滚动且没有可见文字截断或控件重叠。证据的物理边界为 `436x459`、DPI 96；150%/200% 缩放仍待真实 Windows 环境验收。 |
| 2026-08-10 | `P1-420x420-current` | 当前 Release 原生设置页截图探针在深色最小 `420x420` 窗口运行成功；物理边界 `436x459`、DPI 96、`scale_match: true`。 | 待执行 | 视觉复核 `target/ui-acceptance/settings-p1-current-dark-420x420-20260810.png`：顶部 Capture/Library/Record/App 导航、Capture 主动作和状态栏均可见，未发现文字截断或控件重叠；150%/200% 缩放仍待真实 Windows 环境验收。 |
| 2026-08-10 | `77137fd` | 当前 Release 设置页探针支持按 `capture|library|record|app` 选择初始页面；336 项库测试、严格 Clippy、格式检查和全目标编译通过。 | 待执行 | Record 页 520x640 截图 `target/ui-acceptance/recording-p2-current-settings-20260810.png` 已复核：显示器、音频、支持检查、录制按钮和状态栏均可见且无重叠；真实录制 UI 的暂停/恢复/停止交互仍待手工矩阵。 |
| 2026-08-10 | `recording-ui-runtime` | 当前可见的 Flash Shot Record 页窗口由生产 `recording-acceptance` 录制并通过 `ffprobe` 校验；H.264、520x640、2.8 秒，暂停/恢复均观察到，最高进度帧 36，录制结束后无 FFmpeg/FFprobe 残留进程。 | 待执行 | MP4 与报告位于 `target/ui-acceptance/recording-p2-record-ui-20260810.mp4` 和 `target/ui-acceptance/recording-p2-record-ui-20260810.json`；这证明窗口目标与后端联通，不替代应用内完整显示器/区域/窗口 UI 手工矩阵。 |
| 2026-08-10 | `recording-metadata` | `recording-acceptance` 报告升级为 schema 2，并在探针内部解析和校验 FFprobe 的视频编码、尺寸与时长；区域目标 Release 运行及全量质量门禁通过。 | 待执行 | `target/ui-acceptance/recording-p2-region-20260810-metadata.json` 自包含 `codec_name: h264`、`width: 640`、`height: 360`、`duration_seconds: 2.4`、暂停/恢复和最高进度帧 29；独立 `ffprobe` 输出一致，无残留进程。窗口 UI 和完整录屏矩阵仍待真实手工验收。 |
| 2026-08-10 | `recording-window-metadata` | 可见的 Record 设置窗口由生产 `recording-acceptance` 窗口目标录制；schema 2 报告、独立 `ffprobe` 和原生 UI 截图复核通过。 | 待执行 | `target/ui-acceptance/recording-p2-window-20260810-metadata.mp4` 与同名 JSON 报告为 H.264、520x640、2.8 秒，暂停/恢复均观察到，最高进度帧 35；`target/ui-acceptance/settings-window-background.png` 中 Record 导航、Display/Audio、Check support、Record display 和状态栏均可见且无重叠。完整录屏 UI 手工矩阵仍待执行。 |
| 2026-08-10 | `f33acdb` | Release 启动与覆盖层性能脚本改为至少 20 次采样；当前 20 次采样全部通过。 | 待执行 | `release-startup-performance-20260810-20.json` 的启动 p95 为 `233.21ms`（上限 500ms）；`release-capture-performance-20260810-default.json` 的 frame-ready p95 为 `56.14ms`、覆盖层 p95 为 `80.02ms`（上限均 100ms）。10 次采样曾受一次 `205.70ms` 离群值影响，现已禁止低于 20 次的门禁运行；真实混合 DPI p95 仍待硬件验收。 |
| 2026-08-10 | `8cbfac2` | 336 项库测试、严格 Clippy、格式检查、全目标编译通过；快速保存失效目录回退测试通过。 | 待执行 | 选择截图、全屏快捷保存和 Pin 保存会在首选历史目录不可写时回退到托管 `Pictures/Flash Shot`，成功后切换历史根并清除失效目录配置；真实不同 Windows profile 权限仍待手工验收。 |
| 2026-08-10 | `18de6de` | 336 项库测试、严格 Clippy、格式检查、全目标编译和 Release 构建通过；`scroll-acceptance` 通过 6 帧、5 个 90 像素重叠区的确定性拼接验收，输出 96x630，像素校验和为 `2267123376996061824`；本机 FFmpeg 9.0 可用并识别 `gdigrab`。 | 待执行 | 滚动控制器现在只接受未修饰的 `Escape` 取消，Ctrl/Alt/平台键/Fn 组合不会误取消；报告写入 `target/ui-acceptance/scroll-acceptance-20260810-current.json`。真实滚动 UI、双屏/混合 DPI 和录屏手工矩阵仍待执行。 |
| 2026-08-10 | `e0c647c` | 滚动截图控制窗口沿用覆盖层的原生置顶策略；336 项库测试、严格 Clippy、格式检查、全目标编译和 Release `scroll-acceptance` 验收通过。 | 待执行 | 滚动控制条首次渲染后异步保持在目标窗口上方，避免滚动过程中被目标应用遮挡；确定性报告仍为 6 帧、5 个 90 像素重叠、96x630。真实滚动 UI、双屏/混合 DPI 和录屏手工矩阵仍待执行。 |
| 2026-08-10 | `036c450` | 336 项库测试、严格 Clippy、格式检查、全目标编译通过；Release `recognition-acceptance` 探针通过并输出 OCR/翻译可选依赖状态。 | 待执行 | 报告位于 `target/ui-acceptance/recognition-acceptance-20260810-current.json`：OCR 为 `program not found`，翻译端点未配置；探针不创建截图、不发起网络请求。OCR/翻译真实 UI 与失败后重试仍待手工矩阵。 |
| 2026-08-10 | `066feb6` | 337 项库测试、严格 Clippy、格式检查、全目标编译和 Release 设置页探针通过；新增零基显示器索引定位。 | 待执行 | 当前机器仅有 `DISPLAY1`（2560x1440、DPI 96、scale 1.0）；索引 `0` 的 Capture 520x640 截图位于 `target/ui-acceptance/settings-display-0-current-520x640.png`，JSON 报告 `scale_match: true`。索引 `1` 按预期失败并报告仅检测到 1 块显示器；150%/200% 实机仍待执行。 |
| 2026-08-10 | `711951f` | 当前提交的便携包通过结构校验、SHA-256 生成、5 秒启动冒烟和 fixture 正/反例门禁；安装器 `-ValidateOnly` 通过。 | 待执行 | 便携包证据位于 `dist/acceptance-711951f/FlashShot-0.1.0-windows-x86_64.zip`；启动后进程保持运行 5 秒并被测试脚本清理。当前环境没有 `ISCC.exe`/`signtool.exe`，真实安装器编译、签名和干净 Windows profile 仍待执行。 |
| 2026-08-10 | `e5e69e0` | 339 项库测试、严格 Clippy、格式检查和工作区全目标编译通过；录屏设置页新增启动、录制、暂停状态与 FFmpeg 秒数/帧数反馈。 | 待执行 | Release 原生 Record 页 520x640 截图 `target/ui-acceptance/recording-progress-idle-520x640.png` 已复核，空闲布局无文字截断或控件重叠；启动/暂停/实时进度文案由单测覆盖。应用内完整录屏 UI 手工矩阵仍待执行。 |
| 2026-08-10 | `40a3126` | 340 项库测试、严格 Clippy、格式检查和工作区全目标编译通过；OCR、翻译和二维码请求新增忙状态保护，重复点击不会启动第二个异步任务覆盖首个结果。 | 待执行 | 识别请求冲突文案由回归测试覆盖；OCR/翻译真实依赖与失败后 UI 重试仍待手工矩阵。 |
| 2026-08-10 | `042fe15` | 340 项库测试、严格 Clippy、格式检查和工作区全目标编译通过；选区工具栏的“更多操作”菜单在识别任务运行时显示 `Recognizing selection...` 状态行，并预留对应高度防止菜单遮挡工具栏。 | 待执行 | 识别进行中状态的布局逻辑和菜单高度回归测试已通过；OCR/翻译真实 UI 与失败后重试仍待手工矩阵。 |
| 2026-08-10 | `current-P1-acceptance` | 当前 Release 探针串行运行通过：滚动拼接 6 帧、5 个 90 像素重叠区，输出 96x630 且像素校验和匹配；环境探测识别单屏 2560x1440、DPI 96、FFmpeg 9.0，并确认 `gdigrab`、窗口和区域输入可用。 | 待执行 | 当前 100% 环境下重新截图复核深色 Capture 520x640 与浅色 Record 520x640，未发现文字截断、控件重叠或状态栏遮挡；真实滚动 UI、150%/200% 缩放、双屏和录屏完整 UI 矩阵仍待执行。证据位于 `target/ui-acceptance/settings-current-dark-520x640.png`、`target/ui-acceptance/settings-current-light-520x640.png`、`target/ui-acceptance/scroll-acceptance-current.json` 和 `target/windows-acceptance-environment-20260810-current.json`。 |
| 2026-08-10 | `197d520` | 341 项库测试、严格 Clippy、格式检查、工作区全目标编译和 Release `scroll-acceptance` 验收通过；滚动控制窗在捕获期间临时隐藏，取消/关闭时推进 generation，避免控制窗进入帧或晚到结果污染新会话。 | 待执行 | 6 帧、5 个 90 像素重叠区的输出仍为 96x630，像素校验和为 `2267123376996061824`；真实滚动 UI 与双屏/混合 DPI 手工矩阵仍待执行。 |
| 2026-08-10 | `4f3b53d` | 341 项库测试、严格 Clippy、格式检查和 Release 主程序构建通过；滚动控制窗增加聚焦键盘动作。 | 待执行 | 聚焦控制器时 `Space` 捕获当前帧、`Shift+Space` 执行滚动并捕获、`Enter` 完成、未修饰 `Escape` 取消；Ctrl/Alt/Cmd/Fn 组合不会误触发。回归测试覆盖快捷键映射；真实滚动 UI、双屏/混合 DPI 和录屏手工矩阵仍待执行。 |
| 2026-08-10 | `075a325` | 341 项库测试、严格 Clippy、格式检查和 Release 主程序构建通过；辅助滚动输入现在恢复用户原始鼠标位置。 | 待执行 | 滚动前保存光标位置，滚轮注入成功或失败后均尝试恢复；溢出校验在移动光标前执行，避免异常路径留下鼠标副作用。真实滚动 UI、双屏/混合 DPI 和录屏手工矩阵仍待执行。 |
| 2026-08-10 | `current-20260810-ui` | 当前 Release 原生设置窗口顺序运行深色 Capture 与浅色 Record 520x640 截图，滚动拼接 6 帧验收和 FFmpeg 区域录制验收均通过。 | 待执行 | 截图证据位于 `target/ui-acceptance/settings-current3-dark-520x640.png` 与 `target/ui-acceptance/settings-current3-light-520x640.png`，两张图均为 DPI 96、scale 1.0，未发现文字截断、控件重叠或状态栏遮挡；`scroll-acceptance-current2.json` 输出 96x630 且像素校验和匹配；`recording-current2-region.json` 为 H.264 640x360、2.4 秒并观察到暂停/恢复。150%/200% 缩放、双屏和完整录屏 UI 手工矩阵仍待执行。 |
| 2026-08-10 | `recording-current3-window` | 当前 Release `recording-acceptance` 录制可见 Record 设置窗口成功；报告 schema 2、H.264、520x640、2.8 秒，暂停/恢复均观察到，最高进度帧 36。 | 待执行 | 截图 `target/ui-acceptance/recording-current3-window-settings.png` 经目视复核，Record 导航、Display/Audio、Check support、Record display 和状态栏均可见且无重叠；MP4 与报告位于 `target/ui-acceptance/recording-current3-window.mp4`、`target/ui-acceptance/recording-current3-window.json`。完整录屏 UI 手工矩阵、双屏和高 DPI 仍待执行。 |
| 2026-08-10 | `57980ca` | 341 项库测试、严格 Clippy、格式检查、Release 主程序构建和滚动控制器回归测试通过；辅助滚动按钮明确显示 `Scroll down + capture`，等待态显示 `Scrolling...`。 | 待执行 | 文案与当前固定的向下滚轮方向一致，避免用户误判滚动方向；真实滚动 UI、双屏/混合 DPI 和录屏手工矩阵仍待执行。 |
| 2026-08-10 | `a71bf72` | 343 项库测试、严格 Clippy、格式检查、便携包构建和验证通过；便携启动冒烟现在使用隔离 profile 并确认进程保持运行 5 秒。 | 待执行 | `dist/acceptance-profile/FlashShot-0.1.0-windows-x86_64.zip` 通过结构与 SHA-256 校验；`FLASH_SHOT_PROFILE_DIR` 下的 `config/data/cache/history` 均成功创建，调用方原有 profile 环境变量在冒烟结束后恢复；便携包 fixture 与安装器 `-ValidateOnly` 也通过。真实干净 Windows 用户账户、签名和安装器仍待发布时执行。 |
| 2026-08-10 | `7034a07` | `recognition-acceptance` 探针升级为 schema 2；默认模式只读探测并保持非阻塞，`--require-ocr` 与 `--require-translation` 可分别把依赖就绪状态作为机器可读退出门禁。 | 待执行 | 当前默认报告 `target/ui-acceptance/recognition-acceptance-default-current.json` 中 OCR 为 `program not found`、翻译端点未配置且 `passed: true`；显式 `--require-ocr` 与 `--require-translation` 分别在 `target/ui-acceptance/recognition-acceptance-required-ocr-current.json`、`target/ui-acceptance/recognition-acceptance-required-translation-current.json` 写入 `passed: false` 并以退出码 1 结束。未安装 Tesseract 或未配置翻译服务时，不将 OCR/翻译真实 UI 矩阵标为通过。 |
| 2026-08-10 | `current-ocr-timeout` | 347 项库测试、严格 Clippy、格式检查和工作区全目标编译通过；本地 OCR 子进程增加 20 秒有界等待，正常退出会回收输出，超时会终止进程并返回 `TimedOut`，相关生命周期回归测试通过。 | 待执行 | 这补充 OCR 失败恢复的自动证据，不替代含文字选区的真实 OCR UI、翻译服务失败后重试和滚动截图手工矩阵；当前机器仍未安装 Tesseract，真实依赖门禁保持未通过。 |
| 2026-08-10 | `current-update-timeout` | 更新 manifest 请求增加 15 秒连接与总超时，并保留传输超时错误类型；更新配置与 manifest 校验测试通过，严格 Clippy、格式检查和全目标编译通过。 | 待执行 | 这是显式更新检查的失败恢复自动证据；当前未配置 `FLASH_SHOT_UPDATE_ENDPOINT`，因此未进行外部发布端点实测。 |
| 2026-08-10 | `current-ffmpeg-probe-timeout` | 350 项库测试、严格 Clippy、格式检查和工作区全目标编译通过；FFmpeg 版本、格式、设备和音频枚举探测增加 10 秒有界等待，卡住的探测进程会被终止并回收。 | 待执行 | 这补充录屏环境检查的自动失败恢复证据；本机 FFmpeg 9.0 的三种录制目标仍由既有 `recording-acceptance` 报告验证，完整录屏 UI 手工矩阵仍待执行。 |
| 2026-08-10 | `recording-acceptance-ffprobe-timeout` | `recording-acceptance` 的 MP4 元数据校验增加 10 秒 FFprobe 超时与终止回收；探针测试、全量库测试、全目标编译和严格 Clippy 通过。 | 待执行 | 这是验收工具自身的失败恢复保护，不替代三种录制目标的当前 MP4/FFprobe 证据和完整录屏 UI 手工矩阵。 |
| 2026-08-10 | `current-P1-settings-sections-520x640` | Release `settings-ui-acceptance` 串行生成深色/浅色 Capture、Library、Record、App 四页截图；当前 100% DPI 下八张截图均通过目视检查，未发现文字截断、控件重叠或状态栏遮挡。 | 待执行 | 截图位于 `target/ui-acceptance/settings-p1-current-dark-capture-520x640.png`、`settings-p1-current-dark-library-520x640.png`、`settings-p1-current-dark-record-520x640.png`、`settings-p1-current-dark-app-520x640.png` 及对应浅色文件；150%/200% 缩放、双屏和混合 DPI 仍待真实环境验收。 |
| 2026-08-10 | `current-P1-settings-sections-420x420` | Release `settings-ui-acceptance` 串行生成深色/浅色 Capture、Library、Record、App 四页最小窗口截图；紧凑导航、首屏内容和固定状态栏边界均通过目视检查，长页面的后续控件保留在可滚动内容区。 | 待执行 | 截图位于 `target/ui-acceptance/settings-p1-current-dark-capture-420x420.png`、`settings-p1-current-dark-library-420x420.png`、`settings-p1-current-dark-record-420x420.png`、`settings-p1-current-dark-app-420x420.png` 及对应浅色文件；150%/200% 缩放、双屏和混合 DPI 仍待真实环境验收。 |
| 2026-08-10 | `current-scroll-portable-20260810` | 当前主线滚动截图专项测试 19 项通过；Release `scroll-acceptance` 通过 6 帧、5 个 90 像素重叠区的确定性拼接，输出 96x630，像素校验和为 `2267123376996061824`；最新 Windows 便携包结构、SHA-256、隔离 profile 启动冒烟、fixture 和安装器配置校验均通过。 | 待执行 | 滚动截图可从选区工具栏的更多操作启动，支持手动追加、辅助滚动追加、失败重试、完成拼接和导出；便携包 `target/portable-current/FlashShot-0.1.0-windows-x86_64.zip` 的 SHA-256 为 `b1f0cf1fd09373bf5c6870f756affcd64b2cdb2d67acbaebae1338641e677d6a`（见同目录 `.sha256`，校验值完整记录在文件中）。真实滚动 UI、150%/200% 缩放、双屏和完整录屏 UI 手工矩阵仍待执行。 |
| 2026-08-10 | `current-environment-followup` | 当前 Release 环境探针通过：检测到单块 `DISPLAY1`（2560x1440、DPI 96、scale 1.0），FFmpeg 9.0 可用并支持 `gdigrab`、窗口和区域输入；Tesseract 5.5.3 已安装并通过 `FLASH_SHOT_TESSERACT` 指向 `C:\Program Files\Tesseract-OCR\tesseract.exe`；识别就绪探针的 `--require-ocr` 门禁通过。 | 待执行 | 机器可读证据位于 `target/windows-acceptance-environment-20260810-followup.json`、`target/ui-acceptance/recognition-acceptance-ocr-ready.json` 和 `target/ui-acceptance/recognition-acceptance-ocr-ready-default.json`。翻译端点未配置；OCR/翻译真实 UI、150%/200% 缩放、双屏和完整录屏 UI 仍不标记为通过。 |
| 2026-08-10 | `current-ocr-fixture` | Release `recognition-acceptance` 使用含文字的 `settings-current-dark-520x640.png` 执行完整 PNG -> Tesseract OCR；schema 3 报告中 `ocr.available`、`ocr_exercise.passed` 和 `--require-ocr` 均通过，识别文本只保留长度元数据（391 个字符）。 | 待执行 | 报告位于 `target/ui-acceptance/recognition-acceptance-ocr-fixture.json`。这证明本地 OCR 调用链可用，不替代含文字选区的真实 UI 复核、翻译服务失败后重试或滚动截图手工矩阵。 |
| 2026-08-10 | `current-ocr-auto-discovery` | OCR 可执行文件发现现在保留 `FLASH_SHOT_TESSERACT` 覆盖，并自动尝试 Windows 常见安装路径；在未设置该环境变量且 `tesseract` 不在 PATH 的当前机器上，Release `recognition-acceptance --require-ocr --ocr-image` 自动发现 `C:\Program Files\Tesseract-OCR\tesseract.exe`，schema 4 的 OCR 检查与真实 PNG -> Tesseract 练习均通过，文本长度为 391。 | 待执行 | 报告位于 `target/ui-acceptance/recognition-acceptance-ocr-auto-discovery.json`。这关闭了本机 OCR 依赖配置缺口，但不替代含文字选区的真实 UI 复制与翻译服务手工矩阵。 |
| 2026-08-10 | `current-scroll-acceptance-20260810` | 当前 Release `scroll-acceptance` 重新通过 6 帧、5 个 90px 重叠区的确定性滚动拼接，输出 96x630，像素校验和为 `2267123376996061824`。 | 待执行 | 报告位于 `target/ui-acceptance/scroll-acceptance-20260810-current3.json`；这确认滚动截图的重叠匹配、失败可重试和最终拼接链路可用，但真实目标应用滚动 UI、150%/200% 缩放和双屏仍需手工矩阵。 |
| 2026-08-10 | `current-ui-acceptance-4` | 当前 Release 设置页按 Capture、Library、Record、App 顺序串行生成深色/浅色 520x640 截图，并额外复核深色 Library 420x420 最小窗口；100% DPI 下导航、主动作、状态栏和滚动内容均保持可达，没有发现文字截断或控件重叠。 | 待执行 | 截图证据位于 `target/ui-acceptance/settings-current4-dark-capture-520x640.png`、`settings-current4-dark-library-520x640.png`、`settings-current4-light-record-520x640.png`、`settings-current4-light-app-520x640.png` 和 `settings-current4-dark-library-420x420.png`；150%/200% 缩放、双屏和覆盖层真实交互仍需手工矩阵。 |
| 2026-08-10 | `current-release-gates-4` | 当前 Release 工作区全目标构建通过；`scroll-acceptance` 通过 6 帧、5 个 90px 重叠区，`recognition-acceptance --require-ocr --ocr-image` 通过真实 PNG -> Tesseract 调用链（schema 3，文本长度 371）。 | 待执行 | 报告位于 `target/ui-acceptance/scroll-acceptance-20260810-current4.json` 与 `target/ui-acceptance/recognition-acceptance-ocr-current4.json`；翻译端点未配置，滚动/OCR 的真实 UI 手工矩阵仍需执行。 |
| 2026-08-10 | `current-overlay-single-100` | 在当前单屏 2560x1440、DPI 96 环境启动 Release 主程序，触发真实区域截图并拖出 600x496 px 选区；覆盖层显示一组紧靠选区的 `Mark / Pin / Copy / Save / More / Cancel` 操作栏，展开 `Mark` 后工具与样式面板仍保持在选区下方且未被屏幕边界截断；按 `Escape` 取消并清理覆盖层。 | 待执行 | 截图位于 `target/ui-acceptance/manual-overlay-single-100.png` 与 `target/ui-acceptance/manual-overlay-single-100-marking.png`。本次已证明覆盖层与标记工具的实际布局，复制、保存、Pin 的完整操作链仍需在同一手工矩阵中继续完成。 |
| 2026-08-10 | `current-scroll-ui-single-100` | 当前单屏 2560x1440、DPI 96 环境中，从选区工具栏的 `More -> Scroll shot` 启动真实滚动截图；独立的 `Flash Shot - Scrolling Screenshot` 控制条保持在目标 Notepad 窗口上方，显示首帧状态，点击 `Scroll down + capture` 后变为 `2 frames - ready to finish` 并报告 `420 px overlap`，点击 `Finish` 后进入 `Flash Shot - Edit Image`。 | 待执行 | 截图位于 `target/ui-acceptance/manual-scroll-more-menu-open.png`、`target/ui-acceptance/manual-scroll-controller-start.png`、`target/ui-acceptance/manual-scroll-controller-after-auto-2.png` 与 `target/ui-acceptance/manual-scroll-finished.png`。本次证明入口、置顶控制条、自动追加和拼接后编辑页的真实交互；完整导出、双屏/混合 DPI 与其他录屏手工矩阵仍待执行。 |
| 2026-08-10 | `current-overlay-edge-single-100` | 当前单屏 2560x1440、DPI 96 环境中，在右下屏幕边缘创建 `530x476 px` 选区；主操作栏自动放到选区上方且保持完整可点击，展开 `Mark` 后标注工具与颜色/粗细面板仍在安全区域内；点击 `Copy` 后剪贴板实际得到 `530x476` 图片。 | 待执行 | 截图位于 `target/ui-acceptance/manual-overlay-edge-selection.png` 与 `target/ui-acceptance/manual-overlay-edge-marking.png`，复制后的屏幕状态位于 `target/ui-acceptance/manual-overlay-edge-copy-confirmed.png`。本次证明边缘布局和复制像素尺寸；Pin 生命周期、双屏/混合 DPI 与发布矩阵仍待执行。 |
| 2026-08-10 | `current-ocr-ui-single-100` | 当前单屏 2560x1440、DPI 96 环境中，从选区工具栏的 `More -> OCR` 启动真实 Tesseract OCR；显式提供本机 `C:\Program Files\Tesseract-OCR\tesseract.exe` 后，菜单显示 `Recognized text` 结果预览，状态显示 `Text recognized locally`，点击 `Copy text` 后剪贴板实际得到 674 个字符。首次未继承用户环境变量时，选区保持不变并显示 `Retry OCR`，随后重启并重试成功。 | 待执行 | 截图位于 `target/ui-acceptance/manual-ocr-more-menu.png`、`target/ui-acceptance/manual-ocr-result.png` 与 `target/ui-acceptance/manual-ocr-result-configured.png`。本次证明本地 OCR、失败保留选区、重试和结果复制链路；翻译服务、双屏/混合 DPI 与完整录屏矩阵仍待执行。 |
| 2026-08-10 | `current-save-folder-preflight` | 快速保存目录预检新增 4 项库测试，验证可写目录在创建、同步和删除私有探针文件后保持为空，并拒绝非目录路径；用户通过文件夹选择器切换历史根目录前也会先运行相同预检；Release 原生深色/浅色 Library 520x640 截图复核通过。 | 待执行 | `Check folder` 位于当前快速保存路径与文件夹选择之间，空闲时可点击、检测中会禁用以避免重复请求；截图为 `target/ui-acceptance/settings-p2-storage-check-520x640.png` 和 `target/ui-acceptance/settings-p2-storage-check-light-520x640.png`。不同 Windows profile 的真实权限矩阵仍待执行。 |
| 2026-08-10 | `current-recording-stop-state` | 录屏停止请求新增独立 `Stopping` 状态；停止收尾期间 Record 页显示 `Stopping...`、隐藏 Pause、禁用重复停止和 Display/Audio/Check support 切换，收到 Finished/Failed 后才恢复空闲。状态文案、冲突优先级、录屏模块 27 项测试、全目标测试和严格 Clippy 通过。 | 待执行 | Release Record 页深色/浅色 520x640 截图位于 `target/ui-acceptance/recording-stop-state-dark-520x640.png` 与 `target/ui-acceptance/recording-stop-state-light-520x640.png`；截图复核空闲布局，停止态由状态回归测试覆盖。完整 FFmpeg UI 手工矩阵仍待执行。 |
| 2026-08-10 | `current-recording-discovery-guard` | 录屏显示器/音频源枚举现在绑定 workflow generation；枚举期间 Record 页锁定互相冲突的录屏动作，过期结果不会覆盖新会话或正在录制的状态；相关回归测试、全量测试、严格 Clippy 和 Release 设置页截图均通过。 | 待执行 | `target/ui-acceptance/recording-discovery-lock-520x640.png` 的 Record 页面在 100% DPI 下无文字截断或控件重叠；枚举忙状态与过期结果由自动测试覆盖。双屏、混合 DPI 和完整录屏 UI 手工矩阵仍待执行。 |
| 2026-08-10 | `current-translation-service-test` | 设置页翻译操作改为显式 `Test service`：未配置端点时保持本地、可操作提示；配置 HTTPS 端点后才发送固定 `Flash Shot` 探测短语，成功只显示返回字符数，失败保留端点检查恢复提示。聚焦状态格式测试通过，Release 设置页截图复核通过。 | 待执行 | 截图位于 `target/ui-acceptance/settings-p2-translation-service-520x640.png`。当前机器未配置翻译端点，因此真实成功响应仍需在用户服务环境执行；探测不会发送截图原文。 |
| 2026-08-10 | `e393f5d` | 当前 Release `recording-acceptance` 串行完成显示器、区域、窗口三种目标录制；三份 MP4 均由探针内部及 FFprobe 校验为 H.264，暂停/恢复事件均观察到，最终进度帧分别为 32、29、36，录制结束后未残留 FFmpeg、FFprobe 或验收窗口进程。 | 待执行 | 报告位于 `target/ui-acceptance/recording-p2-current3-display.json`、`recording-p2-current3-region.json` 和 `recording-p2-current3-window.json`；显示器输出为 2560x1440、2.53 秒，区域输出为 640x360、2.4 秒，窗口输出为 520x640、2.8 秒。窗口截图 `target/ui-acceptance/recording-p2-current3-window-settings.png` 复核 Record 导航、Display/Audio、Check support、Record display 和状态栏无截断或重叠；完整应用内录屏 UI、双屏和混合 DPI 手工矩阵仍待执行。 |
| 2026-08-10 | `current-recording-p3-20260810` | 当前 Release `recording-acceptance` 重新完成显示器、区域、窗口三种目标；三份 MP4 均由探针内部及 FFprobe 校验为 H.264，暂停/恢复/停止事件均观察到，最终进度帧分别为 31、30、35，录制结束后没有残留 FFmpeg、FFprobe 或验收窗口进程。 | 待执行 | 报告位于 `target/ui-acceptance/recording-p3-display-20260810.json`、`recording-p3-region-20260810.json` 和 `recording-p3-window-20260810.json`；输出分别为 2560x1440、640x360、520x640，时长约 2.53s、2.4s、2.8s。窗口截图 `target/ui-acceptance/recording-p3-window-settings-2.png` 已复核 Record 导航、Display/Audio、Check support、Record display 和状态栏无截断或重叠；双屏、混合 DPI 与完整应用内录屏手工矩阵仍待执行。 |
| 2026-08-10 | `current-scroll-acceptance-final-20260810` | 当前 Release `scroll-acceptance` 重新通过 6 帧、5 个 90px 重叠区的确定性拼接，输出 96x630，像素校验和为 `2267123376996061824`；本机 FFmpeg 9.0 可用并识别 `gdigrab`。 | 待执行 | 报告位于 `target/ui-acceptance/scroll-acceptance-20260810-final.json`；滚动控制器真实 UI 截图 `target/ui-acceptance/manual-scroll-controller-after-auto-2.png` 显示 `2 frames - ready to finish`、`420 px overlap`、`Scroll down + capture`、`Finish` 和 `Cancel` 均完整可见。该证据确认滚动入口、辅助追加、状态反馈和拼接链路；双屏、混合 DPI 与完整录屏 UI 手工矩阵仍待执行。 |
| 2026-08-10 | `current-recording-ui-lifecycle-single-100` | 当前单屏 2560x1440、DPI 96 环境使用 Release `settings-ui-acceptance` 依次渲染 Starting、Recording、Paused、Stopping；每张截图均报告 `scale_match: true`。 | 待执行 | 已人工复核 `target/ui-acceptance/recording-ui-starting-single-100.png`、`recording-ui-recording-single-100.png`、`recording-ui-paused-single-100.png` 和 `recording-ui-stopping-single-100.png`：Preparing/Stopping 禁用冲突动作，Recording 显示 Stop/Pause 与进度，Paused 显示 Stop/Resume 与相同进度，文本未截断或重叠。这是无 FFmpeg 的确定性 UI 证据，不替代完整录屏手工矩阵。 |
| 2026-08-10 | `current-translation-test-busy-single-100` | 当前单屏 2560x1440、DPI 96 环境使用 Release `settings-ui-acceptance` 注入 Translation 测试忙状态，不发起网络请求；截图报告 `scale_match: true`。 | 待执行 | 已人工复核 `target/ui-acceptance/translation-service-testing-single-100.png`：Capture 页底部 Translation 行显示 `Testing...`，状态栏显示 `Testing translation service...`，二者均完整可读且不与固定状态栏重叠。此证据验证设置页反馈布局，不替代真实 HTTPS 端点成功/失败验收。 |

本次自动证据保存为本机未跟踪的 `target\\capture-stress-20260802.json`、
`target\\release-startup-performance-20260802.json` 与
`target\\release-capture-performance-20260802.json`。这些文件可用于复核本表中的数值，
但不得替代缺失的真实双屏和录屏手工证据。
