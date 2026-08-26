# Bug 与可见文案基线

更新日期：2026-08-26
基线提交：`f801e28`
应用版本：`0.1.2`
基线报告：`target/bug-ui-baseline/bug-ui-baseline.json`

本文档是 B0 切片的可重复输入登记，不把尚未执行的原生交互写成“已修复”。B1、B2、B4 和 U1
必须使用这里的稳定 ID、输入边界和失败恢复动作；如果实现改变了窗口、报告或状态字段，先更新本文档，
再开始新的验收。

## 1. 固定条件

除场景另有说明外，使用以下条件：

- Windows 单显示器、DPI 96、缩放 100%；
- 同一提交构建的 Release `flash-shot.exe`，通过唯一入口启动；
- 截图选区使用物理像素坐标，原点和尺寸写入结构化报告；
- 每个场景使用独立的 `target/bug-ui-baseline/<scenario-id>/` 目录；
- 失败后必须确认没有残留覆盖层、Pin、控制窗口、后台任务、FFmpeg 子进程或按键状态；
- English 和简体中文各执行一次；主题至少执行当前产品默认主题，视觉场景再覆盖另一主题。

基线报告由 `scripts/verify-bug-ui-baseline.ps1` 生成，回归验证由 `scripts/test-verify-bug-ui-baseline.ps1` 执行。
脚本只读取源文件、`Cargo.toml` 和 Git 提交，统计直接写入
`self.status` 的调用点与现有 `UiText` 引用，不启动截图、录屏、OCR 或网络服务。

## 2. 场景清单

状态含义：`已登记` 表示输入和预期已经固定；`已有证据` 表示仓库中存在可复用报告；`待执行` 表示仍需
在本次源码构建上完成验证。B0 不凭静态扫描关闭行为缺口。

| 稳定 ID | 最小输入与触发 | 预期结果 | 失败后恢复动作 | 当前状态与证据 |
| --- | --- | --- | --- | --- |
| `baseline-capture-core-single-100` | 单屏 100%；通过 `overlay-interaction-acceptance --allow-input --capture-scenario copy-only` 创建 1178x432 选区，依次 Copy、Save、Pin、Cancel。 | 每个主动作只执行一次；输出像素尺寸等于选区；取消后可再次 Capture；状态栏说明下一步。 | 等待 `capture_teardown_pending=false`，确认 overlay/Pin/控制窗口为零，再重新 Capture；失败报告保留原选区边界。 | 已登记；可复用 `current-overlay-interaction-single-100` 与 `current-save-pin-quiescence-single-100` 报告。 |
| `baseline-annotation-text-watermark` | 160x96 以上选区；分别加入空文本、单字符、`Hello, 中文`、emoji、长文本和 Watermark；快速确认、编辑、删除。 | 空文本不提交；合法文本和水印可预览、命中、撤销/重做并导出；文字内容最多 256 个字符且保持单行。 | 保留原始选区和未提交草稿；关闭编辑器后可重新打开标注，不得退出进程或留下半成品 PNG。 | 已登记；领域测试覆盖 Unicode/水印，图像测试覆盖文字和水印合成；本次源码的完整原生输入仍待执行。 |
| `baseline-annotation-two-arrows` | 在同一画布连续绘制两条箭头：`(32,24)->(144,88)` 与 `(168,96)->(64,152)`，分别反向拖动并切换 Select。 | 第二条箭头的起点、终点、箭头头部朝向和选中边界均使用画布物理坐标，不受第一条箭头影响。 | 删除或撤销第二条后第一条保持不变；关闭覆盖层并重新 Capture，不能出现错位、崩溃或残留对象。 | 已登记；已有箭头合成单元测试；双箭头真实输入与导出截图待 B4。 |
| `baseline-annotation-export-4k` | 3840x2160 合成帧，包含矩形、椭圆、箭头、自由笔画；运行 `annotation-stress --iterations 30`。 | 30 次合成均完成，输出尺寸和稳定像素指纹一致；p95 仅作为性能记录，不掩盖错误或崩溃。 | 失败时保留 JSON 和最后一次输入，不提高重试次数；先回到最小注释组合定位。 | 已有证据；复用 `target/annotation-stress*.json`，新代码变更后必须重新生成。 |
| `baseline-ocr-optional-retry` | 含文字选区；先在不提供 Tesseract 的环境运行 OCR，再恢复依赖并重试；不得发送截图原文到网络。 | 依赖缺失或识别失败时保留选区并显示 Retry；依赖恢复后可识别、复制结果并清空；截图主流程仍可用。 | 取消识别任务，确认 `recognition_in_flight=false`，再执行一次普通 Capture；不把 OCR 失败变成永久 busy。 | 已有证据；复用 `current-ocr-ui-single-100` 与 `current-ocr-auto-discovery`；翻译端点仍不在范围内。 |
| `baseline-repeated-capture-cleanup` | 连续执行 Capture -> Cancel -> Capture -> Save -> Pin -> Close，期间切换 English/简体中文和深浅主题。 | 重复触发不会叠加覆盖层或后台任务；关闭后 Capture preflight 可用；语言和主题只改变 UI，不改变像素与快捷键。 | 逐项等待窗口和任务清零；若清理失败，停止后续场景并保存窗口/任务计数。 | 已登记；依赖 `capture_teardown_pending` 和 Pin 生命周期报告；当前源码的完整连续轮次待 B2。 |
| `baseline-visible-status-inventory` | 执行 `verify-bug-ui-baseline.ps1`，扫描 `workflow.rs`、`workflow/*.rs` 和现有 `UiText` 调用点。 | 报告固定提交、版本、文件/行号、直接状态赋值和资源引用数量；每条后续迁移都有稳定来源。 | 扫描失败不生成“通过”报告；修复脚本或路径后重新运行，不手工填写数量。 | B0 本切片；报告路径见本文档顶部。 |

