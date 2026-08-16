# 架构设计

## 术语表与命名约定

| 规范名称 | English / 缩写 | 当前职责边界 | 不代表什么 |
| --- | --- | --- | --- |
| 工作区根目录 | Workspace Root | 虚拟 Cargo workspace，统一锁定依赖、默认成员和仓库级检查 | 不是可运行包或第二个二进制入口 |
| 领域库 | Domain Crate / `flash-shot-domain` | 几何、选区、会话和标注文档等纯产品值与状态机 | 不是 GPUI 界面或 Windows API 实现 |
| 图像库 | Image Crate / `flash-shot-image` | 不可变截图帧、像素坐标、标注合成、裁切、二维码识别和图像编码 | 不是 Windows 捕获设备或 GPUI 视图 |
| Windows 基础设施库 | Windows Infrastructure Crate / `flash-shot-infra-windows` | 显示器枚举、虚拟桌面边界、Windows 屏幕捕获、全局快捷键、托盘事件、剪贴板、自启动、目录打开、进程生命周期和窗口控制实现；后续承载其他原生服务 | 不是应用用例、GPUI 界面或第二个程序入口 |
| 应用库 | Application Crate / `flash-shot-app` | 当前承载 GPUI 应用、用例、平台适配与验收库模块；后续再按已批准设计细分 | 不是 Cargo 应用入口 |
| 应用入口 | Application Entry | Cargo 唯一的 `flash-shot` 二进制目标，负责启动桌面应用 | 不是压力测试或验收命令集合 |
| 开发工具模块 | Development Tool Modules / `dev-tools` | 仅在显式启用特性时编译的库内压力测试与验收模块 | 不是发布包中的独立 EXE，也不是普通用户入口 |
| 开发工具调度脚本 | Development Tool Runner | `scripts/run-dev-tool.ps1`，在隔离构建目录中选择并运行一个开发工具模块 | 不改变普通 `cargo run`，也不加入生产启动参数 |

正文、目录示例和命令统一使用上述名称；`flash-shot` 专指应用入口，具体开发工具使用其稳定模块名。

## 1. 设计原则

1. GPUI 是界面与交互层，不是业务数据模型。
2. 平台 API 必须隐藏在小而明确的接口之后，平台细节不能进入文档模型。
3. 截图像素必须具有明确的所有权，不做无意义的编码和复制。
4. 长耗时工作运行在后台执行器中，并支持取消。
5. 每条用户工作流使用显式状态机，失败状态必须可观察。
6. 设置和持久化文档从首次发布起就带版本。

## 2. 工程基线

应用壳选择性复用同级项目 `synchub-desktop` 和 `hiposter` 中已验证的工程习惯，并结合 Zed 官方当前示例：

- 使用锁定到具体 Zed 提交的 `gpui` 和官方 `gpui_platform`；
- 在 GPUI 事件循环启动前进入 Tokio 多线程运行时；
- 通过 `crates/flash-shot-bin/build.rs` 和根目录 `resources` 管理原生图标与打包资源；
- `crates/flash-shot-bin/src/main.rs` 保持精简，在 `flash-shot-app` 库完成应用装配，功能状态拆入独立模块；
- 使用 `Context`、`Entity`、`WeakEntity`、`spawn` 和 `notify` 组织 UI 状态与异步更新。

Flash Shot 不依赖 `gpui-component` 或 `gpui-component-assets`。截图覆盖层和标注工具需要直接控制布局、输入、绘制、焦点和帧行为；通用组件库会增加不必要的升级和样式边界。可复用控件将以小型、产品专用的 GPUI 模块实现。

`gpui_platform` 属于当前官方 GPUI workspace，只负责构造对应操作系统的原生平台实现，不是组件或样式依赖。`gpui` 与 `gpui_platform` 必须锁定到同一提交。升级提交是一个需要独立编译、交互和性能验证的功能，不能无审查地跟随 Git 主线。

### 2.1 运行界面模型

Flash Shot 不是常驻主窗口应用。进程启动后仅保留全局快捷键、托盘和后台任务；默认不显示 GPUI 窗口。截图由快捷键或托盘的截图命令开始，完整桌面帧准备完成后才创建覆盖层，选择框、标注工具和导出操作都属于该一次性截图会话。

设置使用单独的按需窗口，托盘菜单负责打开它。关闭设置窗口只隐藏窗口并返回后台，不得退出进程、注销快捷键或停止托盘。只有托盘的退出命令和应用级 Quit 操作可以结束整个生命周期。这样截图主路径不会被设置、历史和录屏选项占据，也不会在截图时出现普通应用窗口。

## 3. 当前模块边界

