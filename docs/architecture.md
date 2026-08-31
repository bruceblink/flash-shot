# 开发设计思路

更新日期：2026-08-31

## 术语表与命名约定

| 规范名称 | English / 缩写 | 当前职责边界 | 不代表什么 |
| --- | --- | --- | --- |
| 工作区根目录 | Workspace Root | 虚拟 Cargo workspace，统一依赖、默认成员、版本和仓库级检查 | 不是可运行包或第二个二进制入口 |
| 领域库 | Domain Crate / `flash-shot-domain` | 几何、选区、截图会话、标注文档和产品状态机等纯值与规则 | 不是 GPUI 界面、Windows API 或图像编码器 |
| 图像库 | Image Crate / `flash-shot-image` | 不可变截图帧、物理像素采样、裁切、标注合成、二维码识别和 PNG/JPEG/WebP 编码 | 不是 Windows 捕获设备或 GPUI 视图 |
| Windows 基础设施库 | Windows Infrastructure Crate / `flash-shot-infra-windows` | 显示器、捕获、快捷键、托盘、剪贴板、自启动、目录、进程、窗口、光标和辅助滚轮的 Windows 实现 | 不是应用用例、界面或组合根 |
| 应用库 | Application Crate / `flash-shot-app` | GPUI 装配、产品用例、持久化策略、状态反馈和迁移期兼容导出 | 不是 Cargo 应用入口或 Windows 服务 |
| 开发工具模块 | Development Tool Modules / `dev-tools` | 库内可选的 Release 验收、压力和资源探针，由唯一二进制调度 | 不是发布包中的独立 EXE 或普通用户入口 |
| 应用入口 | Application Entry / `flash-shot` | `crates/flash-shot-bin` 中唯一的二进制目标，负责启动桌面应用并装配具体服务 | 不是压力测试命令集合 |
| 界面层 | UI Surface | 当前位于 `flash-shot-app/src/app` 的 GPUI 页面、覆盖层、Pin 和设置视图；未来可按稳定边界提取 | 不是业务规则、平台实现或截图像素源 |
| 本地化资源 | Locale / `UiText` | 与 OCR/外部翻译无关的 English/简体中文 UI 资源和参数化模板 | 不是翻译服务响应或报告字段 |
| 截图会话 | Capture Session | 一次从触发采集到完成、取消或失败清理的用户操作范围 | 不是常驻窗口或历史记录条目 |
| 截图帧 | CaptureFrame | 由采集后端产生、供预览和导出的不可变物理像素数据 | 不是 GPUI 纹理或压缩文件 |
| 操作代次 | Operation Generation | 标识当前异步操作是否仍可向界面提交结果的递增值 | 不是版本号或历史条目序号 |
| 拆除屏障 | Teardown Barrier | 在窗口、任务、输入和临时资源清理完成前阻止下一次采集的状态检查 | 不是操作系统同步原语或持久化锁 |
| 资源所有者 | Resource Owner | 负责提交或释放窗口、文件、剪贴板写入和外部进程资源的当前操作 | 不是业务数据的持久化拥有者 |
| 界面度量 | ThemeMetrics | 跨页面共享的颜色语义、间距、尺寸和命中区参数 | 不是截图像素或平台 DPI 值 |
| 剪贴板提交检查点 | Clipboard Commit Checkpoint | `ClipboardCommitGate` 在不可逆剪贴板写入前提供的最后可取消边界 | 不是实际的系统剪贴板写入，也不是消费者确认 |
| 隔离剪贴板观察器 | Isolated Clipboard Observer | `dev-tools` 验收中接收候选 `CaptureFrame` 的进程内观察通道 | 不是生产系统剪贴板或外部应用消费者 |

正文、目录示例、代码和报告统一使用这些名称；标准协议、Cargo、GPUI、Windows 和 FFmpeg 保留标准大小写。

## 1. 设计目标与约束

Flash Shot 是 Windows-first 的原生 Rust/GPUI 截图与录屏工具。设计优先保证低延迟、物理像素正确、失败可恢复、
资源有界和 UI 可验证，不以拆分数量或新功能数量作为架构目标。