## 3. 可见状态迁移登记

下表只记录 B0 盘点结果。`直接状态赋值` 是后续 U1 的迁移候选，不代表每条字符串都必须删除；品牌名、
快捷键、路径和操作系统错误详情可以按 U1 规则保留为参数。

| 代码区域 | 当前来源 | 主要覆盖 | 后续切片 |
| --- | --- | --- | --- |
| `app/workflow.rs` | 直接 `self.status =` 与少量参数化字符串 | 捕获延迟、光标/颜色/导出设置、历史保留、目录和剪贴板等待 | U1 |
| `app/workflow/annotation.rs`、`support.rs` | 标注工具选择、绘制、添加、取消和文本编辑状态 | Watermark、Text、Arrow 及 Select/Undo/Redo 反馈 | B4、U1 |
| `app/workflow/recognition.rs` | OCR、二维码和翻译状态转换 | 识别中、无结果、失败、Retry、Copy text 和依赖缺失 | U1 |
| `app/workflow/recording.rs` | 录屏发现、启动、暂停、恢复、停止和错误状态 | FFmpeg 缺失、目标不可用、冲突和停止超时 | U1、B1 |
| `app/workflow/settings.rs`、`windowing.rs` | 设置保存、快捷键、启动项、更新和窗口错误 | 失败原因、取消和重新尝试动作 | U1、B1 |
| `app/view.rs`、`i18n.rs` | 已迁移的 `UiText` 与状态指示器语义匹配 | 页面标题、按钮、导航、状态点和动态数量 | U1、U2 |

运行脚本后，以报告中的 `repository_commit` 作为这次基线的事实；源码、状态键或 runner 发生变化时，旧
报告只能作为历史对照，不能替代新报告。

## 4. B0 完成条件

- 所有场景都有稳定 ID、最小输入、预期结果、失败恢复动作和证据位置；
- `verify-bug-ui-baseline.ps1` 能在干净或有无关未跟踪目标产物的工作区重复生成 JSON；对应测试脚本同时校验
  稳定字段和仓库外输出路径拒绝；
- 报告不包含截图原文、OCR 结果或外部服务响应，只包含计数、路径、提交和场景元数据；
- 静态检查、工作区测试和脚本验证通过后，B0 才能在 [Bug 修复与 UI 打磨开发计划](bug-ui-polish-plan.md)
  中标记为完成；
- B1/B2/B4/U1 使用本基线时，必须在同一提交上重新生成报告并补充真实失败或通过证据。
