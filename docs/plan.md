# 主线开发计划

更新日期：2026-10-05
当前版本：`0.1.3`
目标版本：`0.2.0` Snow Apps 工作区重写与产品 UI 对齐

本文档是 Flash Shot 唯一的主线开发计划。它整合此前的 B、W、U、M、R、D、P 路线，
只保留当前状态、未完成工作、执行顺序和退出条件。已经完成的切片只保留结果摘要，
逐次运行日志、旧截图和详细报告继续放在 [Windows 手工验收记录](windows-manual-acceptance.md) 或 Git 历史中，
不再另建路线图、阶段计划或同名计划。

## 文档职责边界

本文件负责版本目标、切片顺序、状态、验收条件和暂缓项；[开发设计思路](architecture.md) 负责组件职责、依赖方向、生命周期和演进规则。
其他文档只承担明确的产品或操作职责，不得重新定义路线图或架构：

| 文档 | 固定职责 | 不承担什么 |
| --- | --- | --- |
| `docs/requirements.md` | 产品范围、用户场景和非功能需求 | 不替代切片状态或实现顺序 |
| `docs/windows-manual-acceptance.md` | 真实 Windows 环境、截图、报告和清理证据 | 不把历史记录自动升级为当前通过结论 |
| `docs/windows-distribution.md` | Windows 打包、安装、manifest 和发布前复核步骤 | 不安排产品开发顺序 |
| `docs/linux-platform-validation.md` | Linux 可行性前置条件和独立验收矩阵 | 不承诺当前 Windows 主链路之外的功能对等 |

当前仓库没有其他开发计划文件。已完成、重复或互相矛盾的路线段落已从本文件合并为基线摘要；如需追溯，只查 Git 历史。

## 术语表与命名约定

| 规范名称 | English / 缩写 | 本计划中的职责边界 | 不代表什么 |
| --- | --- | --- | --- |
| 主线开发计划 | Mainline Development Plan | 唯一维护中的版本目标、切片顺序、状态和退出条件 | 不是逐次运行日志或单次验收报告 |
| 开发切片 | Development Slice | 一个可独立实现、验证、提交和推送的用户可观察结果 | 不是把多个风险合并的版本大包 |
| GPUI 组件层 | GPUI Component Layer | 当前 GPUI 应用中的主题、布局、按钮、图标和浮层实现层 | 不是新的业务状态机或截图后端 |
| gpui-kit 兼容性切片 | gpui-kit Compatibility Spike / G1 | 评估 gpui-kit 是否能安全承载现有 GPUI 组件和图标 | 不是未经验证的全量依赖升级 |
| 原生验收 | Native Acceptance | 在真实 Windows Release 会话中执行输入、窗口、像素和清理核验 | 不是单元测试或静态截图探针 |
| 截图工作区 | Screenshot Workspace | 截图覆盖层中承载选区画布、主工具栏、上下文样式栏、结果动作和工具组浮层的连续操作区域 | 不是设置面板、历史页或录屏设置 |
| 主工具栏 | Main Toolbar | 截图工作区中承载当前模式、常用结果动作和 More 入口的稳定主操作行 | 不是设置入口或所有低频功能的容器 |
| 上下文样式栏 | Contextual Style Row | 跟随当前标注工具或选中对象显示颜色、线宽、填充等相关编辑控件的附属操作行 | 不是全局设置面板或独立业务状态机 |
| 工具组浮层 | Tool Group Popover | 从主工具栏动作组展开、承载同类工具选择并返回焦点的短生命周期浮层 | 不是设置页或永久工具箱 |
| 设置面板精简 | Settings Simplification / U5 | 删除重复呈现、按任务重新分组并保持设置数据兼容 | 不是删除设置能力或修改设置键协议 |
| 拆除屏障 | Teardown Barrier | 在旧窗口、任务、输入和临时资源清理完成前阻止下一次采集的条件 | 不是单独的测试工具或持久化锁 |
| 界面度量 | ThemeMetrics | 跨页面共享的颜色语义、间距、尺寸和命中区参数 | 不是截图像素或平台 DPI 值 |

## 当前验收目标

**先完成截图工作区和工具栏的视觉、交互验收，再完成其余产品页面的整体对齐。** 以当前 Flash Shot 为产品基础，
用 Rust + GPUI 重建和收敛界面，保留已验证的 Capture/Save/Pin/Copy/Cancel 业务行为、数据和报告协议。
Snow Shot 是体验基准，不是逐项照搬清单：每项差异都明确选择“采用、适配、舍弃”并写明理由；
优先采用紧凑工具栏、清晰图标层级、上下文操作和稳定交互，合并重复设置，不移植 Qt/Tauri/Web 架构或无价值的重复控件。
本次切片已将生产 GPUI/平台依赖切换到发布版 `gpui-kit 0.6.6`/`gpui-pre-platform 0.3.6`；
完整组件覆盖和最终矩阵已由 G2/W7.11 的生产 Release 构建、84 例视觉矩阵与原生交互报告共同验收；高 DPI/多屏仍按 D1 暂缓。

| 必须保留 | 允许调整 | 当前暂缓 |
| --- | --- | --- |
| 选区物理像素、标注文档坐标、快捷键、Capture/Copy/Save/Pin/Cancel 语义、导出路径、失败恢复、报告字段和清理规则 | 覆盖层 surface、主工具栏和样式栏布局、图标、分组、浮层锚定、tooltip、焦点态、双语文案层级、设置面板分组 | 新标注工具、Snow Shot 设置项逐项移植、Qt/Tauri/Web 架构移植、未具备硬件的高 DPI/多屏矩阵、真实 HTTPS 翻译和插件平台 |

## 1. 研究基线与范围