1. GPUI 只负责界面、输入和呈现，不拥有领域规则或 Windows API 细节。
2. 平台 API 藏在描述产品操作和错误的接口后，平台实现集中在 `flash-shot-infra-windows`。
3. 截图帧（`CaptureFrame`）是不可变像素所有者；预览、标注合成、复制和保存不得无谓地往返编码。
4. 长耗时工作在后台执行器中运行，带取消、操作代次/资源所有者检查和可观察失败状态。
5. 设置、历史和标注文档带版本；原子写入失败时保留可重试的旧状态。
6. 只保留一个 `flash-shot` 二进制，开发工具通过 `dev-tools` 特性和 `scripts/run-dev-tool.ps1` 调度。

## 2. 当前工作区与依赖方向

当前实现是五个 Cargo workspace 成员，根目录默认只选择 `crates/flash-shot-bin`：

```text
flash-shot-bin
  -> flash-shot-app
       -> flash-shot-infra-windows
            -> flash-shot-image
                 -> flash-shot-domain
       -> flash-shot-image
       -> flash-shot-domain
```

`flash-shot-bin` 是唯一组合根，负责单实例、诊断、设置/历史初始化、Windows 资源和 GPUI 启动。`flash-shot-app`
提供 UI、workflow、录屏、识别、历史和状态反馈，并通过兼容导出维持已有调用方。`flash-shot-infra-windows` 提供
原生实现；`flash-shot-image` 和 `flash-shot-domain` 不依赖 GPUI、HWND、COM、FFmpeg 或具体 OCR 运行时。

未来是否提取独立 `flash-shot-ui` 或 `flash-shot-acceptance`，取决于稳定的依赖和发布边界；当前先在应用库内按职责
拆分模块，不预先增加 crate 或二进制。开发工具继续作为库模块，避免把验收路径误发布为用户程序。

## 3. 数据与生命周期

### 3.1 截图主链

```text
全局快捷键或托盘命令
  -> 截图会话（Capture Session）
  -> 显示器提供器（DisplayProvider）/ 捕获后端（CaptureBackend）获取物理像素
  -> 一个不可变截图帧（CaptureFrame）
  -> GPUI 覆盖层选择、标注预览和操作状态
  -> 图像库（Image Crate）确定性合成与裁切
  -> 剪贴板服务（ClipboardService）/ 原子文件写入 / Pin / 可选 OCR
```

覆盖层只是一次截图会话的界面；设置窗口按需打开，关闭时只隐藏，不注销全局快捷键或托盘。Save、Copy、Pin、
Cancel 和再次 Capture 共享显式的拆除屏障（Teardown Barrier）：在原生窗口延迟关闭完成、后台任务归零且
`capture_preflight_ready=true` 前，不开始下一次全屏采集。

### 3.2 标注模型

标注文档使用逻辑图像坐标和稳定 ID。GPUI 渲染将文档坐标变换到视口，导出始终以原始截图帧的物理尺寸
合成。鼠标移动只产生草稿预览，正式提交才进入可撤销命令历史；取消、零尺寸和竞争手势不得写入历史。

### 3.3 异步所有权

识别、保存、历史缩略图、录屏发现/启动和滚动拼接都绑定操作代次或具体资源所有者。完成回调必须先确认：

- 请求仍属于当前会话、历史根目录和条目；
- 目标窗口、文件占用标记、剪贴板写入 slot 或 FFmpeg 子进程仍由当前操作拥有；
- 取消、关闭或新操作没有使结果过期。

过期结果只释放自己的资源，不写回新的 UI 状态。历史缩略图保持有界 FIFO 和最多两个并行解码任务；单条失败
显示可重试状态，不阻塞截图主链。

选择复制的后台 worker 先在图像库完成标注合成和物理像素裁切，再通过
`ClipboardService::copy_image_cancellable` 进入剪贴板提交检查点。`SelectionCopyCancellation` 接收第一个
Escape：在 `ClipboardCommitGate::begin_clipboard_commit` 成功前，取消只丢弃已准备的帧并释放选择复制与剪贴板写入的
操作所有权；提交开始后，取消只等待写入完成，不声称可以恢复已经改变的剪贴板。Windows `SystemClipboard` 在
`OpenClipboard` 成功后、`EmptyClipboard` 之前取得提交资格。`dev-tools` 的 `copy-cancellation-race` 使用隔离
剪贴板观察器在同一检查点暂停 worker，runner 通过真实 Copy 和 Escape 输入验证取消顺序，既不改变生产实现也不写入
系统剪贴板。

