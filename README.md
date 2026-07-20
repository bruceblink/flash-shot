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
- 手动与辅助滚动截图、二维码识别、可选本地 OCR 与 HTTPS 翻译；
- 显示器、窗口和区域录制，以及音频选择、暂停/恢复、进度与 Job Object 清理；
- 可重复性能/资源压力工具、结构化诊断和本地质量门禁。

录屏后端和测试已完成；生成 MP4 仍需要在安装了支持 `ddagrab` 或 `gdigrab` 的 FFmpeg 环境中做手工验收。

### 运行

环境要求：

- Rust 1.92 或更高版本；
- Visual Studio 2022 MSVC 工具链与 Windows SDK；
- GPUI 在目标平台所需的原生构建环境。

```powershell
cargo run
```

应用启动后默认只驻留在通知区域，不显示常驻操作窗口。使用全局快捷键即可进入截图选区；只有在选区出现后才会显示标注和导出工具。单击或右键点击托盘图标都会打开菜单，可开始自由区域截图、全屏截图、3/5/10 秒延时截图，或直接将全屏复制到剪贴板，也可开始或停止显示器录制；菜单还可切换截图是否包含鼠标指针、打开截图目录、本地图片、可编辑项目、历史记录和按需设置窗口，并在用户明确点击时检查更新。关闭设置窗口只会将其隐藏，应用会继续在后台运行。

录屏依赖用户本机或随应用分发的 FFmpeg。默认视频保存到 `Videos\Flash Shot`（不可用时回退到当前目录）。可通过以下环境变量显式指定可执行文件和一个可选音频源：

```powershell
$env:FLASH_SHOT_FFMPEG = "C:\\tools\\ffmpeg.exe"
$env:FLASH_SHOT_RECORDING_MICROPHONE = "Microphone (USB Audio Device)"
# 或者，仅在 FFmpeg 探测到 WASAPI 时：
$env:FLASH_SHOT_RECORDING_SYSTEM_AUDIO = "default"
```

`FLASH_SHOT_RECORDING_MICROPHONE` 与 `FLASH_SHOT_RECORDING_SYSTEM_AUDIO` 不能同时设置；未设置时录制无音频。

主窗口的 `Audio` 按钮会在用户点击后后台发现本机 FFmpeg 支持的输入，并轮换自动配置、关闭、DirectShow 麦克风以及可用的 WASAPI 系统声音。`auto` 保持上述环境变量兼容行为；选择 `off` 会明确禁用音频。

`Display` 按钮会按主显示器优先顺序轮换可录制显示器；显示器选择、音频选择都只在点击时查询系统，不影响应用启动。

快速保存默认写入 `Pictures\Flash Shot\FlashShot-<timestamp>.png`。可通过安全文件名前缀自定义命名：

```powershell
$env:FLASH_SHOT_SAVE_PREFIX = "Release_Notes"
```

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

### 文档

- [产品需求](docs/requirements.md)
- [架构设计](docs/architecture.md)
- [开发计划](docs/plan.md)
- [Windows 截图技术验证报告](docs/windows-capture-validation.md)
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
- manual and assisted scrolling capture, QR recognition, optional local OCR, and HTTPS translation;
- display, window, and region recording with audio selection, pause/resume, progress, and Job Object cleanup;
- repeatable performance/resource stress tooling, structured diagnostics, and local quality gates.

The recording backend and its automated tests are complete. Producing an MP4 still needs manual acceptance with an FFmpeg build that supports `ddagrab` or `gdigrab`.

### Run

Requirements:

- Rust 1.92 or newer;
- Visual Studio 2022 MSVC toolchain and Windows SDK;
- native build prerequisites required by GPUI on the target platform.

```powershell
cargo run
```

The `Audio` control discovers supported local FFmpeg inputs on demand and cycles between automatic configuration, off, DirectShow microphones, and available WASAPI system audio. `auto` preserves the environment-variable behavior documented above.

The `Display` control cycles recordable monitors in primary-first order. Both display and audio discovery happen only after the respective control is clicked, keeping startup free of FFmpeg probing.

The capture shortcut defaults to `Ctrl+Shift+Print Screen`. Set `FLASH_SHOT_CAPTURE_HOTKEY` before launch to use a different safe global combination, for example `Ctrl+Alt+S`, `Shift+F12`, or `Ctrl+PrintScreen`. It must include `Ctrl`, `Alt`, or `Shift`, plus one letter, `F1` through `F24`, or `PrintScreen`; invalid values fall back to the default.

The global capture shortcut can be disabled from the tray `System` menu or Capture settings without changing its configured key combination. The preference persists across restarts, while the tray capture commands remain available.

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

Collect an isolated ten-startup Release baseline without mixing older samples into the result:

```powershell
.\scripts\measure-release-startup.ps1
```

This starts the Release executable ten times, writes a time-windowed startup-only p95 report to
`target\release-startup-performance.json`, and stops on a startup crash, a missing current sample,
or a startup p95 above 500 ms. It deliberately does not claim full release qualification because
the shortcut-to-overlay metrics need their own real interaction sampling.

Use `-SkipBuild` only when both Release binaries were built from the current source; the script
checks the reporter protocol before it starts sampling.

Collect an isolated shortcut-to-overlay baseline with the dedicated `Ctrl+Alt+F12` hotkey:

```powershell
.\scripts\measure-release-capture.ps1
```

The script starts one Release application, triggers and cancels ten real capture overlays, and
gates the current window's frame-ready and overlay p95 values at 100 ms. It requires an
interactive Windows desktop and no existing Flash Shot process.

### Documentation

- [Product requirements (Chinese)](docs/requirements.md)
- [Architecture (Chinese)](docs/architecture.md)
- [Delivery plan (Chinese)](docs/plan.md)
- [Linux platform feasibility validation (Chinese)](docs/linux-platform-validation.md)

## License

GNU Affero General Public License v3.0 only (`AGPL-3.0-only`).
