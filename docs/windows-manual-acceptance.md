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
| 负坐标双屏 | 待执行 | 将副屏放在主屏左侧，跨屏拖动选区并在两屏边缘调整大小。 | 选区和放大镜不跳变；每个显示器只显示自己的覆盖层操作区；导出结果没有偏移或裁切错误。 | |
| 混合 DPI 双屏 | 待执行 | 将两个显示器设置为不同缩放，例如 100% 与 150% 或 200%；跨屏截图、保存并核对像素尺寸。 | 光标、选区、窗口智能识别和导出均以物理像素对齐；无重复或缺失的工具栏。 | |
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
cargo run --release --bin settings-ui-acceptance -- light 980 760 target/ui-acceptance/settings-light-980x760.png
cargo run --release --bin settings-ui-acceptance -- light 520 640 target/ui-acceptance/settings-light-520x640.png
cargo run --release --bin settings-ui-acceptance -- dark 980 760 target/ui-acceptance/settings-dark-980x760.png
```

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

本次自动证据保存为本机未跟踪的 `target\\capture-stress-20260802.json`、
`target\\release-startup-performance-20260802.json` 与
`target\\release-capture-performance-20260802.json`。这些文件可用于复核本表中的数值，
但不得替代缺失的真实双屏和录屏手工证据。
