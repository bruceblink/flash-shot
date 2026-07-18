# Windows 截图技术验证报告

验证日期：2026-07-18
范围：Windows 截图技术验证（里程碑 1），不包含标注编辑器。

## 结论

当前锁定的 GPUI 提交可用于 Windows 原生截图覆盖层：应用能在每个显示器上创建无边框弹出窗口，使用物理像素坐标完成选区，并把 BGRA 捕获帧直接交给 GPUI 渲染。该路径不需要 Tauri、WebView 或 PNG 编码/解码作为预览中间层。

截至本报告，GPUI 公共 `RenderImage` API 已足以完成一次 BGRA 上传和渲染，因此不需要小范围 GPUI 扩展或独立覆盖层渲染器。GPU 驱动内部的资源复制不由公共 API 暴露，未纳入下述 CPU 复制计数。

## 已验证的实现边界

| 能力 | 实现与证据 |
| --- | --- |
| 显示器信息 | `EnumDisplayMonitors`、`GetMonitorInfoW`、`GetDpiForMonitor` 和 `EnumDisplaySettingsW` 提供物理边界、工作区、DPI、缩放、旋转及色深。Windows 合约测试要求至少一个有效的主显示器。 |
| 每显示器截图 | 每个显示器通过 GDI `BitBlt` + `GetDIBits` 生成不可变 BGRA 帧；测试验证帧边界、尺寸、缓冲区长度和稳定排序。 |
| 虚拟桌面 | 将显示器帧按物理坐标合成为虚拟桌面帧；纯单元测试覆盖负坐标、错位显示器和透明空洞。 |
| 覆盖层坐标 | 每显示器一个不可移动、无边框的 GPUI 弹出窗口。窗口逻辑边界使用 `physical / scale_factor`，选区始终保留在物理像素空间。 |
| 跨屏选区 | 覆盖层按显示器裁剪共享选区；模拟矩阵包含负坐标、正坐标和 1.5 倍缩放。真实混合 DPI 硬件矩阵仍是发布前验证项。 |
| 预览上传 | 捕获帧直接构造成 BGRA `RenderImage`。每帧缓存一次，不在交互帧重新编码、解码或上传。 |

## 像素复制清单

对于有 `N` 个显示器的一次截图：

| 阶段 | CPU 像素复制 |
| --- | --- |
| 每显示器 `GetDIBits` 读回 | `N` |
| 合成虚拟桌面帧 | `1` |
| 工作区预览 BGRA 上传缓冲 | `1` |
| 每显示器覆盖层 BGRA 上传缓冲 | `N` |
| 合计 | `2N + 2` |

选区裁剪、PNG 保存或剪贴板导出会按用户操作再增加复制，未计入“快捷键到覆盖层”路径。性能记录会分别输出 `capture_cpu_copy_count`、`render_upload_copy_count` 与总数，避免把上传准备成本隐藏在“零复制”声明中。

## Release 冒烟基线

命令：

```powershell
cargo build --release --bin flash-shot
target\release\flash-shot.exe
```

通过真实全局快捷键 `Ctrl+Shift+Print Screen` 触发后，应用写入本地机器可读指标。该次采样环境为 Windows、NVIDIA GeForce RTX 3060、单个 2560 x 1440 显示器：

```json
{
  "latency_ms": {
    "shortcut_to_frame_ready": 90.6238,
    "shortcut_to_overlay_frame": 113.8516,
    "platform_capture": 72.0313
  },
  "frame": {
    "width": 2560,
    "height": 1440,
    "display_count": 1,
    "cpu_copy_count": 4,
    "capture_cpu_copy_count": 2,
    "render_upload_copy_count": 2
  }
}
```

这是一条 Release 冒烟样本，不是 p95 基线：当前 113.85 ms 的覆盖层首帧也高于产品 100 ms 的 p95 目标。后续应在多次热启动和真实混合 DPI 硬件上收集分布，并重点分析 GPUI 的首次窗口/纹理呈现开销。

## 连续捕获资源基线

2026-07-18 在同一台单显示器 Windows 机器上执行了 100 次 Release
虚拟桌面捕获与 PNG 编码压力测试：

```powershell
cargo build --release --bin capture-stress
target\release\capture-stress.exe --iterations 100 --output target\capture-stress-report.json
```

报告通过了资源门禁：句柄增长 `0`（上限 `8`）、线程增长 `0`（上限
`2`）、工作集增长 `14,757,888` 字节，低于 `64 MiB` 上限。端到端的
捕获与 PNG 编码延迟为 p50 `269.07 ms`、p95 `314.50 ms`。该结果验证了
连续导出不会导致资源无界增长；它包含 PNG 编码，不能替代快捷键到可交互
覆盖层的 100 ms p95 指标。

## 验证命令

```powershell
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release --bin flash-shot
```

2026-07-18 的本地结果：格式、编译、Clippy 和 56 个测试全部通过；Release 构建成功，并生成上述真实快捷键指标。
