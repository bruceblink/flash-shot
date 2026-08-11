# Flash Shot

<p align="center">
  <img src="resources/branding/logo-horizontal.svg" alt="Flash Shot" width="560">
</p>

[中文](#中文) | [English](#english)

## 中文

Flash Shot 是一款使用 Rust 和 [GPUI](https://www.gpui.rs/) 构建的高性能原生截图与录屏工具。项目以 Snow Shot 已验证的实用工作流作为需求参考，但不继承其 Tauri、WebView、React 和定制 Excalidraw 架构。

项目优先支持 Windows。启动速度、截图延迟、混合 DPI 正确性、稳定性以及可预测的资源生命周期，都是产品验收要求，而不是后期优化项。

### 当前状态

当前主线已完成 Windows 原生截图、标注、滚动截图和基于 FFmpeg 的录屏工作流。工程骨架参考了 `synchub-desktop` 与 `hiposter` 中经过验证的精简入口、模块组织、Tokio 后台运行时和原生资源打包方式。界面直接使用原生 GPUI，不依赖 `gpui-component`。

当前基线将 `gpui` 和官方 `gpui_platform` 启动模块锁定到同一个经过验证的 Zed 提交，不使用 crates.io 上较旧的 GPUI 版本，也不引入第三方组件库。

当前已包含：

- Windows 虚拟桌面截图、混合 DPI 选区、窗口/控件识别、放大镜和键盘微调；
- 延时截图、可选系统光标合成、复制、保存、贴图和截图历史；
- 原生矩形、椭圆、直线、箭头、画笔、文字、模糊、马赛克、高亮、水印和序号标注；
- 手动、辅助和滚动后自动捕获的长截图，二维码识别、可选本地 OCR 与 HTTPS 翻译及失败后原选区重试；
- 显示器、窗口和区域录制，以及音频选择、暂停/恢复、进度与 Job Object 清理；
- 可重复性能/资源压力工具、结构化诊断和本地质量门禁。

录屏后端和单屏 100% 下显示器、窗口与区域的录屏 UI 生命周期验收已完成；窗口录制在
移动、缩放、遮挡和最小化后的固定源边界与桌面合成像素语义也已通过 Release 原生探针。
生成 MP4 需要安装支持 `ddagrab` 或 `gdigrab` 的 FFmpeg；150%/200% 缩放仍保留在真实
环境矩阵中。

### 运行

环境要求：

- Rust 1.92 或更高版本；
- Visual Studio 2022 MSVC 工具链与 Windows SDK；
- GPUI 在目标平台所需的原生构建环境。

```powershell
cargo run
```

应用启动后默认只驻留在通知区域，不显示常驻操作窗口。使用全局快捷键即可进入截图选区；只有在选区出现后才会显示标注和导出工具。单击或右键点击托盘图标都会打开菜单，可开始自由区域截图、全屏截图、3/5/10 秒延时截图，或直接将全屏复制到剪贴板，也可开始或停止显示器录制；菜单还可切换截图是否包含鼠标指针、打开截图目录、本地图片、可编辑项目、历史记录和按需设置窗口，并在用户明确点击时检查更新。关闭设置窗口只会将其隐藏，应用会继续在后台运行。

`Capture preferences` 可为区域截图、全屏截图和焦点窗口截图分别配置全局快捷键。三个
动作使用一次原生注册，重复组合会被拒绝；将附加动作切换到 `Off` 可释放对应快捷键。

录屏依赖用户本机或随应用分发的 FFmpeg。默认视频保存到 `Videos\Flash Shot`，不可写时回退到 Flash Shot 的应用数据目录。可通过以下环境变量显式指定 FFmpeg、视频目录和可选音频源：

```powershell
$env:FLASH_SHOT_FFMPEG = "C:\\tools\\ffmpeg.exe"
$env:FLASH_SHOT_RECORDING_DIRECTORY = "D:\\Recordings\\Flash Shot"
$env:FLASH_SHOT_RECORDING_MICROPHONE = "Microphone (USB Audio Device)"
# 或者，仅在 FFmpeg 探测到 WASAPI 时：
$env:FLASH_SHOT_RECORDING_SYSTEM_AUDIO = "default"
```

`FLASH_SHOT_RECORDING_MICROPHONE` 与 `FLASH_SHOT_RECORDING_SYSTEM_AUDIO` 不能同时设置；未设置时录制无音频。

Record 页的 `Video folder` 会显示当前 MP4 目录，并提供选择、恢复默认、可写性检查和打开目录操作。用户选择会持久化；`FLASH_SHOT_RECORDING_DIRECTORY` 存在时仍作为管理员或便携环境的最高优先级覆盖。录制文件遇到相同毫秒时间戳会自动追加编号，FFmpeg 不会覆盖已有 MP4。

主窗口的 `Audio` 按钮会在用户点击后后台发现本机 FFmpeg 支持的输入，并轮换自动配置、关闭、DirectShow 麦克风以及可用的 WASAPI 系统声音。`auto` 保持上述环境变量兼容行为；选择 `off` 会明确禁用音频。

`Display` 按钮会按主显示器优先顺序轮换可录制显示器；显示器选择、音频选择都只在点击时查询系统，不影响应用启动。

快速保存默认写入 `Pictures\Flash Shot\FlashShot-<timestamp>.png`。在 `Files` 中选择
`Choose folder` 可将快速保存与其受限历史记录一起切换到新目录；`File name` 可在安全
前缀间切换，生成的名称始终包含时间戳以避免覆盖已有截图。

### 验证

```powershell
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

连续截图资源与延迟门禁会真实捕获并编码虚拟桌面 100 次，输出机器可读 JSON；性能基线应使用 release 构建：

```powershell
cargo run --release --bin capture-stress -- --output target/capture-stress.json
```

开发时可使用较小轮数快速验证工具链：

```powershell
cargo run --bin capture-stress -- --iterations 5
```

Measure the CPU export compositor against a fixed 4K scene. This is an export
measurement rather than a GPUI interaction-frame gate; use an explicit limit
only after establishing a representative release baseline:

```powershell
cargo run --release --bin annotation-stress -- --iterations 30
cargo run --release --bin annotation-stress -- --iterations 30 --max-p95-ms 80
```

Collect the display geometry, DPI and FFmpeg capability snapshot used by the Windows manual
acceptance record without opening a capture or recording session:

```powershell
cargo run --release --bin windows-acceptance-probe -- --output target/windows-acceptance-environment.json
```

在单显示器交互桌面上，可运行隔离的三 Pin 生命周期门禁。它不会注册托盘或全局快捷键，
复制动作使用内存剪贴板，并将保存、报告和原生截图全部写入 `target`。门禁按实测原生边界
进行 DPI 感知排布，验证 Show all 保持当前焦点，并用全流程超时防止原生验收挂起：

```powershell
.\scripts\check-pin-lifecycle-acceptance.ps1
```

### 文档

- [产品需求](docs/requirements.md)
- [架构设计](docs/architecture.md)
- [开发计划](docs/plan.md)
- [Windows 截图技术验证报告](docs/windows-capture-validation.md)
- [Windows 手工验收记录](docs/windows-manual-acceptance.md)
- [Windows 分发](docs/windows-distribution.md)
- [Linux 平台可行性验证](docs/linux-platform-validation.md)

## English

Flash Shot is a high-performance native screenshot and screen recording application built with Rust and [GPUI](https://www.gpui.rs/). It uses Snow Shot's proven workflows as product research without inheriting its Tauri, WebView, React, or customized Excalidraw architecture.

The project is Windows-first. Startup time, capture latency, mixed-DPI correctness, stability, and predictable resource ownership are acceptance requirements rather than later optimization work.

### Status

The current mainline implements native Windows capture, annotation, scrolling capture, and an FFmpeg-based recording workflow. The engineering shell borrows the thin entry point, module layout, Tokio background runtime, and native resource packaging patterns proven in `synchub-desktop` and `hiposter`. The UI uses GPUI directly without `gpui-component`.

The baseline pins `gpui` and the official `gpui_platform` launcher to the same reviewed Zed commit. It uses neither the older crates.io GPUI release nor a third-party component suite.

The repository currently includes:

- virtual-desktop capture, mixed-DPI selection, window/control inspection, magnification, and keyboard nudging on Windows;
- delayed capture, optional system-cursor compositing, copy/save, pinning, and screenshot history;
- native rectangle, ellipse, line, arrow, pen, text, blur, mosaic, highlight, watermark, and sequence annotations;
- manual, assisted, and scroll-then-auto-capture workflows, QR recognition, optional local OCR, HTTPS translation, and in-overlay retries after a failure;
- display, window, and region recording with audio selection, pause/resume, progress, and Job Object cleanup;
- repeatable performance/resource stress tooling, structured diagnostics, and local quality gates.

The recording backend and the display, window, and region UI lifecycles have passed acceptance on
one 100%-scaled display. Fixed-source desktop-composition semantics after moving, resizing,
occluding, and minimizing a recorded window are also covered by a native Release probe. Producing
an MP4 requires an FFmpeg build that supports `ddagrab` or `gdigrab`; 150%/200% scaling remains in
the real-environment matrix.

### Run

Requirements:

- Rust 1.92 or newer;
- Visual Studio 2022 MSVC toolchain and Windows SDK;
- native build prerequisites required by GPUI on the target platform.

```powershell
cargo run
```

Recording uses a local or bundled FFmpeg. Videos go to `Videos\Flash Shot` by default and fall back to Flash Shot's application-data directory when that folder is unavailable. Set `FLASH_SHOT_FFMPEG` and optionally `FLASH_SHOT_RECORDING_DIRECTORY` to override the executable and output directory. The `Audio` control discovers supported local FFmpeg inputs on demand and cycles between automatic configuration, off, DirectShow microphones, and available WASAPI system audio. `auto` preserves the environment-variable behavior documented above.

The Record page's `Video folder` control shows the active MP4 destination and lets users choose, reset, verify, or open it. A chosen folder is persisted; `FLASH_SHOT_RECORDING_DIRECTORY` remains the authoritative override for managed or portable environments. When timestamped names collide, Flash Shot appends a suffix and FFmpeg refuses to overwrite an existing MP4.

The `Display` control cycles recordable monitors in primary-first order. Both display and audio discovery happen only after the respective control is clicked, keeping startup free of FFmpeg probing.

The capture shortcut defaults to `Ctrl+Shift+Print Screen`. Set `FLASH_SHOT_CAPTURE_HOTKEY` before launch to use a different safe global combination, for example `Ctrl+Alt+S`, `Shift+F12`, or `Ctrl+PrintScreen`. It must include `Ctrl`, `Alt`, or `Shift`, plus one letter, `F1` through `F24`, or `PrintScreen`; invalid values fall back to the default.

The global capture shortcut can be disabled from the tray `System` menu or Capture settings without changing its configured key combination. The preference persists across restarts, while the tray capture commands remain available.

Capture settings can configure separate global shortcuts for region capture, full-screen capture,
and focused-window capture. They are registered as one native set; duplicate combinations are
rejected, and setting a secondary action to `Off` releases that key.

Use `Capture > Focused window` to hide Flash Shot, resolve the current external foreground window,
and open that physical-pixel rectangle as an editable selection. If the target is partly outside the
virtual desktop, the selection is clipped to captured pixels rather than producing an invalid crop.

`Start with Windows` configures only the current user's sign-in entry. It does not require elevation, and it never removes a `Flash Shot` startup entry that points to a different executable.

Use `Files > Pin clipboard image` from the tray menu to turn the current Windows clipboard image into an always-on-top reference without opening the settings window or starting a new capture.

Set `FLASH_SHOT_UPDATE_ENDPOINT` to an HTTPS URL serving a verified `release-manifest.json` to enable the optional `Check Updates` button. No update request, download, or installation happens unless the user explicitly clicks the button; see [Windows distribution](docs/windows-distribution.md#manual-update-check) for the manifest contract.

### Validate

```powershell
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The repeat-capture resource and latency gate captures and encodes the virtual desktop 100 times and emits machine-readable JSON. Use a release build for performance baselines:

```powershell
cargo run --release --bin capture-stress -- --output target/capture-stress.json
```

Use fewer iterations for a quick development smoke test:

```powershell
cargo run --bin capture-stress -- --iterations 5
```

The app also retains the latest 500 startup and shortcut-to-overlay samples at
`<application data directory>\metrics\performance.jsonl`. Summarize a representative release
run and enforce the default p95 thresholds with:

```powershell
cargo run --release --bin performance-report -- --input "<application data directory>\metrics\performance.jsonl" --output target/performance-summary.json
```

The command accepts only Release-profile samples by default, so Debug runs and legacy unmarked
records cannot affect a release p95 decision. Its JSON reports `release_gate_applied: true` only
when all three default p95 gates use Release samples, and `release_qualified: true` only when
those gates pass. It exits with status `2` when a threshold fails, and with status `1` when
samples are malformed or insufficient. `--no-gate` and `--include-nonrelease` produce
exploratory reports with both fields set to `false`.

Collect an isolated twenty-startup Release baseline without mixing older samples into the result:

```powershell
.\scripts\measure-release-startup.ps1
```

This starts the Release executable twenty times, writes a time-windowed startup-only p95 report to
`target\release-startup-performance.json`, and stops on a startup crash, a missing current sample,
or a startup p95 above 500 ms. It deliberately does not claim full release qualification because
the shortcut-to-overlay metrics need their own real interaction sampling.

Use `-SkipBuild` only when both Release binaries were built from the current source; the script
checks the reporter protocol before it starts sampling.

Collect an isolated shortcut-to-overlay baseline with the dedicated `Ctrl+Alt+F12` hotkey:

```powershell
.\scripts\measure-release-capture.ps1
```

The script starts one Release application, triggers and cancels twenty real capture overlays, and
gates the current window's frame-ready and overlay p95 values at 100 ms. It requires an
interactive Windows desktop and no existing Flash Shot process.

Exercise the complete real single-display screenshot path in an isolated profile. The default mode
drags and nudges a selection, expands and collapses More, recaptures and cancels, drives the native
Save dialog into an isolated path, opens and closes a Pin, then clicks Copy through a process-local
image sink. It maps the actual pointer path through the measured client area, checks the committed
physical selection, compares Save/Pin/Copy pixels with their pre-click source frames, and proves
the Windows clipboard sequence did not change:

```powershell
cargo run --release --bin overlay-interaction-acceptance -- --allow-input --output-dir target/overlay-interaction-acceptance
```

Use the opt-in narrow-edge scenario to open the real 420x420 minimum Settings client, drag a
160x96 selection against the bottom-right display edge, and click More/Less and Mark open/close.
It requires one 100%-scaled display, records the exact selection pixels and UI states, and remains
mutually exclusive with recording modes:

```powershell
cargo run --release --bin overlay-interaction-acceptance -- --allow-input --capture-scenario narrow-edge --output-dir target/overlay-interaction-narrow-edge-acceptance
```

Use the opt-in Pin coexistence scenario to create three 360x240 Pins through real toolbar clicks,
arrange them without overlap, move one through the native image drag surface, and trigger Capture
from a focused Pin. It requires one 100%-scaled display, proves all three source frames and HWND
bounds survive the overlay and Cancel, then closes every Pin with Escape without using the system
clipboard:

```powershell
cargo run --release --bin overlay-interaction-acceptance -- --allow-input --capture-scenario pins-coexist --output-dir target/overlay-interaction-pins-coexist-acceptance
```

Use the opt-in selection-transform scenario to perform real committed-selection gestures in one
overlay: move from the selection interior, resize the bottom-right corner, resize with Shift to
preserve the aspect ratio, and resize with Alt around the original center. It requires one
100%-scaled display, records the actual pointer endpoints and committed physical rectangles, and
cleans up the overlay and Settings controller without touching the system clipboard:

```powershell
cargo run --release --bin overlay-interaction-acceptance -- --allow-input --capture-scenario selection-transform --output-dir target/overlay-interaction-selection-transform-acceptance
```

The recording modes click `Record area` or `Record window`, then the real Pause, Resume, and Stop
buttons, and accept the run only after FFprobe validates the finalized H.264 MP4. These commands
move the global pointer, so run them only in a disposable interactive desktop session. The runner
also requires pause progress to remain frozen, compares the app's source bounds with an independent
area/window oracle, and decodes a video frame for content comparison with a desktop reference. It
forces output into its disposable session and clears inherited recording-audio overrides:

```powershell
cargo run --release --bin overlay-interaction-acceptance -- --allow-input --record-target area --output-dir target/overlay-recording-interaction-acceptance
cargo run --release --bin overlay-interaction-acceptance -- --allow-input --record-target window --output-dir target/overlay-recording-interaction-acceptance
```

On a single-display interactive desktop, run the isolated three-Pin lifecycle gate without
registering a tray icon or global shortcuts and without writing to the system clipboard. The gate
uses measured native bounds for DPI-aware layout, verifies that Show all preserves focus, and
bounds the complete native lifecycle with a watchdog:

```powershell
.\scripts\check-pin-lifecycle-acceptance.ps1
```

### Documentation

- [Product requirements (Chinese)](docs/requirements.md)
- [Architecture (Chinese)](docs/architecture.md)
- [Delivery plan (Chinese)](docs/plan.md)
- [Linux platform feasibility validation (Chinese)](docs/linux-platform-validation.md)

## License

GNU Affero General Public License v3.0 only (`AGPL-3.0-only`).
