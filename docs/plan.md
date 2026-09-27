# 主线开发计划

更新日期：2026-09-27
当前版本：`0.1.3`
目标版本：`0.2.0` 质量阶段与截图工作区 UI 改造

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

**截图 UI 与工具栏的完整复刻是当前验收目标。** 复刻以 Snow Shot 公开截图工作区为视觉和交互参考，
继续使用当前 Flash Shot 的 Rust + GPUI 代码、Capture/Save/Pin/Copy/Cancel 语义和报告协议；
gpui-kit 只在兼容性切片通过后进入组件和图标实现。设置面板精简是配套切片，不能阻塞截图主链的验收。

| 必须保留 | 允许调整 | 当前暂缓 |
| --- | --- | --- |
| 选区物理像素、标注文档坐标、快捷键、Capture/Copy/Save/Pin/Cancel 语义、导出路径、失败恢复、报告字段和清理规则 | 覆盖层 surface、主工具栏和样式栏布局、图标、分组、浮层锚定、tooltip、焦点态、双语文案层级、设置面板分组 | 新标注工具、Snow Shot 设置项逐项移植、Qt/Tauri/Web 架构移植、未具备硬件的高 DPI/多屏矩阵、真实 HTTPS 翻译和插件平台 |

## 1. 研究基线与范围

