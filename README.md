# Flash Shot

<p align="center">
  <img src="resources/branding/logo-horizontal.svg" alt="Flash Shot" width="560">
</p>

**中文** | [English](README_EN.md)

Flash Shot 是一款使用 Rust 和 [GPUI](https://www.gpui.rs/) 构建的高性能原生截图与录屏工具。项目以 Snow Shot 已验证的实用工作流作为需求参考，但不继承其 Tauri、WebView、React 和定制 Excalidraw 架构。

项目优先支持 Windows。启动速度、截图延迟、混合 DPI 正确性、稳定性以及可预测的资源生命周期，都是产品验收要求，而不是后期优化项。

## 当前状态

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

## 下载与安全提示

预编译版本只通过 [GitHub Releases](https://github.com/bruceblink/flash-shot/releases) 发布。
当前 Windows 安装器和便携版有意保持未签名，因此项目不需要、也不会在 GitHub Release 工作流中
配置 `WINDOWS_SIGNING_CERTIFICATE_BASE64` 或 `WINDOWS_SIGNING_CERTIFICATE_PASSWORD`。
Windows 可能显示 SmartScreen 或“未知发布者”警告；请只从上述页面下载，核对随附的 SHA-256
校验文件和发布清单，并仅在接受该风险后继续运行。

## 运行

环境要求：

- Rust 1.95 或更高版本；
- Visual Studio 2022 MSVC 工具链与 Windows SDK；
- GPUI 在目标平台所需的原生构建环境。

```powershell
cargo run
```

workspace 根目录将 `crates/flash-shot-bin` 中的 `flash-shot` 包设为唯一默认成员，且只声明一个可运行程序入口，
因此无需指定 `--bin`。压力测试和原生验收程序
作为可选的 `dev-tools` 库模块保留，由 `scripts\run-dev-tool.ps1` 调度；它们不会成为发布包中的
额外 EXE，也不会改变普通 `cargo run` 的启动行为。

源码按职责拆成小型 workspace crate：`flash-shot-domain` 保存纯领域模型，`flash-shot-image` 保存不可变
截图帧与图像处理，`flash-shot-infra-windows` 负责显示器枚举、Windows 屏幕捕获、全局快捷键、托盘、剪贴板、自启动、目录打开、进程生命周期、窗口控制、光标定位和辅助滚轮输入，`flash-shot-app` 负责
应用用例、GPUI 和基础设施装配，`crates/flash-shot-bin` 是唯一启动入口。
除该入口外，其余 crate 都是库，不会被 `cargo run` 当作额外程序启动。

首次启动时，界面语言会读取 Windows 的首选 UI 语言；目前支持的简体中文区域会自动使用简体中文，
其他语言回退为英文。之后在设置中手动切换的语言会保存为用户偏好，不会再被系统语言覆盖。

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

快速保存默认写入 `Pictures\Flash Shot\FlashShot<yyyyMMddHHmmssSSS><UUIDv7>.png`。在 `Files` 中选择
`Choose folder` 可将快速保存与其受限历史记录一起切换到新目录；`File name` 可在安全
前缀间切换，生成的名称始终包含本地时间戳和 UUIDv7 以避免覆盖已有截图。

## 验证

```powershell
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

连续截图资源与延迟门禁会真实捕获并编码虚拟桌面 100 次，输出机器可读 JSON；性能基线应使用 release 构建：

```powershell
.\scripts\run-dev-tool.ps1 -Release capture-stress --output target/capture-stress.json
```

开发时可使用较小轮数快速验证工具链：

```powershell
.\scripts\run-dev-tool.ps1 capture-stress --iterations 5
```

验证 Library 历史缩略图的有界资源流控时，使用 Release 构建展开 300 条隔离 fixture。报告会记录
默认 5 条预览、显式展开后的缓存/加载/失败/队列峰值、工作集与私有提交，以及 fixture 清理结果；解码并发上限为 2：

```powershell
.\scripts\run-dev-tool.ps1 -Release history-resource-acceptance --output-dir target\history-resource-acceptance\release-gate-current
```

需要覆盖损坏/缺失文件、预览重试和历史目录切换时，在同一 Release 会话追加故障场景：

```powershell
.\scripts\run-dev-tool.ps1 -Release history-resource-acceptance --exercise-failures `
  --output-dir target\history-resource-acceptance\release-fault
```

该场景会在报告中记录两个失败条目、恢复后的 300 条缓存、3 条新目录记录、故障截图和全部临时 history 根目录的清理结果。

使用固定 4K 场景测量 CPU 导出合成器。该指标衡量导出性能，不是 GPUI 交互帧门禁；
只有在建立具有代表性的 Release 基线后，才应显式设置上限：

```powershell
.\scripts\run-dev-tool.ps1 -Release annotation-stress --iterations 30
.\scripts\run-dev-tool.ps1 -Release annotation-stress --iterations 30 --max-p95-ms 80
```

在不打开截图或录屏会话的情况下，采集 Windows 手工验收记录所需的显示器几何、DPI 和
FFmpeg 能力快照：

```powershell
.\scripts\run-dev-tool.ps1 -Release windows-acceptance-probe --output target/windows-acceptance-environment.json
```

在单显示器交互桌面上，可运行隔离的三 Pin 生命周期门禁。它不会注册托盘或全局快捷键，
复制动作使用内存剪贴板，并将保存、报告和原生截图全部写入 `target`。门禁按实测原生边界
进行 DPI 感知排布，验证 Show all 保持当前焦点，并用全流程超时防止原生验收挂起：

```powershell
.\scripts\check-pin-lifecycle-acceptance.ps1
```

需要复核 Pin 的界面国际化和主题组合时，使用同一 Release 构建串行生成 English/简体中文与深色/浅色四个
隔离会话；每份 schema 5 报告会记录并校验实际 locale/theme：

```powershell
.\scripts\check-pin-lifecycle-matrix.ps1 -OutputDirectory target\pin-lifecycle-u4-release -TimeoutMilliseconds 30000 -SettleMilliseconds 700
```

需要持续观察同一组三个 Pin 时，可显式运行 60 秒 soak。它会轮流执行生产 Solo/Show all，
持续校验窗口注册表、不可变源帧、原生边界、前台焦点和 Capture preflight，并记录工作集与私有
提交量；该模式仍不注入全局输入或写入系统剪贴板：

```powershell
.\scripts\check-pin-lifecycle-acceptance.ps1 -SoakMilliseconds 60000 -TimeoutMilliseconds 90000 -SettleMilliseconds 500 -OutputDirectory target\pin-lifecycle-soak-60s
```

## 文档

- [产品需求](docs/requirements.md)
- [架构设计](docs/architecture.md)
- [开发计划](docs/plan.md)
- [开发流程与验收证据](docs/development-workflow.md)
- [Windows 手工验收记录](docs/windows-manual-acceptance.md)
- [Windows 分发](docs/windows-distribution.md)
- [Linux 平台可行性验证](docs/linux-platform-validation.md)

## 许可证

GNU Affero General Public License v3.0 only (`AGPL-3.0-only`).