### 3.4 录屏与外部进程

FFmpeg 由录屏 workflow 管理，状态使用 `idle/starting/recording/paused/stopping/failed`。启动阶段先建立
Job Object/等价的进程边界和输出读取器；任何中途失败都终止并回收已创建子进程。正常停止优先发送控制输入，
超时才强制终止；stdout/stderr 始终被消费并限制错误详情，不能把外部输出原文写进用户状态或日志。

## 4. UI 设计原则

界面层使用统一的界面度量（`ThemeMetrics`）和语义颜色，而不是页面内散落固定颜色和尺寸：

- Surface：`canvas`、`surface`、`surface_elevated`、`surface_hover`；
- Content：`text`、`text_muted`、`text_disabled`；
- Action：`accent`、`accent_hover`、`accent_pressed`、`focus`；
- Status：`success`、`warning`、`danger`、`info`；
- Geometry：4px 间距基线、稳定 toolbar/control 命中区、圆角不超过 8px。

Capture 覆盖层保留 Copy、Save、Pin、Cancel 作为主要动作，滚动、OCR、录屏和诊断进入 More 或对应页面的次级区。
Library 以最近截图和筛选为主，Record 以当前目标和生命周期为主，App 以设置和全局状态为主，Pin 只保留图片与轻量
工具栏。每个区域最多一个视觉最强的主要动作；复制、保存、缩放、关闭等熟悉命令优先使用图标和 tooltip。

布局必须在 English/简体中文、浅色/深色和 420x420、520x640、980x760 下保持文本不截断、控件不重叠、焦点可见；
视觉截图只证明布局，像素、快捷键、窗口和资源清理仍由原生验收报告单独证明。

## 5. 平台边界

应用用例通过以下小接口表达系统能力，接口定义操作和产品错误，不逐一暴露 Windows API：

- `CaptureBackend`、`DisplayProvider`；
- `GlobalShortcutService`、`TrayService`；
- `ClipboardService`、`AutoStartService`；
- `WindowInspector`、窗口可见性控制和 `RecordingBackend`。

`flash-shot-app::platform` 只保留迁移期兼容导出。只有当应用用例不再直接依赖具体系统集成、接口约定测试和真实验收
能够独立运行时，才把剩余实现移动到 Windows 基础设施库。

## 6. 演进顺序

1. 先关闭剪贴板、文件系统、FFmpeg 和标注输入的失败/恢复证据，冻结当前行为与报告 schema。
2. 完成 `Locale`/`UiText` 动态状态盘点，再收敛 App、Library、Record 的信息层级和三种窗口尺寸。
3. 先拆原生验收 runner 的一个职责，再拆 `overlay.rs` 的一个职责；每步使用同一场景对照测试和 Release 报告。
4. 有真实硬件时执行 150%/200% 单显示器矩阵；双屏、真实翻译和跨平台另立范围，不混入当前切片。
5. 只有前述证据稳定且全量门禁通过，才准备 `v0.2.0` 候选版或评估 GPUI 依赖升级。

## 7. 验证策略

- 领域几何、会话、标注文档、命令和配置：纯单元测试；
- 图像合成、坐标和编码：golden image、物理像素和文件原子性测试；
- Windows 基础设施：接口约定测试、资源释放测试和真实桌面探针；
- GPUI 页面与工具栏：双语、双主题、三尺寸截图和真实键鼠验收；
- 录屏、剪贴板和历史：确定性故障 fixture 加同一 Release session 的进程/窗口/文件清理报告；
- workspace 根目录统一运行 `cargo fmt --all -- --check`、`cargo check --workspace --all-targets`、
  `cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace` 和 `git diff --check`。
- 使用 `dev-tools` 特性的检查还需运行 `cargo check --workspace --all-targets --all-features --locked` 和
  `cargo test --workspace --all-features --locked`；在 Windows 主机可用时再运行
  `cargo check -p flash-shot-app --target x86_64-pc-windows-msvc --all-targets --all-features --locked`。
  严格全特性 Clippy 的当前未完成项记录在主线计划的 P1 表中。

完成条件以 [主线开发计划](plan.md) 和 [Windows 手工验收记录](windows-manual-acceptance.md) 为准；本设计文档只
说明职责、依赖和演进规则，不复制逐次运行日志。