Snow Shot 参考固定为用户提供的 `F:/project/snow-apps` 源码及其 `origin/main`。
本轮在 2026-10-05 通过 `git fetch --no-tags origin main` 核验到参考提交
[`0b28e9db8dfc2428d96781b9bab36ceec1c18318`](https://github.com/mg-chao/snow-apps/commit/0b28e9db8dfc2428d96781b9bab36ceec1c18318)。
用户本机 `main` 工作区仍位于 `2052d439`，落后 217 个提交；本轮只读取更新后的远端对象，没有切换分支或改写其源码文件。
每个开发切片开始前仍需记录实际参考提交，避免把旧 release 或落后 checkout 当成最新基准。参考范围包括工作区截图工具栏、样式行、工具组浮层、tooltip、键盘可达性和选区边缘呈现；只借鉴可观察的视觉层级、几何关系和交互规则，不复制设置项数量、具体颜色、图标素材或 Qt/Tauri/Web 类层次。

`0.2.0` 继续以 Windows 截图体验为主：全局快捷键、选区、标注、撤销/重做、复制、保存、Pin、历史、滚动截图、可选 OCR、录屏和失败恢复。
当前工作只改变 UI surface、布局、控件交互和设置面板呈现，不改变截图像素、导出协议、快捷键、报告字段和资源清理边界。

## 2. 已完成基线（不再排入后续开发）

以下切片已有当前代码或当前 Release 证据，后续只在回归失败时修复，不再重复安排同一目标：

| 基线 | 已完成结果 | 证据位置 |
| --- | --- | --- |
| `v0.1.3` 发布、B1-B4 | 发布资产、安装器、便携包、manifest、SHA-256、失败恢复、Capture/Save/Pin 生命周期、历史资源流控和标注输入回归在单屏 100%、DPI 96 通过 | `docs/windows-manual-acceptance.md` 与对应 `target/` 报告 |
| U0 | 语义颜色、ThemeMetrics、双主题和三种窗口尺寸的基础探针已建立 | Windows 验收记录 |
| W0 | Snow Shot 工作区研究和 Flash Shot 转化边界已冻结 | 本计划的研究基线 |
| W1-W5 | 工作区 surface、共享控件、主工具栏、上下文样式栏、工具组/More 浮层、选区锚定、HUD 和布局快照已完成单屏 100%、DPI 96 范围 | Windows 验收记录 |
| W6 基线 | 当前单屏 100%、DPI 96 已完成双主题/双语状态、工具组交互和 Capture → Copy/Save/Pin/Cancel → 清理主动作证据；高 DPI/多屏仍由 D1 处理 | `target/overlay-interaction-acceptance*` 与 Windows 验收记录 |
| U5 | 设置面板重复说明与导航层级已精简；48 例双语/双主题/三尺寸 Release 矩阵通过，设置键与行为兼容 | `docs/windows-manual-acceptance.md` |
| R0 | 已确认 `flash-shot-domain`、`flash-shot-image` 的复用边界；候选 capture-core 仅保留架构评估，不创建公共 crate | `docs/architecture.md` |

这些结果不等于“截图 UI 与工具栏完整复刻”已完成。W6 证明现有实现的行为和单屏基线，W7 负责把完整视觉/交互复刻目标、矩阵和证据收敛为最终验收。

## 3. 当前状态与执行顺序

| 编号 | 主线切片 | 状态 | 退出条件 |
| --- | --- | --- | --- |
| G0 | 整合并冻结本主线计划 | 本次完成 | 只有 `docs/plan.md` 维护路线；旧完成段落不再作为待办 |
| G1 | gpui-kit 兼容性与回滚评估 | 已完成（隔离探针，2026-09-27） | 隔离探针编译和组件测试通过；当时生产应用仍使用 Zed GPUI，生产迁移由 G2 单独验收 |
| W7 | 截图 UI 与工具栏完整复刻验收 | 已完成（单屏 100%、DPI 96；2026-10-05） | 最新可核验 Snow 基准下的视觉矩阵、工具栏交互、主动作、选区边界与资源清理通过；高 DPI/多屏按 D1 暂缓 |
| G2 | gpui-kit 生产接入评估与迁移 | 已完成（生产 Release 与完整组件矩阵，2026-10-05） | 生产截图工作区使用 gpui-kit 并通过 Release 验收；本次无版本冲突或回滚阻碍 |
| U5 | 设置面板精简与 gpui-kit 组件落地 | 已完成（U5.4 综合设置页矩阵，2026-09-27；单屏 100%） | 重复内容收敛、所有设置键兼容、双语/双主题/三尺寸无截断或重叠；高 DPI/多屏按 D1 暂缓 |
| U1 | 动态文案国际化收尾 | 部分完成 | 用户可见动态状态全部参数化并有 English/简体中文测试 |
| U2 | App/Library/Record 入口和恢复动作收尾 | 部分完成 | 主/次/破坏性/忙/错误/恢复层级和真实键鼠矩阵通过 |
| U3 | 视觉 token、滚动和其他页面布局收尾 | 部分完成 | ThemeMetrics 覆盖稳定，真实输入和可用 DPI 矩阵通过 |
| U4 | Pin 中英文实时输入与窗口生命周期 | 部分完成 | 真实点击、焦点、关闭、再次 Capture 和语言/主题矩阵通过 |
| M1 | 按职责拆分 runner 与 overlay | 待开始 | 行为证据稳定后完成至少一个职责拆分且报告、快捷键和用户行为无回归 |
| R1 | 私有 capture-core 提取评估后的最小实现 | 待开始 | 仅在行为稳定、API/许可证/MSRV 明确后执行，不影响 `0.2.0` 主链 |
| D1 | 真实 150%/200% DPI 与多屏 | 暂缓 | 具备对应 Windows 硬件后逐项提供物理像素、窗口和清理证据 |
| P3 | 插件平台（`0.3.0+`） | 暂缓 | `0.2.0` 不开发；另行冻结插件清单、权限、资源上限和发布策略 |

后续执行顺序为：**U1-U4 其余页面收尾 → 全产品复核与发布**。G0、G1、G2、U5 和 W7.1-W7.11 均已交付，不重复开发；M1、R1、P3 不阻塞 `0.2.0`，D1 在具备对应硬件后单独执行。

## 4. 开发切片与交付记录

### W7：截图 UI 与工具栏完整复刻验收

**目标**：完整复刻 Snow Shot 参考截图工作区的视觉层级和工具栏交互，同时保留 Flash Shot 已验证的业务行为。

**必须覆盖**：

- 选区边框、控制点、尺寸/HUD、阴影和安全边距；
- 主工具栏的当前模式、选择/移动图标、Copy、Save、Pin、Cancel、More、分组分隔和图标命中区；
- 上下文样式栏、Text/Shape/Line/Obscure 等已有工具组、工具组浮层和外部关闭；
- active、hover、pressed、focus、disabled、busy、失败/重试状态；
- tooltip、可访问名称、键盘导航、Escape/方向键和边缘避让；
- English/简体中文、浅色/深色，以及 420x420、520x640、980x760；
- 顶部、底部、左右边缘、极小选区和主工具栏/样式栏/浮层的锚定、翻转、限界与清理。

**本次验收范围**：

1. `W7.10`：基于当前 Snow 基准和用户截图复核图标、密度、主次层级、样式栏和窄窗布局；让生产依赖和验收 runner 读取同一组界面 token。
2. `W7.11`：在最终生产栈上完成截图矩阵和 Capture → 标注 → Copy/Save/Pin/Cancel → 清理原生闭环；以最终报告确认当前单屏范围。

`W7.1-W7.11` 已完成图标/动作目录、单行工具栏、上下文样式栏、工具组、主动作 Release 输入、边缘避让、紧凑密度和最终矩阵。除 D1 外不留待办；后续若改动布局、主题或本地化共用代码，需重跑对应矩阵。

**W7.1 当前结果（2026-09-27）**：现有 GPUI 工作区控件继续作为生产实现，`WorkspaceIcon` 已冻结 13 个图标的稳定语义 ID 和审查顺序，
覆盖 Move、Text、Shape、Line、Highlight、Obscure、Undo、Redo、Pin、Copy、Save、More、Cancel；确定性测试确认目录完整且没有重复 ID。
按钮仍由共享组件统一处理命中区、tooltip、可访问名称、焦点、hover、pressed、disabled 和 busy 颜色。gpui-kit 组件只在 G1 隔离探针中验证，
不与生产 GPUI 类型混用；W7.2 再将这套映射用于完整工具栏状态矩阵。

**W7.2 当前结果（2026-09-27）**：新增 `WorkspaceResultAction` 主动作目录，冻结 Pin → Save → More → Cancel → Copy 的稳定 ID、图标和顺序。
生产 overlay 的结果动作按钮和工具栏宽度计算均读取该目录，避免按钮顺序、图标和几何数量分别维护；已有 handler、快捷键、Copy busy 状态和 More 焦点身份保持不变。
相关 overlay/toolbar 测试 74 项通过，严格 Clippy 和格式检查通过。验收 runner 的真实输入矩阵仍待 W7.3/W7.4 执行。

**W7.3 当前结果（2026-09-27）**：当前源码 Release `overlay-interaction-acceptance --allow-input --capture-scenario tool-group`
在单屏 `2560x1440`、DPI 96、`scale_factor=1.0` 下通过 More 打开/外部关闭、Text/Shape 工具组、Escape、方向键 Watermark、鼠标 Rectangle 和最终清理。
报告 `target/overlay-interaction-acceptance-w7-tool-group/session-1790484880670-29100/report.json` 为 schema 30、`status=passed`，
`selected_tool=rectangle`，清理状态为 `session_state=idle`、`overlay_count=0`、`pinned_count=0`、`visible_process_windows=0`、
`capture_teardown_pending=false`、`capture_preflight_ready=true`。关键截图 `02-tool-group-toolbar.png`、`03-tool-group-text-open.png`、
`06-tool-group-shape-open.png` 和 `08-tool-group-child-clicked.png` 已目视复核；Computer Use 当前没有可操作的 Flash Shot 原生窗口，
因此本次证据标记为 Release runner 原生输入，不宣称为 Computer Use 证据。

**W7.4 当前结果（2026-09-27）**：当前源码 Release `overlay-interaction-acceptance --allow-input` 在单屏 `2560x1440`、DPI 96、
`scale_factor=1.0` 下完成 Capture、1px 键盘微调、More/Less、再次 Capture、Cancel、Save 对话框取消/重试、Pin、toolbar Copy 和 Escape 清理。
报告 `target/overlay-interaction-acceptance-w7-standard/session-1790485216176-19448/report.json` 为 schema 30、`status=passed`；
Save 与 Pin 均 `exact_match=true`，Copy 的隔离观察器结果与源帧逐像素一致，取消后选区恢复，最终 `session_state=idle`、
`overlay_count=0`、`pinned_count=0`、`capture_teardown_pending=false`、`visible_process_windows=0`、`capture_preflight_ready=true`。
本次 Copy 使用隔离观察器，没有修改系统剪贴板；真实系统剪贴板独立消费者仍由既有 Copy-only 证据覆盖。关键截图已目视复核，Computer Use 当前没有可操作的 Flash Shot 原生窗口。

**W7.5 当前结果（2026-09-27）**：当前源码 Release `run-w6-workspace-matrix.ps1 -Release -SettleMs 800` 在单屏 `2560x1440`、DPI 96、
`scale_factor=1.0` 下完成 `dark/light × en/zh-CN × 420x420/520x640/980x760 × 7 surfaces` 共 84 例。
`target/ui-acceptance/w7-workspace-matrix-20260927/matrix-report.json` 报告为 `84 passed, 0 failed`，84 份 PNG 与 84 份相邻 JSON 全部生成，
每例均记录 `dpi=96`、`scale_factor=1.0` 和物理窗口尺寸；代表性 More、工具组、中文和浅色截图已目视复核。该矩阵覆盖视觉布局，不替代 W7.3/W7.4 的真实输入与动作报告，
也不覆盖 D1 的高 DPI、负坐标双屏和混合 DPI；当前 Computer Use 会话没有可操作的 Flash Shot 原生窗口，证据来自 Release runner。

**W7.6 当前结果（2026-10-03）**：依据用户提供的 Snow Shot 截图，将默认工作区收敛为单行连续图标工具栏；11 种标注工具均可直接点击选择，
并保留 Text/Shape 同类工具浮层、撤销/重做、上下文操作、结果动作及 More 入口。新增图标遵循共享 16px 画布，普通/禁用按钮融入 toolbar surface，
工具组触发器只在重复激活当前工具时打开，现有标注与结果动作 handler 保持不变。

当前源码 Windows Release `overlay-interaction-acceptance --allow-input --capture-scenario tool-group` 在单屏 `2560x1440`、DPI 96、
`scale_factor=1.0` 下通过真实鼠标依次选择 Ellipse、Arrow、Line、Freehand、Highlight、Text、Number、Blur、Mosaic、Watermark 和 Rectangle，
并完成 More/工具组打开与关闭、键盘 Watermark 选择及 Escape 清理。报告
`target/overlay-interaction-acceptance-w7-toolbar-20261003/session-1790994289343-21792/report.json` 为 schema 32、`status=passed`；
最终 `session_state=idle`、`overlay_count=0`、`pinned_count=0`、`visible_process_windows=0`、`capture_teardown_pending=false`、
`capture_preflight_ready=true`。代表性截图 `screenshots/09-direct-annotation-toolbar.png` 已目视复核；Computer Use 未取得可操作的 Flash Shot 原生窗口，
该证据标记为 Release runner 原生输入，不宣称为 Computer Use。workspace 全量测试、严格 Clippy、格式和 all-target 检查通过。

**W7.7 当前结果（2026-10-03）**：将 More 面板的 14 个动作收敛为 `WorkspaceMoreAction` 目录，统一稳定 ID、焦点索引、
可选识别动作顺序和本地化宽度预算。主工具栏、More 面板和布局测量共同读取动作目录；保存、识别、录屏、复制颜色和重试 handler
保持原有状态机与清理语义不变。确定性动作目录、overlay 布局和验收规划测试通过；Release 工具组回归继续作为同一工作区验收证据。

**W7.8 历史结果（2026-10-03）**：以 Snow Apps 当时的 `origin/main` 提交
`168a259beca1825c2bf4ec03114758fb565fc1a5` 为参考，复核 `screenshottoolbarmainpanel.cpp`、
`screenshottoolbarlayoutmodel.h` 和选区工具栏组件。将 11 个直显标注动作的顺序、稳定 ID、图标、标签、工具组子项顺序和原生验收步骤名
集中到 `WorkspaceAnnotationToolSpec`；GPUI 渲染、工具组焦点/尺寸计算和 Release runner 均读取同一目录。验收 runner 从目录生成全部测试动作，
仅将 Rectangle 放到最后，避免重复激活 Shape 组触发器；业务 handler 和截图工作区布局未改变。

当前源码 Release `overlay-interaction-acceptance --allow-input --capture-scenario tool-group` 在单屏 `2560x1440`、DPI 96、
`scale_factor=1.0` 下通过 More、Text/Shape 工具组、键盘选择和 11 个直显标注工具点击。
报告 `target/overlay-interaction-acceptance-w7-annotation-catalog-20261003-final/session-1791007312112-5392/report.json`
为 schema 32、`status=passed`，最终 `session_state=idle`、`overlay_count=0`、`pinned_count=0`、
`visible_process_windows=0`、`capture_preflight_ready=true`。截图 `02-tool-group-toolbar.png`、
`03-tool-group-text-open.png` 和 `09-direct-annotation-toolbar.png` 已复核。Computer Use 在启动和刷新后均未返回 Flash Shot 进程或窗口；
本次交互证据来自 Release runner 原生输入，不标记为 Computer Use 证据。

**W7.9 验收要求**：真实拖动工具栏握柄后，工具栏落在指针位置并保持可见；贴近四边时完整留在安全区域；拖动开始/结束不会改变截图选区、打开工具组或误触主动作；手动位置在更换标注工具后保留，在开始新选区后回到自动锚定。通过当前源码 Release runner 或 Computer Use 的真实窗口输入验证，并回读选区、工具和工具栏边界。

**W7.9 当前结果（2026-10-04）**：截图覆盖层新增真实拖动握柄输入，纯布局辅助函数覆盖安全区夹紧和附属工作区表面同步移动；验收状态桥回读 action toolbar 的物理边界。当前源码 Release runner
`overlay-interaction-acceptance --allow-input --capture-scenario toolbar-drag`
在单屏 `2560x1440`、DPI 96、`scale_factor=1.0` 下通过选区创建、工具栏拖动、Rectangle 工具切换后位置保持、新选区自动重新锚定和 Escape 清理。
报告 `target/overlay-interaction-acceptance-w7-toolbar-drag-20261004-acceptance/session-1791084273370-7304/report.json`
为 schema 32、`status=passed`，生成 `01-toolbar-drag-selected.png`、`02-toolbar-drag-moved.png`、`03-toolbar-drag-tool-selected.png` 和
`05-toolbar-drag-new-selection.png`。Computer Use 当前没有可操作的 Flash Shot 原生窗口，因此证据标记为 Release runner 原生输入，不宣称为 Computer Use 证据。

**W7.10 / G2 当前结果（2026-10-04）**：生产 workspace 的 `gpui` 与 `gpui_platform` 已分别解析到发布版
`gpui-kit 0.6.6` 与 `gpui-pre-platform 0.3.6`，不再从 Zed git revision 获取 GPUI 实现；截图工具栏和验收 runner
统一读取 `ThemeMetrics` 的 Snow 紧凑 token：32px 控件、28px 样式行、12px 水平内边距、4px 垂直内边距、3px 主动作间距、
8px 标注工具间距、24px 图标画布和 16px 分隔线。工具栏布局、次级菜单宽高和原生输入坐标已同步更新，未改变 Capture/Copy/Save/Pin/Cancel
处理器、快捷键、报告字段或清理语义。

本次切片验证通过 `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --offline -- -D warnings`、
`cargo test -p flash-shot-app --lib --offline`（406 passed）和
`cargo build --release -p flash-shot --features dev-tools --offline`。当前源码 Release runner
`overlay-interaction-acceptance --allow-input --capture-scenario toolbar-drag` 在单屏
`2560x1440`、DPI 96、`scale_factor=1.0` 下通过选区创建、工具栏拖动、Rectangle 工具切换后位置保持、新选区重新锚定和
Escape 清理；报告 `target/overlay-interaction-acceptance-w7-toolbar-drag-20261004-gpui-kit/session-1791089624740-35200/report.json`
为 schema 32、`status=passed`，截图为 `01-toolbar-drag-selected.png`、`02-toolbar-drag-moved.png`、
`03-toolbar-drag-tool-selected.png` 和 `05-toolbar-drag-new-selection.png`。Computer Use 当前没有可操作的 Flash Shot
原生窗口，因此证据来自 Release runner，不宣称为 Computer Use；双主题/双语/三尺寸完整矩阵和 Capture → Copy/Save/Pin/Cancel
最终闭环由下方 W7.11 记录完成。

**W7.11 当前结果（2026-10-05）**：以当前可核验 Snow commit 和用户给出的 Snow Shot 对照截图完成最终单屏验收。Release 视觉矩阵覆盖 English/简体中文、浅色/深色、420x420/520x640/980x760 与 7 类工作区 surface，`case_count=84`、`passed_count=84`、`failed_count=0`；代表性 More、简体中文样式栏、右下角避让与完整工具栏截图已目视复核，所有样本为 DPI 96、scale 1.0。报告位于 `target/ui-acceptance/w7-workspace-matrix-20261004-safe-menu-final/matrix-report.json`。

最终代码门禁通过：`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --offline -- -D warnings`、`cargo check --workspace --all-targets --all-features --locked --offline`、`cargo test --workspace --all-features --locked --offline`、`cargo check -p flash-shot-app --target x86_64-pc-windows-msvc --all-targets --all-features --locked --offline`，以及 `cargo build --locked --target-dir target/dev-tools --release --features dev-tools -p flash-shot --offline`。

当前源码 Windows Release 原生输入报告覆盖 English/深色与简体中文/浅色标准流程、11 种标注工具、Text/Watermark/Line/Arrow 注释回归、工具组键鼠选择、Save 失败后重试、工具栏拖动与重新锚定、窄边缘 More/Cancel 命中，以及选区边界矩阵。边界矩阵 8/8 通过：拒绝零宽/零高，1x1、全屏、顶部/底部/左侧/右侧选区的隔离 Copy 均与提交选区逐像素一致；左侧边缘 Copy 使用测量到的实际工具栏边界定位。每例结束后 `session_state=idle`、`overlay_count=0`、`pinned_count=0`、可见进程窗口为 0、后台任务空闲、输入释放；隔离观察器保持系统剪贴板序号不变。报告与截图路径在 [Windows 手工验收记录](windows-manual-acceptance.md) 的 W7.11 行。

Computer Use 本轮仍未提供原生窗口枚举或启动接口，因此未声称完成 Computer Use；视觉与键鼠证据均来自当前源码 Release runner。验收环境为单显示器 2560x1440、DPI 96、scale 1.0；150%/200% 与多显示器继续按 D1 暂缓。

**不做**：不新增标注能力，不改变选区像素、标注文档坐标、导出合成、快捷键、报告 schema 或失败恢复；不移植 Snow Shot 设置页和 Qt/Tauri/Web 架构。

**验收条件**：

- 确定性布局/状态测试覆盖全部动作组合，且 `cargo fmt --all -- --check`、严格 Clippy、相关 workspace 测试通过；
- Windows Release runner 生成同一 session 的截图、结构化 JSON、像素产物和清理结果；
- Copy/Save/Pin 的结果与源帧、Cancel 的恢复、真实系统剪贴板（若使用）和最终窗口/任务/输入/临时文件清零分别可核验；
- 优先使用 Computer Use 完整验证真实窗口；若当前环境无法获得可操作窗口，只能使用 Release runner 证据，并在报告中写明未覆盖的真实窗口行为；
- 本次单屏矩阵、主动作、标注和边界闭环均已通过，W7 标记完成；静态截图或单一成功路径不能替代此验收范围。

### G2：gpui-kit 生产接入评估与迁移

G1 只验证了隔离探针：`gpui-kit 0.6.6` 使用 `gpui-pre 0.3.6`，不能把探针结果当作生产接入。
本次 W7.10/G2 已把生产 `gpui` 与 `gpui_platform` 切到发布版包，并在真实截图工作区拖动路径中验证；
完整组件覆盖、最终矩阵和回滚复核已由 W7.11 的 Release 构建、84 例视觉矩阵、工具栏输入与主动作报告收口；本次不扩展为全产品视觉重做或运行时升级。

**退出条件**：

- 生产应用不再混用两套不兼容的 GPUI 类型，gpui-kit 组件能由截图工作区实际使用；
- Windows Release 构建、窗口启动和 Capture → 工具栏 → Copy/Save/Pin/Cancel 流程通过；
- gpui-kit 的按钮、图标、tooltip、popover、焦点和主题在三种窗口尺寸、双语/双主题下通过；
- 具有可执行的回滚路径；版本冲突、运行时或性能问题未解决时，G2 保持未完成，不以隔离探针代替生产证据。

### U1-U4：非截图页面收尾

这些切片保留原有目标，但删除已经完成的逐次日志，只处理剩余缺口：

- `U1`：清点 workflow、错误、忙状态、数量、路径、耗时和进度等动态文案，全部进入 `UiText` 参数模板，并补齐双语参数化测试；已完成录屏状态模板、语言反馈辅助方法和历史来源标签的统一，剩余文案按相同边界继续收敛；
- `U2`：完成 App/Library/Record 主动作、恢复入口和真实键鼠矩阵，保持稳定语义 ID 和现有 handler；
- `U3`：继续收敛 ThemeMetrics、滚动控制器、页面布局和可用 DPI，清除组件内重复几何常量；
- `U4`：完成 Pin 的真实点击、焦点保持、关闭、Copy/Save、语言切换和再次 Capture 恢复；
- 每个子切片都必须单独验证、单独提交和立即推送，不能把多个页面改造合并成一个无法回滚的提交。

**U3.1 当前结果（2026-09-27）**：设置页、状态栏和历史操作控件中原本散落的状态指示器、页面标记、说明间距、快捷操作最小宽度、开关圆钮和历史选择按钮高度，现统一由 `ThemeMetrics` 提供；默认像素值保持不变，减少后续真实输入与 DPI 调整时的重复几何来源。验证包括 `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings` 和 `cargo test -p flash-shot-app --lib`（399 项通过）。U3 仍为部分完成，真实窗口键鼠与高 DPI/多屏证据尚未补齐。

**U2.1 当前结果（2026-09-27）**：新增 `settings-interaction-acceptance` Release runner，使用真实 Windows 鼠标点击四个设置入口，再用紧凑布局的右箭头或宽布局的下箭头循环导航，并验证 Enter/Space 激活；每一步通过 GPUI 命令通道回读 `SettingsSection`，保存窗口物理边界/DPI、原生窗口截图和输入清理状态。runner 需要显式 `--allow-input`，会在输入前确认修饰键/鼠标按钮已释放，输入时核对前景 HWND，并在异常时恢复鼠标与窗口层级。单屏 100% 的 520x640 English/深色与 980x760 简体中文/浅色可丢弃会话均通过 4 次鼠标点击、5 次方向导航、Enter/Space 和资源清理；证据分别为 `target/settings-interaction-acceptance-u2-final-520-en/session-23124/report.json` 和 `target/settings-interaction-acceptance-u2-final-980-zh/session-29084/report.json`，每个目录保存 10 张 PNG、DPI 96、scale 1.0。U2 仍需补齐 App/Library/Record 主动作、恢复入口及更多语言/主题/尺寸矩阵，因此保持部分完成。

**U2.2 当前结果（2026-09-27）**：在 `0462551` 之后用当前源码 Release 重跑 `settings-interaction-acceptance`，复核设置导航没有被历史来源本地化改动影响。English/深色 `520x640` 会话报告为 `target/settings-interaction-acceptance-u2-followup-520-en/session-21168/report.json`，简体中文/浅色 `980x760` 会话报告为 `target/settings-interaction-acceptance-u2-followup-980-zh/session-9756/report.json`；两份报告均为 schema 1、`status=passed`，各完成 4 次真实鼠标导航、5 次方向导航、Enter/Space，DPI 96、scale 1.0，`cursor_restored`、`input_released` 和 `window_demoted` 均为 true。Computer Use 当前会话因 `Trusted RPC service is not configured: sky` 不可用，证据来自项目 Release runner；U2 仍需真实业务主动作、恢复入口和更完整矩阵。

**U2.3 当前结果（2026-09-27）**：为 `settings-interaction-acceptance` 增加显式 `--exercise-app-update` 业务动作模式。runner 将 App 页更新检查置为生产的“检查中”状态，再使用真实 Windows 鼠标点击同一“取消更新检查”主动作，逐步回读 `update_check_in_flight`、状态栏本地化文案和页面区段，并保存点击前/后的窗口截图。English/深色 `520x640` 报告为 `target/settings-interaction-acceptance-u2-app-update-520-en-rerun/session-10344/report.json`，简体中文/浅色 `980x760` 报告为 `target/settings-interaction-acceptance-u2-app-update-980-zh-rerun/session-27256/report.json`；两份均为 schema 2、`status=passed`，忙状态到取消状态、DPI 96、scale 1.0、鼠标/键盘释放和窗口降层均通过，代表性前后截图已目视复核。该模式不访问外部更新端点；Computer Use 当前会话仍因 `Trusted RPC service is not configured: sky` 不可用，证据来自项目 Release runner。U2 仍需 Library/Record 主动作、恢复入口和更完整矩阵。

**U2.4 当前结果（2026-09-27）**：为同一 runner 增加 `--exercise-record-support` Record 恢复动作模式，并在 runner 进程内固定短录屏目录，使不同窗口宽度下的按钮位置和截图内容可复现。English/深色 `520x640` 报告为 `target/settings-interaction-acceptance-u2-record-support-520-en-rerun/session-24444/report.json`，简体中文/浅色 `980x760` 报告为 `target/settings-interaction-acceptance-u2-record-support-980-zh-rerun/session-20828/report.json`；两份均为 schema 2、`status=passed`，真实鼠标点击将 FFmpeg 支持检查从忙状态切换到取消状态，状态栏双语文案、DPI 96、scale 1.0 和输入/窗口清理均通过，点击前/后截图已目视复核。该模式不启动 FFmpeg 进程；Computer Use 当前会话仍因 `Trusted RPC service is not configured: sky` 不可用，证据来自项目 Release runner。U2 仍需 Library 主动作、Record 录制主动作、其他恢复入口和更完整矩阵。

**U2.5 当前结果（2026-09-27）**：为同一 runner 增加 `--exercise-library-format` Library 主动作模式，将测试历史根固定为进程隔离的短目录，使用真实 Windows 鼠标点击“另存为 PNG”，回读生产导出格式和状态栏文案。English/深色 `520x640` 报告为 `target/settings-interaction-acceptance-u2-library-format-520-en-cleanup-rerun/session-30056/report.json`，简体中文/浅色 `980x760` 报告为 `target/settings-interaction-acceptance-u2-library-format-980-zh-final/session-29956/report.json`；两份均为 schema 2、`status=passed`，PNG→JPEG、双语状态栏、DPI 96、scale 1.0、输入/窗口清理均通过，点击前/后截图已目视复核，runner 不生成历史截图文件。该模式不打开原生文件对话框，也不写入截图文件；Computer Use 当前会话仍因 `Trusted RPC service is not configured: sky` 不可用，证据来自项目 Release runner。U2 仍需 Record 录制主动作、其他恢复入口和更完整矩阵。

**U2.6 当前结果（2026-09-27）**：为同一 runner 增加 `--exercise-record-start` Record 主动作恢复模式，将生产“正在启动录屏”状态预置后用真实 Windows 鼠标点击“取消启动”，回读录屏启动标志和取消状态。English/深色 `520x640` 报告为 `target/settings-interaction-acceptance-u2-record-start-520-en/session-29092/report.json`，简体中文/浅色 `980x760` 报告为 `target/settings-interaction-acceptance-u2-record-start-980-zh/session-26300/report.json`；两份均为 schema 2、`status=passed`，启动忙状态到取消状态、双语状态栏、DPI 96、scale 1.0 和输入/窗口清理均通过，点击前/后截图已目视复核。该模式不启动 FFmpeg 进程；Computer Use 当前会话仍因 `Trusted RPC service is not configured: sky` 不可用，证据来自项目 Release runner。U2 仍需 Record 成功录制主动作、其他恢复入口和更完整矩阵。

**U2.7 当前结果（2026-09-27）**：为 `settings-interaction-acceptance` 增加 `--exercise-record-success`，使用隔离录屏目录通过真实 Windows 鼠标点击生产 `Record display`，等待 FFmpeg 进入活动态，再点击 `Stop recording` 并等待保存完成。English/深色 `520x640` 报告为 `target/settings-interaction-acceptance-u2-record-success-520-en-final/session-30696/report.json`，简体中文/浅色 `980x760` 报告为 `target/settings-interaction-acceptance-u2-record-success-980-zh-final/session-24616/report.json`；两份均为 schema 3、`status=passed`，均观察到活动态和停止态、生成唯一非空 MP4、DPI 96、输入/鼠标/窗口清理通过。English MP4 经 FFprobe 校验为 H.264 2560x1440、1.566992 秒、173747 bytes；简体中文 MP4 为 H.264 2560x1440、1.633984 秒、327475 bytes；活动态、停止态和保存态截图已目视复核。Computer Use 当前不可用，证据来自同一提交的 Release runner；U2 仍需其他 Record 恢复入口和更完整矩阵。

**U2.8 当前结果（2026-10-03）**：`settings-interaction-acceptance --exercise-pin-appearance` 在同一进程内打开三个生产 Pin，再通过真实鼠标输入切换 App 页主题与语言；GPUI 状态桥逐窗回读三 Pin 的实际外观。English/深色 `520x640` 会话报告 `target/settings-interaction-u4-live-en-20261003/session-20108/report.json`，简体中文/浅色 `980x760` 会话报告 `target/settings-interaction-u4-live-zh-20261003/session-13736/report.json`；两份均为 schema 4、`status=passed`、`pin_count=3`、`all_pins_updated=true`、DPI 96、scale 1.0，并保存外观切换前/后的三张窗口截图。鼠标位置、键盘状态、窗口层级与验收进程均恢复；Computer Use 未提供可操作的 Flash Shot 窗口，证据来自 Release runner 原生输入，不宣称为 Computer Use。

**U4.1 当前结果（2026-09-27）**：当前源码 Release `pin-lifecycle-acceptance` 复验 English/深色与简体中文/浅色两个组合；每组在单屏 `2560x1440`、DPI 96、scale 1.0 下创建 3 个 Pin，完成缩放、透明度、内存 Copy、隔离 Save、Solo、Show all、焦点保持、关闭和 Capture preflight。English 报告为 `target/pin-lifecycle-acceptance-u4-followup-en/session-1790499114528-28552/report.json`，简体中文报告为 `target/pin-lifecycle-acceptance-u4-followup-zh/session-1790499162252-28692/report.json`；两份 schema 5 均为 `status=passed`，Copy `complete_frame_equal=true`、Save 文件存在、`show_all_preserved_focus=true`、`capture_preflight_ready=true`，每组关闭后保留 2 个窗口。该 runner 不注入真实鼠标/键盘，Computer Use 当前不可用，U4 仍需真实 Pin 输入、语言切换和再次 Capture 矩阵。

**U4.2 当前结果（2026-09-27）**：为 `overlay-interaction-acceptance --capture-scenario pins-coexist` 增加进程隔离的彩色桌面 fixture，并统一 Pin 创建与再次 Capture 的物理屏幕坐标注入路径。当前源码 Release 在单屏 `2560x1440`、DPI 96、scale 1.0、`--settle-ms 1000` 下完成三次真实 Pin 点击、三张 360x240 源选区、指针拖动、三 Pin 共存时再次 Capture/Cancel、持久 Close 和两次 Escape 关闭；fixture 在源帧采样后关闭，报告记录 fixture 进程/窗口和 `source_fixture_cleaned_up=true`。报告为 `target/overlay-interaction-u4-pins-coexist-fixture-rerun/session-1790505296001-24964/report.json`，schema 31、`status=passed`，`pins=3`、Capture/Cancel 前后均为 3 个 Pin、`closed_with_pointer=1`、`closed_with_escape=2`、最终可见进程窗口为 0 且 Capture preflight 可用；7 张 PNG 已目视复核，包含彩色源帧和带三 Pin 的再次 Capture 画面。Computer Use 当前不可用，证据来自同一提交的 Release runner；U4 仍需运行中语言切换、主题/语言矩阵和更完整再次 Capture 矩阵。

**U4.3 当前结果（2026-09-27）**：为真实 Pin 共存 runner 增加 `--locale <en|zh-CN>` 与 `--theme <dark|light>`，报告 schema 32 记录实际组合。当前源码 Release 在同一单屏 `2560x1440`、DPI 96、scale 1.0、`--settle-ms 1000` 下分别完成 English/深色与简体中文/浅色的三次真实 Pin 点击、源帧验证、指针拖动、再次 Capture/Cancel、Close/Escape 清理；两份报告均为 `status=passed`、`pins=3`、`pins_during_capture=3`、`pins_after_cancel=3`、`source_fixture_cleaned_up=true`、最终可见进程窗口为 0 且 Capture preflight 可用。报告分别为 `target/overlay-interaction-u4-pins-coexist-en-dark/session-1790506056882-28532/report.json` 与 `target/overlay-interaction-u4-pins-coexist-zh-light/session-1790506028000-27896/report.json`；简体中文/浅色截图已目视复核，Pin 工具栏文案与颜色随组合变化。Computer Use 当前不可用，证据来自同一提交的 Release runner；U4 仍需运行中语言切换和高 DPI/多屏矩阵。

**U4.4 当前结果（2026-09-27）**：生产 Pin 窗口不再只在创建时缓存语言和主题；设置状态切换后，所有已注册 Pin 同步更新颜色、工具栏文案和主题记录，并清除旧的瞬时反馈。`pin-lifecycle-acceptance` 增加 `--switch-appearance`，在三个已打开的生产 Pin 上先记录请求组合，再切换语言和主题，回读每个窗口的实际组合；报告 schema 6 的 `live_appearance.all_pins_updated` 必须为 `true`。当前源码 Release 的 English/深色和简体中文/浅色两组均通过，报告分别为 `target/pin-lifecycle-u4-live-en/session-1790509309646-22576/report.json` 与 `target/pin-lifecycle-u4-live-zh/session-1790509330925-27088/report.json`，两组均为 `status=passed`、三窗口全部更新。该 runner 不注入真实设置页鼠标/键盘，Computer Use 当前不可用；设置页真实点击、再次 Capture 和高 DPI/多屏矩阵仍按 U4/D1 待执行。

**U1.2 当前结果（2026-09-27）**：`Locale::language_changed`、`language_preference_save_failed`、`pinned_window_input_restored` 和 `ready_with_shortcut` 四个动态反馈入口统一调用 `Locale::format_template`，不再各自直接替换占位符；English/简体中文回归测试覆盖语言切换、错误详情、Pin 数量和快捷键值。`cargo test -p flash-shot-app --lib i18n::tests` 10 项通过；U1 仍需继续清点实际窗口中的动态文案并补齐真实双语交互矩阵。

**U1.3 当前结果（2026-09-27）**：`HistorySource::localized_label(Locale)` 成为图库来源的统一可见标签入口；历史搜索同时保留当前语言标签和稳定英文来源名，保存反馈与历史列表复用同一映射，不改变持久化来源值。新增 English/简体中文 Pin 来源断言，历史筛选与标签测试通过；真实窗口双语交互矩阵仍待执行。

**U1.4 当前结果（2026-10-05）**：`Locale::format_template` 改为单次从左到右扫描模板。路径、错误详情等动态参数即使包含 `{sidecar}` 形式的文本，也只作为参数写入结果，不会被后续模板替换再次处理；English/简体中文回归测试覆盖此行为。最终内容通过 `cargo fmt --all -- --check`、`cargo check --workspace --all-targets --all-features --locked --offline`、`cargo clippy --workspace --all-targets --offline -- -D warnings`、`cargo test --workspace --all-features --locked --offline`（707 项通过）和 `git diff --check`。Computer Use 重置后仍未提供原生应用清单或应用启动接口，本切片没有真实窗口证据；完整 U1 双语交互矩阵仍待执行。

**U1.5 当前结果（2026-10-05）**：补齐更新检查、OCR 探测和翻译流程状态的英中精确断言，覆盖版本、字符数、错误详情和恢复提示。更新检查的四种终态、翻译的准备/OCR/服务失败、翻译服务空结果和探测失败均由生产状态辅助函数生成并逐字比对。`cargo test -p flash-shot-app --lib --offline app::workflow::tests::`（93 项通过）以及格式、全 workspace check、严格 Clippy、全 workspace 测试（707 项通过）和 diff 检查均通过。项目 Release runner 在真实 Windows 窗口中注入鼠标/键盘输入，完成 English/深色/520×640 和简体中文/浅色/980×760 两组设置页更新检查；两组都从检查中状态切换到取消状态，DPI 96、scale 1.0，点击和资源清理均通过。报告分别为 `target/settings-interaction-acceptance-u1-bilingual-520-en-20261005/session-26544/report.json` 与 `target/settings-interaction-acceptance-u1-bilingual-980-zh-20261005/session-25428/report.json`。Computer Use 重置后仍未提供原生应用清单或应用启动接口，以上窗口证据来自项目 Release runner 而非 Computer Use；完整 U1 双语状态矩阵仍待执行。

**U1.6 当前结果（2026-10-05）**：新增 recognition、OCR 和 translation 支持状态模板的 English/简体中文参数化精确断言，覆盖 13 个动态文案入口、错误详情、计数及本地化语言名称；目标测试 `cargo test -p flash-shot-app --lib --offline i18n::tests::recognition_and_support_templates_localize_dynamic_parameters`（1 项通过）。最终内容通过 `cargo fmt --all -- --check`、`cargo check --workspace --all-targets --all-features --locked --offline`、`cargo clippy --workspace --all-targets --offline -- -D warnings`、`cargo test --workspace --all-features --locked --offline`（708 项通过）和 `git diff --check`。

**U1.7 当前结果（2026-10-05）**：扩展录屏动态状态的 English/简体中文精确断言，覆盖 31 个模板入口，包括目录路径、来源名称、录制生命周期、计时/帧数进度、FFmpeg 能力探测和启动/停止错误；目标测试 `cargo test -p flash-shot-app --lib --offline i18n::tests::recording_status_templates_keep_targets_progress_paths_and_failures_localized`（1 项通过）。最终内容通过 `cargo fmt --all -- --check`、`cargo check --workspace --all-targets --all-features --locked --offline`、`cargo clippy --workspace --all-targets --offline -- -D warnings`、`cargo test --workspace --all-features --locked --offline`（708 项通过）和 `git diff --check`。


### M1：按职责拆分大型模块

前置条件是 W7、U1-U4 的行为证据和报告 schema 稳定。先拆 `overlay-interaction-acceptance` 的 Capture/Copy/Pin/Scroll/Recording runner，
再拆 `overlay.rs` 的渲染、输入、选择变换、菜单/工具状态和导出生命周期。每次只迁移一个职责；CLI 参数、输出目录、报告字段、快捷键和用户行为必须保持不变。

### R1：可复用截图核心的最小实现

只有在 B1-B4 和 W7 行为稳定、公共类型/API/许可证/MSRV 明确后，才在 workspace 内提取私有 capture-core 模块，覆盖请求、帧、标注合成、导出和取消边界。
不在 `0.2.0` 为第三方发布公共 crate，不把 GPUI、Windows 窗口、剪贴板、FFmpeg、历史和设置带入核心模块。

### D1：真实 DPI 与多屏

具备真实 Windows 硬件后，分别验证 150%/200% DPI、负坐标双屏和混合 DPI：自由拖选、键盘微调、标注、Copy/Save/Pin、滚动、录屏、设置窗口和清理。
报告必须记录实际 DPI/scale、物理边界、导出像素、窗口、任务、输入和临时文件。没有硬件时保持“暂缓”，不得用缩放模拟或静态图宣称通过。

### P3：插件平台（`0.3.0+`）

当前只保留方向，不进入 `0.2.0`：先冻结插件清单、能力/权限、FrameStream、取消/超时、资源上限和发布策略；录屏/GIF 先作为内置能力评估，
进程外第三方插件、市场、签名和跨平台分发另行立项。未完成安全、隔离、故障清理和发布证据前，不宣称插件生态可用。

## 5. 统一验证、提交和推送条件

每个代码切片先执行与风险匹配的测试，再从 workspace 根目录执行最终检查：

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --check
```

使用 dev-tools 或锁定依赖的切片还需执行：

```powershell
cargo check --workspace --all-targets --all-features --locked
cargo test --workspace --all-features --locked
cargo check -p flash-shot-app --target x86_64-pc-windows-msvc --all-targets --all-features --locked
```

用户可见切片必须使用同一提交构建的 Windows Release 程序保存结构化报告、关键截图、像素产物和清理结果；Computer Use 优先，
不可用时明确标记 Release runner 的证据范围。集成测试只使用本机 Docker；本机 Docker 不可用时标记未完成，不以 mock 或静态 fixture 代替。

验证失败、证据不足或只完成子集时保持“部分完成/待执行”，不得提交为完成状态。每个独立可验收功能只使用一次独立 Conventional Commit，
验证通过后立即推送当前 `main`；本次 W7.11 代码、主计划状态与 Windows 验收证据作为同一完整切片提交。

## 6. `v0.2.0` 退出条件

- G2 已完成 gpui-kit 生产接入并通过最终工作区 Release 验收；
- W7 的截图 UI 与工具栏在三种尺寸、双主题、双语、单屏 100%/DPI 96 的完整视觉和交互矩阵通过，截图/JSON/像素/清理证据齐全；
- U5 设置面板完成精简，现有设置键、默认值、迁移、快捷键和托盘行为兼容；
- U1-U4 的剩余动态文案、主动作、滚动/页面 token 和 Pin 真实窗口缺口完成；
- B1-B4 的失败恢复、标注回归和资源清理持续通过；
- 已具备的 DPI 环境全部执行，未具备的矩阵显式标记为 D1 暂缓；
- CI、Release 构建、便携包/安装器、manifest、SHA-256 和下载复核通过；
- README、需求、架构、计划、Windows 验收、分发和 Linux 可行性文档之间没有失效链接或相互矛盾的状态。

在 W7 完成前，不扩展新截图工具、不推进 P3，也不把历史静态截图或单一 runner 成功路径写成完整复刻通过。