Snow Shot 参考固定为[公开 GitHub 仓库](https://github.com/mg-chao/snow-apps)的[发布版本 `v1.0.0-beta`](https://github.com/mg-chao/snow-apps/releases/tag/v1.0.0-beta)
（提交 [`395fdab`](https://github.com/mg-chao/snow-apps/commit/395fdab690d5681a46d3324ce944ffb1b84240b6)）及其工作区截图工具栏、样式行、工具组浮层、
tooltip、键盘可达性和选区边缘呈现。只借鉴可观察的视觉层级、几何关系和交互规则，不复制其设置项数量、具体颜色、图标素材或 Qt/Tauri/Web 类层次。

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
| R0 | 已确认 `flash-shot-domain`、`flash-shot-image` 的复用边界；候选 capture-core 仅保留架构评估，不创建公共 crate | `docs/architecture.md` |

这些结果不等于“截图 UI 与工具栏完整复刻”已完成。W6 证明现有实现的行为和单屏基线，W7 负责把完整视觉/交互复刻目标、矩阵和证据收敛为最终验收。

## 3. 当前状态与执行顺序

| 编号 | 主线切片 | 状态 | 退出条件 |
| --- | --- | --- | --- |
| G0 | 整合并冻结本主线计划 | 本次完成 | 只有 `docs/plan.md` 维护路线；旧完成段落不再作为待办 |
| G1 | gpui-kit 兼容性与回滚评估 | 已完成（隔离探针，2026-09-27） | `gpui-kit 0.6.6` 的组件、图标、tooltip、焦点、popover 和 420x420 Release 探针可编译；生产应用仍保留当前 GPUI 0.2.2，待 W7 后再评估全量迁移 |
| W7 | 截图 UI 与工具栏完整复刻验收 | 已完成（W7.5 视觉矩阵，2026-09-27；单屏 100%） | 视觉矩阵、真实交互、截图/JSON 报告和最终清理全部通过；高 DPI/多屏按 D1 暂缓 |
| U5 | 设置面板精简与 gpui-kit 组件落地 | 已完成（U5.4 综合设置页矩阵，2026-09-27；单屏 100%） | 重复内容收敛、所有设置键兼容、双语/双主题/三尺寸无截断或重叠；高 DPI/多屏按 D1 暂缓 |
| U1 | 动态文案国际化收尾 | 部分完成 | 用户可见动态状态全部参数化并有 English/简体中文测试 |
| U2 | App/Library/Record 入口和恢复动作收尾 | 部分完成 | 主/次/破坏性/忙/错误/恢复层级和真实键鼠矩阵通过 |
| U3 | 视觉 token、滚动和其他页面布局收尾 | 部分完成 | ThemeMetrics 覆盖稳定，真实输入和可用 DPI 矩阵通过 |
| U4 | Pin 中英文实时输入与窗口生命周期 | 部分完成 | 真实点击、焦点、关闭、再次 Capture 和语言/主题矩阵通过 |
| M1 | 按职责拆分 runner 与 overlay | 待开始 | 行为证据稳定后完成至少一个职责拆分且报告、快捷键和用户行为无回归 |
| R1 | 私有 capture-core 提取评估后的最小实现 | 待开始 | 仅在行为稳定、API/许可证/MSRV 明确后执行，不影响 `0.2.0` 主链 |
| D1 | 真实 150%/200% DPI 与多屏 | 暂缓 | 具备对应 Windows 硬件后逐项提供物理像素、窗口和清理证据 |
| P3 | 插件平台（`0.3.0+`） | 暂缓 | `0.2.0` 不开发；另行冻结插件清单、权限、资源上限和发布策略 |

执行顺序固定为：**G0 → G1 → W7.1-W7.5 → U5 → U1-U4 收尾 → M1 → R1（可选）→ D1（有硬件时）**。
P3 不插入 `0.2.0`；任何切片都必须先通过其自身验收，再进入下一切片。

## 4. 未完成切片

### G1：gpui-kit 兼容性与回滚评估（已完成）

**目标**：在当前 Rust + GPUI 应用上确认 gpui-kit 是否适合作为组件和图标层，形成可执行的采用或放弃结论。

**范围**：依赖版本与许可证审计；按钮、图标、tooltip、popover、焦点和主题 token 的最小接入；420x420、520x640、980x760 三种窗口尺寸；
English/简体中文、浅色/深色；Windows Release 编译和启动冒烟。

**不做**：不迁移 Capture/Save/Pin/Copy/Cancel 状态机，不替换截图后端，不一次性改造所有页面，不把未验证的 GPUI/Zed 跟随升级写入依赖。

**结果**（2026-09-27）：

- 新增隔离 workspace crate `flash-shot-gpui-kit-spike`，固定 `gpui-kit = 0.6.6`，不导入生产 `flash-shot-app` 状态机；
- `cargo check -p flash-shot-gpui-kit-spike`、`cargo build --release -p flash-shot-gpui-kit-spike` 和组件 headless 测试均通过；
- headless 测试验证主按钮回调、图标按钮可访问名称、tooltip 组件和 popover 打开状态；
- Release 探针启动后保持响应并创建非零 Windows 窗口句柄；当前 Computer Use 会话没有返回可操作的 Windows 原生窗口，因此没有把进程启动或 headless 结果写成 Computer Use 证据；原生窗口完整流程留给 W7 的 Release runner 验收；
- gpui-kit 0.6.6 固定使用 `gpui-pre 0.3.6`，当前 Flash Shot 仍使用 Zed GPUI `0.2.2`。两套 GPUI 类型不能直接混用，因此本切片只证明隔离组件层可用，不宣称生产应用已完成全量依赖迁移；
- 若 W7 需要生产组件迁移，必须另建依赖升级子切片，先完成 GPUI 类型迁移、窗口启动、性能和回滚验证。

**原计划退出条件复核**：

1. 最小组件样例能在当前 workspace 编译，并能在真实 Release 窗口绘制、点击、获得焦点和关闭浮层；
2. 图标资源、主题颜色、命中区和文本测量在三种尺寸及双语/双主题下没有截断或重叠；
3. 现有 workspace 测试、严格 Clippy 和 Release 冒烟通过；
4. 任何 API、运行时、许可或性能问题都有记录，并能回滚到当前 GPUI 实现。

G1 已通过隔离兼容性范围；W7 可以继续使用当前 GPUI 的 ThemeMetrics 和图标封装，或在独立依赖迁移切片通过后采用 gpui-kit 组件。不得把隔离探针写成生产应用迁移完成。

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

**分步交付**：

1. `W7.1`：冻结组件、图标、ThemeMetrics、语义状态和可访问名称映射；
2. `W7.2`：将当前 overlay handler 和语义 ID 接入完整主工具栏、样式栏及工具组浮层；
3. `W7.3`：完成布局快照、边缘避让、键盘/鼠标交互、双语双主题和三尺寸验收；
4. `W7.4`：以同一 Release 构建执行 Capture → Copy/Save/Pin/Cancel → More/工具组 → Escape 清理，并固化截图和 JSON 报告。
5. `W7.5`：以同一源码 Release 构建完成双主题、双语和三种窗口尺寸的 84 例视觉矩阵，复核代表性截图并固化矩阵报告。

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

**不做**：不新增标注能力，不改变选区像素、标注文档坐标、导出合成、快捷键、报告 schema 或失败恢复；不移植 Snow Shot 设置页和 Qt/Tauri/Web 架构。

**验收条件**：

- 确定性布局/状态测试覆盖全部动作组合，且 `cargo fmt --all -- --check`、严格 Clippy、相关 workspace 测试通过；
- Windows Release runner 生成同一 session 的截图、结构化 JSON、像素产物和清理结果；
- Copy/Save/Pin 的结果与源帧、Cancel 的恢复、真实系统剪贴板（若使用）和最终窗口/任务/输入/临时文件清零分别可核验；
- 优先使用 Computer Use 完整验证真实窗口；若当前环境无法获得可操作窗口，只能使用 Release runner 证据，并在报告中写明未覆盖的真实窗口行为；
- 所有矩阵通过后才将 W7 标记完成；静态截图或单一成功路径不能替代完整交互验收。

### U5：设置面板精简与 gpui-kit 组件落地

**目标**：删除重复、低频且难以区分的呈现，按用户任务重组设置面板，让设置面板具有与截图工作区一致的简洁、现代层级。

**范围**：App、Library、Record、Pin、外观/语言/通用设置的分组、标签、说明、开关、选择器和图标；使用 G1 已验证的 gpui-kit 组件和图标层，
否则沿用当前 GPUI 组件封装。保持现有设置键、默认值、迁移、全局快捷键、托盘和随 Windows 启动行为。

**不做**：不删除现有可用设置，不新增产品能力，不改变持久化字段或打开窗口的生命周期，不把 Snow Shot 设置项逐项复制过来。

**U5.1 当前结果（2026-09-27）**：Capture 偏好页将全局快捷键的注册状态合并到同一设置行的紧凑说明中，删除原先重复出现的 `Global shortcut` 标题和说明块，
保留快捷键启用开关、当前注册状态和快捷键选择器的设置键与行为。新增的双行设置行复用现有 ThemeMetrics 和 GPUI 控件，不引入 gpui-kit 与生产 GPUI 类型的混用。
当前源码 Release 在单屏 2560x1440、DPI 96、`scale_factor=1.0` 下完成 English/简体中文、浅色/深色、420x420/520x640/980x760 共 12 例 Capture 设置截图，
截图和同名 JSON 位于 `target/ui-acceptance/u5-settings-simplification-20260927/`，全部 `scale_match=true`，代表性窄、中、宽窗口已目视复核。

**U5.2 当前结果（2026-09-27）**：设置页宽窗侧栏改为单行任务导航，删除与内容页标题和说明重复的侧栏描述，保留稳定语义 ID、鼠标点击、Enter/Space 激活、方向键切换和窄窗顶部导航。
Capture 页在同一单屏 Release 矩阵下重新生成 12 例 PNG/JSON，所有 `scale_match=true`，宽窗侧栏和窄窗顶部导航均已目视复核；移除的描述键不再进入生产 UI。

**U5.3 当前结果（2026-09-27）**：为四个设置导航项增加统一 16px 线性图标（截图、图库、录屏、应用），图标仅承担视觉识别，既有文字标签、稳定 ID、焦点和键盘行为保持不变。
图标在宽窗侧栏和窄窗顶部导航中共享同一坐标网格与主题色；窄窗通过隐藏重复的活动竖线并缩小标签字号避免截断。当前源码 Release 重新生成 12 例 Capture 设置截图，代表性浅色/深色、中英文和窄窗样本已目视复核。

U5.1-U5.4 合并验收：当前源码 Release 已完成四页 `App/Library/Record/Capture × dark/light × en/zh-CN × 420x420/520x640/980x760` 共 48 例设置页矩阵，输出位于 `target/ui-acceptance/u5-settings-full-20260927/`；四页代表性截图已目视复核，当前未发现文本截断、控件重叠或导航图标错位。

**验收条件**：

1. 每个现有设置键都能从一个明确位置访问，重复标签、重复开关和重复说明被移除；
2. 420x420、520x640、980x760、English/简体中文、浅色/深色下无截断、重叠或滚动死区；
3. 鼠标、键盘焦点、Enter/Space、Escape 和页面返回行为可完成；
4. 旧设置文件读取、默认值、迁移和现有设置测试保持兼容；
5. Release 截图、JSON 布局记录和真实窗口清理证据来自同一版本。

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

**U4.1 当前结果（2026-09-27）**：当前源码 Release `pin-lifecycle-acceptance` 复验 English/深色与简体中文/浅色两个组合；每组在单屏 `2560x1440`、DPI 96、scale 1.0 下创建 3 个 Pin，完成缩放、透明度、内存 Copy、隔离 Save、Solo、Show all、焦点保持、关闭和 Capture preflight。English 报告为 `target/pin-lifecycle-acceptance-u4-followup-en/session-1790499114528-28552/report.json`，简体中文报告为 `target/pin-lifecycle-acceptance-u4-followup-zh/session-1790499162252-28692/report.json`；两份 schema 5 均为 `status=passed`，Copy `complete_frame_equal=true`、Save 文件存在、`show_all_preserved_focus=true`、`capture_preflight_ready=true`，每组关闭后保留 2 个窗口。该 runner 不注入真实鼠标/键盘，Computer Use 当前不可用，U4 仍需真实 Pin 输入、语言切换和再次 Capture 矩阵。

**U1.2 当前结果（2026-09-27）**：`Locale::language_changed`、`language_preference_save_failed`、`pinned_window_input_restored` 和 `ready_with_shortcut` 四个动态反馈入口统一调用 `Locale::format_template`，不再各自直接替换占位符；English/简体中文回归测试覆盖语言切换、错误详情、Pin 数量和快捷键值。`cargo test -p flash-shot-app --lib i18n::tests` 10 项通过；U1 仍需继续清点实际窗口中的动态文案并补齐真实双语交互矩阵。

**U1.3 当前结果（2026-09-27）**：`HistorySource::localized_label(Locale)` 成为图库来源的统一可见标签入口；历史搜索同时保留当前语言标签和稳定英文来源名，保存反馈与历史列表复用同一映射，不改变持久化来源值。新增 English/简体中文 Pin 来源断言，历史筛选与标签测试通过；真实窗口双语交互矩阵仍待执行。


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
验证通过后立即推送当前 `main`；本次计划整理只修改本文件，不混入代码功能提交。

## 6. `v0.2.0` 退出条件

- G1 的 gpui-kit 采用或放弃结论已记录，且没有未经验证的依赖升级；
- W7 的截图 UI 与工具栏在三种尺寸、双主题、双语、单屏 100%/DPI 96 的完整视觉和交互矩阵通过，截图/JSON/像素/清理证据齐全；
- U5 设置面板完成精简，现有设置键、默认值、迁移、快捷键和托盘行为兼容；
- U1-U4 的剩余动态文案、主动作、滚动/页面 token 和 Pin 真实窗口缺口完成；
- B1-B4 的失败恢复、标注回归和资源清理持续通过；
- M1 至少完成一个 runner 或 overlay 职责拆分，且行为无回归；
- 已具备的 DPI 环境全部执行，未具备的矩阵显式标记为 D1 暂缓；
- CI、Release 构建、便携包/安装器、manifest、SHA-256 和下载复核通过；
- README、需求、架构、计划、Windows 验收、分发和 Linux 可行性文档之间没有失效链接或相互矛盾的状态。

在 W7 和 U5 完成前，不扩展新截图工具、不推进 P3，也不把历史静态截图或单一 runner 成功路径写成完整复刻通过。