项目当前是一个小型 workspace。第一阶段已将无 GPUI、无 Windows API 依赖的领域模型提取为
`flash-shot-domain`；第二阶段已将不可变截图帧和图像处理提取为 `flash-shot-image`，并由
`flash-shot-app` 保留兼容导出，避免调用方在同一切片中发生行为变化。后续 crate 边界与迁移顺序见
[Workspace crate 迁移设计](workspace-crate-design.md)：

```text
Cargo.toml                                  虚拟 workspace；默认只选择 flash-shot-bin
crates/flash-shot-domain/src/domain/        几何、选区、会话、标注和路线模型
crates/flash-shot-image/src/frame.rs        不可变 BGRA 帧、像素格式和物理坐标采样
crates/flash-shot-image/src/image.rs        裁切、滤镜、标注合成、二维码和 PNG/JPEG/WebP 编码
crates/flash-shot-infra-windows/src/        显示器枚举、屏幕捕获、全局快捷键、托盘、剪贴板、自启动、目录、进程和窗口控制
crates/flash-shot-app/src/app.rs            GPUI 应用装配、托盘入口与生命周期
crates/flash-shot-app/src/app/              覆盖层、Pin、历史、设置和交互状态
crates/flash-shot-app/src/app/workflow/     截图、导出、滚动、识别、录屏和设置用例
crates/flash-shot-app/src/platform/         剪贴板、快捷键、托盘和窗口服务的兼容导出与剩余平台边界
crates/flash-shot-app/src/image.rs          `flash-shot-image` 的兼容导出，不承载图像实现
crates/flash-shot-app/src/dev_tools/        可选的 Release 验收、资源压力和报告库模块
crates/flash-shot-bin/src/main.rs           唯一 `flash-shot` 应用入口
scripts/run-dev-tool.ps1                    开发工具调度脚本；产物与普通应用构建目录隔离
```

当前依赖方向保持如下约束：

```text
flash-shot-bin -> flash-shot-app
                       -> flash-shot-infra-windows -> flash-shot-image -> flash-shot-domain
                       -> flash-shot-image
                       -> flash-shot-domain
```

领域库不得依赖 GPUI、HWND、COM 对象、FFmpeg 进程或具体 OCR 运行时。应用库内尚未提取的
平台适配继续由现有测试和原生验收保护，不能被误称为独立的 Windows 基础设施 crate。

## 4. 截图管线

```text
全局快捷键
  -> 创建截图会话
  -> 获取各显示器帧
  -> 上传并缓存不可变纹理
  -> 覆盖层选择与标注预览
  -> 确定性图像合成
  -> 剪贴板 / 文件 / 贴图 / OCR
```

捕获、导出和验收工具分别记录 CPU 复制、编码、资源与窗口生命周期。当前基准和真实 Windows 证据集中
记录在 [开发计划](plan.md) 与 [Windows 手工验收记录](windows-manual-acceptance.md)；单次通过不能替代
批量性能或 DPI 矩阵。单次截图上传一次纹理可以接受；交互帧不能重复编码或解码整张截图。

## 5. 标注文档

标注数据使用逻辑图像坐标和稳定 ID。渲染阶段将文档坐标转换为视口坐标；导出阶段使用原图尺寸进行合成，而不是截取应用界面。

操作通过命令模型表达。命令保存逆操作或足以完成撤销/重做的前后状态。鼠标移动可以产生临时预览，但只有正式提交的操作才能进入历史记录。

## 6. 平台边界

平台职责由下列接口概念表达，当前具体实现位于 `crates/flash-shot-app/src/platform/`；它们会在接口
边界稳定后迁移到 Windows 基础设施 crate，而不是按单个 API 拆出 crate：

- `CaptureBackend`
- `DisplayProvider`
- `GlobalShortcutService`
- `ClipboardService`
- `TrayService`
- `WindowInspector`
- `AutoStartService`
- `RecordingBackend`

接口描述产品操作和错误，不应逐个映射操作系统 API。

## 7. FFmpeg 边界

当前录屏后端启动随应用分发或由用户选择的 FFmpeg 可执行文件，并负责：

- 能力和设备探测；
- 参数构建；
- 持续消费 stdout/stderr 并解析进度；
- 优先正常结束封装，超时后才强制终止；
- Windows Job Object 或进程组清理；
- `idle/starting/recording/paused/stopping/failed` 类型化状态机。

只有测量证明进程边界成为实质瓶颈后，才考虑直接集成 libav。

## 8. 测试策略

- 几何、文档、命令、状态机、命名和配置使用纯单元测试。
- 图像合成与标注输出使用 golden image 测试。
- 坐标转换和资源释放使用平台契约测试。
- 选区和工具栏交互在可行时使用 GPUI 交互测试。
- 在自动化成熟前维护真实硬件混合 DPI 测试矩阵。
- 建立可重复的延迟、帧时间、工作集、句柄和纹理基准。
