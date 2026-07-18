# Linux 平台可行性验证

Flash Shot 当前是 Windows-first 应用。核心图像、标注、会话和导出代码可以复用，但系统显示器、截图、鼠标指针、剪贴板、全局快捷键、托盘、窗口检查和进程管理均只有 Windows 实现。因此 Linux 不能被描述为已支持，也不能仅凭交叉编译通过就承诺功能对等。

## 当前证据

- `x86_64-unknown-linux-gnu` Rust 目标已可安装；在当前 Windows 开发机上，`cargo check --target x86_64-unknown-linux-gnu --all-targets` 进入 GPUI 的 Linux 依赖构建后因缺少 `x86_64-linux-gnu-gcc` 停止。这是交叉 C 工具链缺失，不是 Linux 后端已经可用的证据。
- `src/platform/display.rs`、`capture.rs`、`shortcut.rs`、`clipboard.rs`、`tray.rs` 和 `window_inspector.rs` 的非 Windows 路径会明确返回 `Unsupported`。当前运行时不能在 Linux 上完成截图主链路。
- 锁定的 GPUI 依赖解析出了 Linux 的 Wayland、X11、AT-SPI、D-Bus 和 portal 相关依赖；这只证明 UI 平台具备候选基础，不能替代 Flash Shot 自己的系统服务实现。

## 验证前置条件

必须在原生 64 位 Linux 桌面会话中执行，不使用 Windows 交叉编译结果代替：

```bash
./scripts/check-linux-feasibility.sh
```

X11 会话需要可访问的 `DISPLAY` 和 `xdpyinfo`。Wayland 会话需要 `WAYLAND_DISPLAY`、用户 D-Bus、`busctl`，并且 `org.freedesktop.portal.Desktop` 必须公开 `ScreenCast` 与 `GlobalShortcuts` 接口。脚本完成会话和服务前置检查后执行宿主 `cargo check --workspace --all-targets`。

运行脚本的系统还需要 Rust 1.92+、C 编译器、`pkg-config` 和 GPUI 所需的图形/字体开发库。以发行版包名为准安装相关依赖；不要在项目中固化只适用于单一发行版的包名。

## 验收矩阵

| 能力 | X11 验收 | Wayland 验收 | 结论门槛 |
| --- | --- | --- | --- |
| 显示器与坐标 | 多显示器、负坐标、不同缩放下得到稳定物理像素边界 | 多显示器、分数缩放和输出变更后重新选择源 | 坐标模型必须保持 `PhysicalRect` 不变式 |
| 截图 | 截取整个桌面、窗口和自由区域，验证 PNG 像素 | 通过 xdg-desktop-portal `ScreenCast` 选择源并从 PipeWire 获得帧 | 用户批准、取消、拒绝和源断开均可恢复 |
| 选择覆盖层 | 覆盖层不截入导出结果，拖动/键盘微调正确 | 不假设能绕过合成器；验证 portal 选择流程与应用内编辑边界 | 不允许使用未获授权的屏幕或输入访问 |
| 剪贴板 | PNG 与文本可粘贴到另一应用 | PNG 与文本可在同一桌面会话粘贴 | 所有权丢失和目标退出不导致崩溃 |
| 全局快捷键 | 在常见桌面环境注册、冲突和退出回收 | 通过 xdg-desktop-portal `GlobalShortcuts` 完成注册与用户授权 | 不将 X11 抢键盘方式移植到 Wayland |
| 托盘和窗口检查 | 至少验证一个支持 StatusNotifier 的桌面环境 | 验证目标桌面是否提供托盘；窗口/控件检查不得承诺 Windows UI Automation 对等 | 不支持时功能必须降级且可见 |

## 设计决策

Linux 后端按两个独立阶段处理：

1. 先完成原生 X11 的显示器、截图、剪贴板、快捷键和基本选择流程，并通过矩阵。
2. 单独实现 Wayland portal/ PipeWire 流程；`ScreenCast`、`GlobalShortcuts` 与用户授权生命周期是产品边界，不能以 X11 API 兼容层替代。

每个阶段都需要真实桌面环境的截图、复制、取消、重连和多显示器手工验收，以及对应的平台契约测试。完成这些证据前，发行资产和产品文档继续只承诺 Windows。
