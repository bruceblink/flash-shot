# 主线开发计划

更新日期：2026-09-24
当前版本：`0.1.3`
目标版本：`0.2.0` 质量阶段与截图工作区 UI 改造

本文档是 Flash Shot 唯一的主线开发计划。它记录当前代码事实、后续切片、验收条件和明确暂缓项；
逐次运行日志、旧计划和机器专属输出留在 Git 历史或 Windows 验收记录中，不再另建执行计划。

## 文档职责边界

当前工作树只保留一份路线图和一份开发设计来源：本文件负责版本目标、切片顺序、状态和退出条件，
[开发设计思路](architecture.md) 负责组件职责、依赖方向、生命周期和演进规则。其他文档只承担明确的产品或操作职责，
不得重新定义路线图或架构：

| 文档 | 固定职责 | 不承担什么 |
| --- | --- | --- |
| `docs/requirements.md` | 产品范围、用户场景和非功能需求 | 不替代切片状态或实现方案 |
| `docs/windows-manual-acceptance.md` | 真实 Windows 环境、截图、报告和清理证据 | 不把历史记录自动升级为当前通过结论 |
| `docs/windows-distribution.md` | Windows 打包、安装、manifest 和发布前复核步骤 | 不安排产品开发顺序 |
| `docs/linux-platform-validation.md` | Linux 可行性前置条件和独立验收矩阵 | 不承诺当前 Windows 主链路之外的功能对等 |

历史路线图、旧 UI 基线、重复工作流和重复 crate 设计文档已从工作树移除；如需追溯，只查 Git 历史，
不得重新创建同名或平行计划。

## 术语表与命名约定

| 规范名称 | English / 缩写 | 本计划中的职责边界 | 不代表什么 |
| --- | --- | --- | --- |
| 主线开发计划 | Mainline Development Plan | 唯一维护中的版本目标、切片顺序、状态和退出条件 | 不是逐次运行日志或单次验收报告 |
| 开发切片 | Development Slice | 一个可独立实现、验证、提交和推送的用户可观察结果 | 不是把多个风险合并的版本大包 |
| 原生验收 | Native Acceptance | 在真实 Windows Release 会话中执行输入、窗口、像素和清理核验 | 不是单元测试或静态截图探针 |
| 操作代次 | Operation Generation | 判断异步结果是否仍属于当前截图或历史操作的递增标识 | 不是应用版本或历史序号 |
| 拆除屏障 | Teardown Barrier | 在旧窗口、任务、输入和临时资源清理完成前阻止下一次采集的条件 | 不是单独的测试工具或持久化锁 |
| 资源所有者 | Resource Owner | 当前负责提交或释放窗口、文件、剪贴板写入和外部进程资源的操作 | 不是业务数据的持久化拥有者 |
| 界面度量 | ThemeMetrics | 跨页面共享的颜色语义、间距、尺寸和命中区参数 | 不是截图像素或平台 DPI 值 |
| 截图工作区 | Screenshot Workspace | 截图覆盖层中承载选区画布、主工具栏、上下文样式栏、结果动作和工具组浮层的连续操作区域 | 不是设置面板、历史页、录屏设置或完整应用导航 |
| 主工具栏 | Main Toolbar | 截图工作区中承载当前模式、常用结果动作和 More 入口的稳定主操作行 | 不是设置入口或所有低频功能的容器 |
| 上下文样式栏 | Contextual Style Row | 跟随当前标注工具或选中对象显示颜色、线宽、填充等相关编辑控件的附属操作行 | 不是全局设置面板或独立的标注业务状态机 |
| 工具组 | Tool Group | 将已有同类标注或操作入口作为一个主工具栏动作组展示，并由工具组浮层承载选择 | 不是新增领域工具或全局设置分类 |
| 工具组浮层 | Tool Group Popover | 从主工具栏动作组展开、承载同类工具选择并返回焦点的短生命周期浮层 | 不是设置页、永久工具箱或跨窗口菜单 |
| 选区提示 | Selection HUD | 在选区附近显示尺寸、目标或放大预览等即时反馈的工作区元素 | 不是导出图像内容或持久化状态 |
| 工作区布局快照 | Workspace Layout Snapshot | 一次布局计算中同时记录主工具栏、样式栏、More/工具组浮层和安全边距几何的不可变结果 | 不是截图帧、导出结果或持久化设置 |
| 暂缓项 | Deferred Scope | 已明确不纳入当前版本、等待硬件/服务/产品决策的范围 | 不是已完成或默认支持 |

## 1. 代码复审结论

本次复审基于 `main` 提交 `4fa4832`（2026-09-06）以及 workspace 中的五个 crate。复审范围包括 Cargo
依赖方向、应用生命周期、截图/标注/导出/历史/录屏 workflow、UI 状态和开发工具入口。

当前静态与自动化结果：

- `cargo fmt --all -- --check` 通过；
- `cargo clippy --workspace --all-targets -- -D warnings` 通过；依赖仍有 Rust future-incompatibility 提示，
  但没有当前 warning；
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` 通过；历史资源验收
  helper 已使用共享参数上下文，未改变验收行为；
- `cargo check --workspace --all-targets --all-features --locked` 与
  `cargo test --workspace --all-features --locked` 均通过；
- `cargo test --workspace` 通过；Copy 取消竞争场景解析、隔离 sink 检查、快速保存失败后重试清理、首选目录失效回退和录屏 worker 启动失败释放测试也在全特性测试中通过；
- `cargo check -p flash-shot-app --target x86_64-pc-windows-msvc --all-targets --all-features --locked` 通过，
  仅证明 Windows 原生分支可编译，不替代真实窗口、输入和像素验收；
- 当前 workspace 只有 `flash-shot-domain`、`flash-shot-image`、`flash-shot-infra-windows`、
  `flash-shot-app` 和 `flash-shot` 五个成员，只有一个 `flash-shot` 二进制目标；
- `v0.1.3` 已发布为 Latest Release，发布资产、安装器、便携包、manifest、SHA-256 和发布说明均已从公开
  Release 下载复核；已有 Windows 单显示器 100% 主流程、历史流控、Pin 生命周期和设置/覆盖层视觉探针证据。

复审没有确认新的静态 P0，但发现以下未完成项。它们决定后续顺序，不能用单元测试或旧报告代替：

| 优先级 | 未完成项 | 当前事实 | 完成所需证据 |
| --- | --- | --- | --- |
| P1 | 动态文案盘点 | `Locale`/`UiText` 已覆盖大部分设置、Capture、Library、Record、Pin 和 workflow；剩余动态状态需要重新盘点，不能沿用旧的 317 条计数 | 中英文资源覆盖、参数化模板测试、无未登记用户可见硬编码 |
| P1 | UI 信息层级 | 设置壳层、Pin token 和 Library/Record 尺寸已有部分复核；截图工作区仍需按 W1-W6 收敛主行、样式行、工具组浮层和错误恢复入口 | 420x420、520x640、980x760；中英文、深浅主题；真实入口可达且无重叠 |
| P1 | 大模块维护成本 | `overlay.rs`、`view.rs`、`i18n.rs` 和 `overlay-interaction-acceptance.rs` 仍集中在 `flash-shot-app` | 先冻结行为，再按职责小步拆分；报告 schema、快捷键和用户行为不变 |
| P2 | 环境矩阵 | 当前证据主要是单显示器 100%；150%/200%、负坐标双屏和混合 DPI 双屏没有当前硬件证据 | 对应真实 Windows 环境、物理像素、窗口布局和清理报告 |
| P2 | 在线翻译与跨平台 | 真实 HTTPS 翻译闭环以及 Linux/macOS 功能对等尚未排期 | 产品范围恢复、可丢弃服务或原生桌面环境，以及独立验收计划 |

## 2. 范围与原则

### 2.1 当前主线

`0.2.0` 只推进稳定、清晰、可维护的 Windows 截图体验：全局快捷键、选区、标注、撤销/重做、复制、
保存、Pin、历史、滚动截图、可选 OCR、录屏及失败恢复。UI 统一 English 与简体中文的动作层级、
状态反馈、视觉 token 和可达性。

### 2.2 不在当前版本

- 新的标注工具、OCR 引擎、翻译供应商或录屏后端；
- 真实 HTTPS 翻译成功/失败/重试闭环；
- 负坐标双屏、混合 DPI 双屏和未具备硬件的 150%/200% 矩阵；
- Linux/macOS 功能对等、协作/云同步、浏览器或移动端；
- `0.3.0+` 插件扩展平台：插件宿主、版本化接口约定、进程外插件以及可选录屏/GIF 能力；当前版本只冻结设计方向，
  不提前承诺第三方插件兼容或独立安装包；
- 未经性能和原生验收的 GPUI/Zed 跟随升级。

### 2.3 执行原则

1. 一个切片只解决一个可观察结果，先写边界和失败行为，再修改代码。
2. 行为、视觉、结构重构、依赖升级和文档各自独立提交；不把未完成的证据写成通过。
3. 静态测试、确定性测试、Release 原生输入和像素/清理报告分别说明，不能互相替代。
4. 每个异步操作都绑定操作代次或等价的资源所有者检查；失败必须释放 busy、窗口、输入、任务、进程和临时文件。
5. 继续保持唯一 `flash-shot` 二进制、版本化设置和现有报告/脚本参数兼容。

截图工作区 UI 另按 W0-W6 管理：只调整截图覆盖层的层级、外观、布局和控件交互，不改变选区物理像素、标注文档坐标、
Copy/Save/Pin/Cancel 语义、快捷键、报告字段或失败恢复规则。Snow Shot 的设置面板、设置项组织、工具栏定制设置、Qt/Tauri/Web
架构和未被工作区直接验证的功能不进入本路线；它只作为公开工作区外观和交互的参考来源。

### 2.4 Snow Shot 最新版研究基线

本路线以[公开 GitHub 仓库](https://github.com/mg-chao/snow-apps)为准，不读取或推断本机旧版 checkout。截至 2026-09-13，
[Snow Shot 最新公开 Release `v1.0.0-beta`](https://github.com/mg-chao/snow-apps/releases/tag/v1.0.0-beta)
对应提交为 [`395fdab`](https://github.com/mg-chao/snow-apps/commit/395fdab690d5681a46d3324ce944ffb1b84240b6)。Release 说明和该 tag
用于确认已发布的工作区能力；`main` 分支的测试文档只作为补充研究材料，不把未发布代码写成 Flash Shot 已验证能力。

| 公开参考 | 观察到的工作区做法 | Flash Shot 转化决定 |
| --- | --- | --- |
| [Release 说明](https://github.com/mg-chao/snow-apps/releases/tag/v1.0.0-beta) | 自定义截图工具栏、可隐藏/重定位的次级工具栏、滚动模式、Save As 实时预览、分数 DPI 边框修正、分组动作 tooltip、箭头附着文字和拖拽期间保留选区的动作图标 | 采用稳定主工具栏加上下文样式栏的层级；常用动作图标优先、说明通过 tooltip 和可访问名称补足；不复制 Snow Shot 的设置项数量 |
| [`ScreenshotToolPalette` 声明](https://github.com/mg-chao/snow-apps/blob/v1.0.0-beta/snow_shot/include/snow_shot/presentation/screenshottoolpalette.h) 与[实现](https://github.com/mg-chao/snow-apps/blob/v1.0.0-beta/snow_shot/src/presentation/tools/screenshottoolpalette.cpp) | 浮动半透明调色板由主工具栏、可选次级/样式行和录制控件组成；工具组按需生成，布局提交会同步激活并按 size hint 调整几何 | 在 `overlay.rs` 保留现有语义 ID 和 handler，抽出工作区共享 surface、按钮、分隔线、swatch、工具组浮层和一次性布局快照；不迁移 Qt 类层次 |
| [主面板测试](https://github.com/mg-chao/snow-apps/blob/main/snow_shot/tests/screenshot_toolbar_main_panel_tests.cpp) | 工具栏按内容和 DPI 计算尺寸，拖动、换行、样式组布局请求和按钮图标不会造成重复几何失效；覆盖 50/100/125/150/200% 的阴影与尺寸一致性 | 用纯布局测试覆盖可见动作组合、三种窗口尺寸和可用 DPI；移除截图工作区对单一总宽度与每个固定按钮宽度的依赖 |
| [共享控件测试](https://github.com/mg-chao/snow-apps/blob/main/snow_shot/tests/toolbar_shared_controls_tests.md) 与[浮层调查](https://github.com/mg-chao/snow-apps/blob/main/snow_shot/tests/toolbar_popover_investigation.md) | 截图、Pin、录制共享控件状态、tooltip、可访问性、阴影和分隔线；工具组支持悬停发现、点击确定、局部坐标命中、Escape/外部关闭和单一浮层所有者 | 统一 GPUI 工作区控件状态与浮层生命周期；More 保持点击/键盘可达，不依赖悬停才能操作；浮层位置只在最终放置阶段使用全局坐标 |
| [`ScreenshotSelectionShadowRenderer`](https://github.com/mg-chao/snow-apps/blob/v1.0.0-beta/snow_shot/src/presentation/overlay/screenshotselectionshadowrenderer.cpp) | 选区周边的 checkerboard、圆角阴影和缓存九宫格只用于预览/结果呈现，导出合成器独立 | 只借鉴选区边缘的层次、阴影和安全间距；所有效果停留在 overlay paint 路径，禁止进入导出像素 |

工作区外观基线固定为：深色或浅色语义 surface 上的轻量浮动工具栏、细边框、柔和阴影、4px 间距基线、稳定命中区、单一强调色，
以及明确的 active/hover/pressed/focus/disabled/busy 状态。Flash Shot 保留现有 teal 语义色和浅色/深色主题，不复制 Snow Shot 的具体颜色、
图标素材或设置面板布局；截图工作区要让选区、主动作和当前上下文成为第一视觉层级。

## 3. 当前状态

| 编号 | 主线切片 | 状态 | 说明 |
| --- | --- | --- | --- |
| A | `v0.1.3` 发布基线 | 已完成（2026-09-06） | 标签 `v0.1.3` 的 CI 源码/Release 构建、安装器生命周期、便携启动、manifest、SHA-256、双语发布说明和公开 Release 下载复核均通过 |
| B1 | 外部失败恢复 | 已完成（单屏 100%，DPI 96） | 保存、历史和录屏进程的确定性恢复，以及 `copy-cancellation-race`、真实剪贴板争用、Quick Save/原生 Save 对话框目录权限和 FFmpeg 启动失败重试均已有证据；150%/200%、多屏仍按 D1 单独验收 |
| B2 | Capture/Save/Pin/Close 生命周期 | 已完成（单屏 100%） | 操作代次、拆除屏障、输入释放和下一次 Capture 已有 Release 证据 |
| B3 | 历史异步流控 | 已完成（单屏 100%） | 300 条队列、失败/重试、删除、目录切换和窗口关闭已有资源证据 |
| B4 | 标注回归保护 | 已完成（单屏 100%，DPI 96） | 当前 Release 已通过真实 Text、Watermark、Line、正向/反向 Arrow、Quick Save、物理坐标、像素变化和清理验收；150%/200%、多屏仍按 D1 单独验收 |
| U0 | 视觉设计基线 | 已完成（单屏 100%） | 语义颜色、几何 token、双主题和三种窗口尺寸已有设置/覆盖层探针 |
| W0 | 最新版 Snow Shot 截图工作区研究与路线冻结 | 已完成（文档研究，2026-09-13） | 已锁定公开 `v1.0.0-beta` 的工作区参考、Flash Shot 转化边界和 W1-W6 验收顺序；不纳入 Snow Shot 设置面板 |
| W1 | 工作区视觉基础与共享控件 | 已完成（2026-09-13；单屏 100%） | 已统一截图工作区 surface、按钮、分隔线、swatch、tooltip、焦点态和布局 token；确定性布局测试与当前源码 Release 视觉矩阵通过 |
| W2 | 主工具栏重排 | 已完成（2026-09-13；静态 UI，单屏 100%） | 常用 Copy/Save/Pin/Cancel 和 More 已形成稳定主行；低频动作移入按需面板，保留语义 ID、快捷键和异步状态 |
| W3 | 上下文样式栏 | 已完成（2026-09-13；静态 UI，单屏 100%） | 样式控件已按当前工具/对象能力收敛为工作区附属行；中英文、双主题和 360/520/700/900/1100 宽度 Release 视觉矩阵通过 |
| W4 | 工具组与 More 浮层 | 已完成（2026-09-13；静态双语/双主题，单屏 100% 真实输入） | 已统一低频动作、工具组选择、键盘焦点、关闭规则和局部坐标放置；More 保持独立的按需浮层 |
| W5 | 选区锚定、HUD 与布局快照 | 已完成（2026-09-25；单屏 100%，DPI 96） | 已统一工作区布局快照；零面积拒绝、1px、全屏和四边贴边的真实 Release 矩阵通过，包含导出尺寸/边界、隔离 Copy sink、状态复位和窗口/任务/输入清理；150%/200% DPI 与多屏按 D1 暂缓 |
| W6 | 双主题、双语、DPI 与真实输入验收 | 进行中（2026-09-26；Snow Shot 工具栏视觉对齐完成） | 选区默认显示完整横向编辑工具栏，选择/移动图标、分组分隔、蓝色 Copy、红色 Cancel 和小窗口避让已按参考图复核；主动作像素导出行为、真实系统剪贴板与完整多动作主链仍待执行，高 DPI 与多屏按 D1 暂缓 |
| U1 | 动态文案国际化 | 部分完成 | 继续清点 workflow、错误、忙状态和动态数量；Record 无输入验收状态已改为读取活动语言资源，其他动态文案仍需盘点 |
| U2 | 信息架构与动作收敛 | 部分完成（Record 主动作，2026-09-09） | Record 页已将录制/重试作为唯一强调动作，支持检查保持次要层级；App、Library 和真实输入矩阵仍待继续 |
| U3 | 视觉 token 与布局 | 部分完成 | 设置、覆盖层、Pin、Library/Record 已使用部分 token；真实输入和高 DPI 未覆盖 |
| U4 | Pin 中英文实时输入 | 部分完成 | 四组合无输入 runner 已通过；真实鼠标/键盘与打开窗口切换待执行 |
| M1 | 模块职责拆分 | 待开始 | B1-B4、U1-U4 行为证据稳定后，先拆 runner，再拆 overlay |
| R0 | 可复用截图核心 crate 评估 | 已完成（文档决策，2026-09-07） | 已确认 `flash-shot-domain` 与 `flash-shot-image` 是可复用基础；候选 `flash-shot-capture-core` 的边界、API、依赖和许可证门槛已写入[开发设计思路](architecture.md)，当前不宣称已有稳定公共 crate |
| D1 | 单显示器高 DPI | 暂缓 | 需要真实 150%/200% Windows 硬件；未执行环境不得推断通过 |
| P3 | 插件扩展平台 | 规划中（`0.3.0+`） | 先冻结插件宿主、清单、接口约定和有界帧流；录屏/GIF 先作为内置插件适配，进程外第三方插件待后续安全与发布决策 |

## 4. 后续主线切片

### B1：关闭外部失败恢复缺口

**目标**：外部资源失败后立即可诊断、可重试，并恢复到可再次 Capture/Record 的状态。

**边界**：剪贴板短暂占用、消费者超时、所属 HWND 创建失败；快速保存目录只读、同步失败、文件名冲突；
FFmpeg 缺失、启动失败、运行中退出和停止超时。只修改相关 workflow、Windows 基础设施、录屏状态和测试/runner。

**验收**：每个失败都由确定性测试或隔离 Windows Release 探针触发；状态说明下一步；再次操作成功；
`capture_teardown_pending=false`，覆盖层/Pin/控制窗口、后台任务、FFmpeg 子进程、按键和 `.tmp` 均清零。
剪贴板场景必须记录消费者 ready/observing/cleanup 和 PNG/CF_DIB 像素结果，不能只检查应用内状态。

**当前切片**：选择复制的后台 worker 在 `ClipboardCommitGate` 检查点等待，runner 使用真实 Copy 输入后立即注入
Escape，再释放检查点。`copy-cancellation-race` 只使用进程内 `isolated_observer`，不会修改系统剪贴板；schema 20
报告记录检查点是否在 Escape 前到达、Escape 是否在提交前获胜、选择复制和剪贴板写入是否释放、取消状态和最终清理状态。
确定性测试、参数解析、Windows 目标编译和真实 Release 会话均已通过。当前单屏 100%、DPI 96 的两份证据为：

- 工具栏 Copy：`target/overlay-copy-cancellation-race-toolbar/session-1788186329822-11284/report.json`；
- Enter Copy：`target/overlay-copy-cancellation-race-enter/session-1788186375319-19480/report.json`。

两份报告均为 schema 20、`status=passed`，检查点在 Escape 前到达、观察器未收到帧、两类操作状态均释放，最终
overlay/Pin/任务/按键清零；截图与路径已登记在 [Windows 手工验收记录](windows-manual-acceptance.md)。在当时记录点 B1 尚保持
“部分完成”。快速保存确定性 fixture 已在提交 `a4e5081` 覆盖损坏帧失败、最终保留名和 `.tmp` 清理、使用同一
时间戳/UUID 的再次保存、首选目录失效后的全屏 PNG 回退；提交 `a6f053e` 又覆盖录屏 worker 启动失败后的运行标记
释放；提交 `a62461d` 覆盖持续系统剪贴板争用在固定预算后返回最后错误。当前源码 Release 的
`settings-ui-acceptance` 已以无输入探针生成并目视复核 `failed`/`cancelled` Record 状态，报告和截图已登记在
Windows 验收记录；截至该记录点，真实系统剪贴板争用和 Quick Save 只读目录已补齐，下一步只处理原生 Save 对话框目录权限和 FFmpeg 用户界面失败恢复。

2026-09-10 的当前源码 Release `overlay-interaction-acceptance --capture-scenario save-failure-retry`
补齐了 Quick Save 目标失效后的真实输入回归：runner 在同一选区提交 `Shift+Enter` 前临时替换隔离历史目录，
生产 Quick Save 收到真实文件系统错误后保留选区和覆盖层；恢复原目录后再次 Quick Save 成功。schema 22
报告为 `status=passed`，1178x432 PNG 与源帧逐像素一致，失败和成功路径的 `.tmp` 均为 0，最终
`capture_teardown_pending=false`、覆盖层/后台任务/可见 runner 窗口均清零。证据位于
`target/overlay-save-failure-retry-current-v3/session-1789053581823-18576/report.json` 及其同目录截图。
该证据只覆盖单屏 100% 的 Quick Save 目标失效与恢复；截至该记录点，B1 仍保持“部分完成”。

2026-09-11 的当前源码 Release `overlay-interaction-acceptance --capture-scenario clipboard-contention-retry
--allow-system-clipboard` 又补齐了真实系统剪贴板争用后的 Copy 重试：runner 启动无窗口子进程持有剪贴板锁，
不读取、不清空也不写入原有内容；第一次真实 toolbar Copy 和 Enter Copy 均在有界等待后返回
`拒绝访问 (os error 5)`，选区和覆盖层保留且剪贴板序号不变。释放占用后再次触发生产 Copy，独立消费者读取
PNG、CF_DIB 和普通图像结果，三者与源帧逐像素一致；schema 23 两份报告均为 `status=passed`，最终
`capture_teardown_pending=false`、覆盖层/Pin/后台任务/可见 runner 窗口和输入均清零。工具栏报告位于
`target/overlay-clipboard-contention-retry-current-toolbar-v3/session-1789132623560-21696/report.json`，
Enter 报告位于 `target/overlay-clipboard-contention-retry-current-enter-v2/session-1789132728113-22384/report.json`，
截图和清理/消费者字段均来自对应 session。该证据只覆盖单屏 100% 的系统剪贴板锁争用恢复；截至该记录点，B1 仍保持“部分完成”。

2026-09-11 的当前源码 Release `overlay-interaction-acceptance --capture-scenario save-permission-retry`
补齐了 Quick Save 真实只读目录的失败恢复：runner 在隔离 profile 的 `history` 目录上为当前 Windows 用户应用
真实 `icacls` deny-write ACL，并用文件创建 probe 确认写入确实被拒绝；生产 Quick Save 返回包含
`拒绝访问 (os error 5)` 的可重试状态，保留同一选区和覆盖层。移除 ACL 并再次确认目录可写后，第二次
`Shift+Enter` 在同一选区成功。schema 24 报告为 `status=passed`，证据位于
`target/overlay-save-permission-retry-current-v3/session-1789136577503-20804/report.json`，同 session 的
`screenshots/01-save-permission-selection.png`、`03-save-permission-reported.png` 和
`05-save-permission-retry-clean.png` 已目视复核；报告记录权限 probe、历史索引未改写、失败/成功 `.tmp` 均为 0，
重试 PNG 为 1178x432 且 `exact_match=true`，最终 `capture_teardown_pending=false`、覆盖层/Pin/后台任务/可见
runner 窗口均清零。该证据只覆盖 Quick Save 目录权限；截至该记录点，B1 仍保持“部分完成”。

2026-09-11 的当前源码 Release `overlay-interaction-acceptance --capture-scenario save-dialog-permission-retry`
补齐了原生 Save 对话框目录权限失败后的恢复：runner 在隔离 `exports/read-only` 目录应用当前 Windows 用户的
真实 `icacls` deny-write ACL，点击生产 Capture 工具栏的 Save，向原生 `另存为` 对话框输入被拒绝路径，并等待
真实 `#32770` 权限提示。提示关闭后 Save 对话框仍保留，选区没有丢失；恢复 ACL 后再次打开 Save 对话框并保存到
`exports/retry`。schema 25 报告为 `status=passed`，证据位于
`target/overlay-save-dialog-permission-retry-current-v3/session-1789139877235-8004/report.json`；同 session 的
`screenshots/01-save-dialog-permission-selection.png`、`03-save-dialog-permission-error.png`、
`04-save-dialog-permission-selection-restored.png`、`05-save-dialog-permission-retry-path.png` 和
`06-save-dialog-permission-retry-clean.png` 已目视复核。报告记录权限 probe、拒绝目标保持不存在、Save 对话框保留、
选区恢复、失败/重试 `.tmp` 均为 0；重试 PNG 为 1178x432、259556 bytes 且 `exact_match=true`，最终
`capture_teardown_pending=false`、覆盖层/Pin/后台任务/可见 runner 窗口均清零。该证据只覆盖单屏 100% 的原生
Save 对话框目录权限；截至该记录点，B1 仍保持“部分完成”。

本次 B1 子切片补齐录屏 worker 失败后的可重试反馈：失败状态保留 FFmpeg 原始诊断，并明确提示检查 FFmpeg
和输出目录后重试；English/简体中文的长状态在固定 48px 状态栏中自动换行，不再以省略号隐藏下一步。
确定性状态测试与当前 Release 的 520x640 Record 截图均通过，证据见
`target/ui-acceptance/recording-ui-failed-retry-en.png`、
`target/ui-acceptance/recording-ui-failed-retry-zh-CN.png` 及同名 JSON。该证据仍不替代真实 FFmpeg
运行中退出和原生 Save 对话框目录权限恢复矩阵；截至该子切片记录，B1 尚未完成。

2026-09-13 的当前源码 Release `overlay-interaction-acceptance --capture-scenario recording-failure-retry`
通过 `scripts/run-dev-tool.ps1 -Release` 在单屏 `\\.\DISPLAY1`、2560x1440、DPI 96 会话中执行；runner 用
`dev-tools` 专用环境变量对真实录屏 worker 注入一次启动失败，等待 GPUI 完成失败态绘制后抓取
`02-recording-failed.png`，再点击同一个 `Record display` 主操作重试。schema 26 报告为 `status=passed`，证据位于
`target/overlay-interaction-acceptance/recording-failure-retry-b1-20260913/session-1789260976220-17344/report.json`；
`recording_failure_retry.failure_status` 保留 `Screen recording failed: acceptance-injected recording startup failure...`，
`failure_state_cleared=true`、`retry_started=true`，并确认重试目标为整块显示器物理边界。
同一 session 的 `01-recording-idle.png`、`02-recording-failed.png`、`03-recording-retried.png`、`04-paused.png`、
`05-resumed.png` 和 `06-saved.png` 已目视复核，失败、重试、暂停和保存状态均可读。真实 Pause、Resume、Stop
完成后，FFprobe 确认一个 H.264 视频流、2560x1440、5.566667 秒、371113 bytes；解码帧在 0.333333 秒与同一
显示器源帧比较，`grid_mean_absolute_error=0.5364583333333334`，低于允许值 18。`cargo fmt --all -- --check`、
`cargo test -p flash-shot-app --locked`（378 项）、全 workspace 严格 Clippy 和 Release 构建均通过；会话结束后本机
复查 `.tmp=0` 且 `flash-shot` 进程为 0。该场景不读取系统剪贴板，区域录制与窗口录制仍走原有路径；B1 的外部失败恢复
范围现标记为“已完成（单屏 100%，DPI 96）”，150%/200% 和多屏继续由 D1 单独验收。

2026-09-06 的当前源码 Release `overlay-interaction-acceptance --capture-scenario copy-only
--allow-system-clipboard` 已完成一次真实工具栏 Copy：PNG、CF_DIB 和普通消费者均逐像素一致，编辑器保留选区，
消费者已回收且最终 `capture_teardown_pending=false`。随后 `overlay-copy-batch` 在同一单屏环境完成 2 次预热和
30/30 有效样本，失败 0，p50 `25.9655 ms`、p95 `50.9424 ms`、最大值 `59.0203 ms`；批次报告中的每个样本均为
生产系统剪贴板、三路 `exact_match=true` 和 `cleanup_safe=true`。同日首次预热曾在拖选后观察到整屏选区，失败报告保留在
`target/`，不计入本次批量统计。

**顺序**：确定性 fault fixture、真实系统剪贴板争用、Quick Save 真实只读目录、原生 Save 对话框目录权限和
FFmpeg 启动失败重试均已完成；任何一类无法清理都保留失败报告并停止该切片。

### B4：执行当前 HEAD 标注回归

**目标**：把已存在的标注编辑与导出测试落实为当前 Release 的真实输入证据。

**边界**：固定单屏 100%、真实拖选、Text、Watermark、Line、正向/反向 Arrow、Quick Save 和清理；不新增工具，
不改变标注文档坐标、Undo/Redo 或导出规则。

**验收**：每个步骤的类型、内容、起终点与注入记录一致；第二个箭头不受第一个影响；导出 PNG 尺寸与选区物理尺寸
一致且像素发生预期变化；`.tmp`、窗口、任务和按键清理。报告、截图和导出文件必须来自同一 Release session。

2026-09-13 的当前源码 Release `overlay-interaction-acceptance --allow-input --capture-scenario annotation-regression`
通过 `scripts/run-dev-tool.ps1 -Release` 在单屏 `\\.\DISPLAY1`、2560x1440、DPI 96 会话中完成真实拖选、Text、Watermark、
Line、正向 Arrow、反向 Arrow 和 Quick Save。schema 26 报告为 `status=passed`，证据位于
`target/overlay-interaction-acceptance/b4-annotation-regression-current-20260913/session-1789261735559-9828/report.json`；
请求与提交选区均为 `(563,288)-(1741,720)`，物理尺寸 `1178x432`。报告记录 Text/Watermark 内容分别为
`B4 Text 中文`、`B4 Watermark 中文`，Line 起终点、两个方向相反的 Arrow 起终点以及对应步骤截图；同一 session
导出 PNG 为 `1178x432`、71672 bytes，导出指纹与源帧不同。`00-annotation-tools.png` 至
`05-annotation-arrow-reverse.png` 已目视复核，五类标注和 Layers 顺序清晰可见；cleanup 为
`session_state=completed`、overlay/Pin/可见进程窗口均为 0、`capture_teardown_pending=false`、
`capture_preflight_ready=true`，会话后本机 `.tmp=0` 且 `flash-shot` 进程为 0。该证据关闭 B4 的单屏 100% 原生回归范围，
150%/200% 和多屏仍由 D1 单独验收。

### W0：冻结 Snow Shot 工作区参考与 Flash Shot 边界（本次完成）

**结果**：已按公开 GitHub 的 `v1.0.0-beta` Release、tag 源码和 `main` 分支工作区测试文档完成研究，确认参考重点是
截图工具栏的视觉层级、次级样式行、工具组浮层、稳定几何、tooltip/键盘可达性和选区边缘呈现。当前路线不引入 Snow Shot
的设置面板、设置项组织或 Qt/Tauri/Web 架构。

**冻结决定**：Flash Shot 的截图工作区采用“选区画布 + 主工具栏 + 上下文样式栏 + 按需浮层”的单一操作区域。Copy、Save、
Pin、Cancel 继续是结果主动作，More 收纳低频动作；现有 GPUI、`ThemeMetrics`、`UiText`、标注文档坐标、操作代次、报告字段和
导出合成路径保持不变。W1-W6 只调整 UI surface、布局和控件交互，每个切片独立验证、提交和推送。

### W1：工作区视觉基础与共享控件

**目标**：在不改变任何截图、标注和导出行为的前提下，建立截图工作区可复用的视觉与几何基础。

**负责范围**：`crates/flash-shot-app/src/app/theme.rs`、`overlay.rs`、`i18n.rs`；若 `overlay.rs` 的职责边界已由行为测试冻结，
再按现有应用模块风格新增 `app/overlay_toolbar.rs`，只承载工作区呈现和布局辅助，不提前拆业务状态机。

**交付内容**：

1. 在 `ThemeMetrics` 和语义颜色中补齐 toolbar surface/elevated/border/hover/active/focus、主行/样式行高度、内边距、间距、
   icon button 命中区、popover gap、shadow margin 等 token；颜色、半径和几何不得在按钮分支中散落硬编码。
2. 提供统一的工作区 surface、icon button、separator、swatch、tooltip/accessibility 和 popover shell；保留现有稳定语义 ID、
   handler、快捷键和 busy/disabled 状态。
3. Copy、Save、Pin、Cancel、Undo/Redo、标注入口等熟悉操作优先使用图标；每个图标必须有 `UiText` tooltip 和可访问名称，
   中英文长文案仍由容器负责换行或调整，而不是截断。

**验收**：纯布局/状态测试覆盖无选区、选区、忙态、禁用、焦点和错误状态；English/简体中文、浅色/深色、420x420、520x640、
980x760 均无截断和重叠；Capture、Copy、Save、Pin、Cancel 的 handler 调用次数与改造前一致；`git diff` 中不存在新的工作区
固定颜色、固定总宽度或与 `ThemeMetrics` 重复的 token。2026-09-13 已完成 W1：`48` 个 overlay 布局/状态测试通过，当前源码 Release
生成的 12 组 `overlay-marking` PNG/JSON 全部 `scale_match=true`，四组合 Pin lifecycle 矩阵通过；本次只关闭单屏 100% 的 W1 范围，
150%/200% DPI、负坐标双屏和混合 DPI 仍由 D1/W6 处理。

**独立提交建议**：`feat: add screenshot workspace toolbar foundation`。

### W2：主工具栏重排

**目标**：让截图工作区的常用动作形成一条稳定、可扫描、可预测的主操作行，同时保留现有行为和键盘路径。

**布局决定**：截图结果工具栏以用户提供的 Snow Shot 截图为逐项视觉验收基准，采用单行深色浮动条、统一方形图标命中区、居中线性图标、细分隔线和紧凑组间距。左侧保留拖动把手与模式/选区操作，中段按截图顺序排列现有标注工具和 Undo/Redo，右侧按截图顺序排列已有网格/识别/固定/翻译/保存/取消/复制动作；不为对齐而改变 handler、快捷键、权限、导出或失败恢复语义。每个图标继续提供 `UiText` 可访问名称与 tooltip，活动工具使用主题强调色，取消使用破坏性样式，复制保留主动作强调。

**实现约束**：保留 `overlay-annotation-controls`、`overlay-pin`、`overlay-copy`、`overlay-save`、
`OVERLAY_MORE_ACTIONS_ID`、`overlay-cancel` 及 Alt+M；保留 Copy busy、Save/Pin disabled、Escape 取消和无选区 fallback。
用可见控件的 intrinsic size、token 间距和当前窗口安全边距计算布局，逐步移除对 `OVERLAY_ACTION_BAR_WIDTH=620` 以及每个动作
固定宽度的依赖；布局结果交给 W5 的 `WorkspaceLayoutSnapshot` 统一保存。

**验收**：逐项对照用户截图核对单行结构、按钮尺寸/基线、图标形态、分组顺序、分隔线和留白；同一状态下每个主动作最多触发一次；点击工具栏不会穿透到画布；主行在三种窗口尺寸和中英文下不遮挡选区、HUD 或其他浮层；Copy/Save/Pin/Cancel 的结果、快捷键、错误恢复和清理报告与现有行为一致。

**本次完成（2026-09-13）**：主行改为固定命中区和分组结构，Mark、Undo/Redo 与结果动作按上下文排列；Copy 保持唯一主强调，
Cancel 保持破坏性样式；More 使用稳定 element ID、动态展开图标、Alt+M 和菜单内方向键导航。低频保存、滚动截图、二维码、OCR、
颜色复制、翻译和录屏继续留在按需面板。`cargo test -p flash-shot-app --lib overlay::tests` 的 48 个测试、全 workspace 严格
Clippy 和格式检查通过；当前源码 Release 生成的 `w2-final2-selection`、`w2-final2-selection-more`、
`w2-final2-marking-en`、`w2-final2-marking-zh-CN` 四组 PNG/JSON 均为 `scale_match=true`。本次只关闭单屏 100% 的静态
渲染范围，真实鼠标键盘命中、高 DPI 和多屏仍由 W6/D1 验收。

**独立提交建议**：`feat: reorganize screenshot workspace main toolbar`。

### W3：上下文样式栏

**目标**：把标注样式控制收敛为跟随当前工具或选中对象的附属行，避免默认显示杂乱的侧向/纵向设置面板。

**交付内容**：

1. 样式栏只在当前标注工具或选中对象需要时出现，与主工具栏共享 surface、边框、阴影、焦点和命中区规则；在选区上方空间不足时，
   整行移动到下方或安全边界内，不能拆成互相覆盖的多个面板。
2. 复用现有 `AnnotationStyle` 和领域规则，只映射已经支持的颜色、线宽、填充/透明度、字体大小等属性。Shape、Line/Arrow、
   Freehand、Highlight、Text/Watermark/Number、Blur/Mosaic 只显示各自相关控件，不新增标注工具或导出语义。
3. 颜色使用 swatch，数值使用紧凑控件，active/mixed/disabled/busy 状态可见；连续调整按“预览后释放提交”执行，单击控件只产生
   一个领域命令。对象删除、复制、层级调整等低频动作进入上下文溢出入口，不挤占样式栏。

**验收**：每类现有标注工具的样式栏均能打开、切换、Escape 关闭并恢复焦点；样式预览、Undo/Redo、文档坐标、Quick Save 和
导出 PNG 像素与改造前一致；无激活工具时不显示空样式栏；中英文、双主题和三种窗口尺寸均无溢出。

**本次完成（2026-09-13）**：`AnnotationStyleCapabilities` 根据当前标注工具和选中对象只暴露已有渲染器支持的颜色、透明度、填充、线宽和字体大小控件；样式行复用 `AnnotationStyle`、工作区 surface 和共享命中区，默认线宽 `4` 也纳入可选值。样式行按可用宽度换行，主工具行对简体中文使用紧凑估算，避免在宽屏保留空行。`cargo test -p flash-shot-app --lib overlay::tests` 的 49 个测试通过；当前源码 Release `settings-ui-acceptance` 生成
`target/ui-acceptance/w3-workspace-current-20260913/` 下 6 组 `overlay-marking` PNG/JSON，覆盖 1100/900/700/520/360 宽度、深色 English 和浅色简体中文，全部 `scale_match=true`，已目视复核无截断或重叠。该切片只证明单屏 100% 的静态渲染和布局，真实样式点击、Escape/焦点回收、150%/200% DPI 及多屏仍由 W6/D1 验收。

**独立提交建议**：`feat: add contextual screenshot style row`。

### W4：工具组与 More 浮层

**目标**：将同类工具和低频能力放入可发现、可关闭、可回到原焦点的短生命周期浮层，同时避免悬停或异步回调造成误操作。

**交互规则**：

1. 以数据驱动的 `ToolGroup` 描述工具项和现有 handler；首批只整理已有能力，例如 Shape（Rectangle/Ellipse）、Line（Line/Arrow/
   Freehand）、Obscure（Blur/Mosaic）和 Text（Text/Watermark/Number）。Highlight、Scroll、OCR、Record 等是否分组只按当前
   handler 和可用状态决定，不为了模仿参考产品增加工具。
2. 悬停只负责发现，点击、Enter 或 Space 才确定选择；支持方向键移动、Escape 关闭、外部点击关闭和关闭后焦点返回触发按钮。
   More 必须支持点击和键盘操作，不能设计成悬停才能使用的入口；tooltip 不得代替可访问名称。
3. 工具组按需 materialize，工作区只保留一个浮层所有者；打开、关闭和操作代次变化时清理旧回调、捕获和 busy 状态。浮层命中先在
   本地/`PreviewTransform` 坐标完成，最终放置才换算全局坐标，禁止混用不同 DPI 下的全局矩形交集。
4. More 保留滚动、OCR、二维码、颜色复制、翻译、录屏、保存标注/可编辑结果以及识别进行中/成功/失败/重试/复制/清除状态；不把
   Snow Shot 设置项或 Flash Shot 全局设置塞进该浮层。

**验收**：工具组与 More 的点击、键盘、焦点、Escape、外部关闭和重复打开均无鬼影/重复实例；选中工具只提交一次命令；浮层不
穿透画布、不丢失选区、不留下任务或窗口；各状态的中英文文案、tooltip 和错误恢复可见。

**本次完成（2026-09-13）**：标注工具栏已将 Text/Watermark/Number、Rectangle/Ellipse、Line/Arrow/Freehand 和 Blur/Mosaic
收敛为四个按需工具组；每个显示器只保留一个工具组所有者，打开新组会关闭 More，重置、取消、滚动截图和覆盖层关闭都会清理组状态。
工具组支持点击打开、方向键移动、Enter/Space 选择、Escape 关闭、外部点击关闭，并在关闭后把焦点交还给触发按钮；选中子工具后
只提交一次既有 `AnnotationTool` handler，不改变标注文档和导出语义。`settings-ui-acceptance` 的
`w4-tool-group-en-dark-final.png` 与 `w4-tool-group-zh-CN-light-final.png` 均为 `scale_match=true`；真实 Windows 单屏 100%、DPI 96
的 `overlay-interaction-acceptance --capture-scenario tool-group` 报告
`target/overlay-interaction-acceptance/more-outside-tool-group-final/session-1789803426675-25240/report.json`
为 schema 28、`status=passed`，新增验证 More 打开后外部点击关闭且选区保持不变（`more_opened=true`、
`more_outside_closed=true`），并继续覆盖 Text/Shape 打开、Escape、键盘 Watermark、鼠标 Rectangle、工具组外部关闭和最终清理。
该切片仍不替代 W6 的完整状态矩阵、150%/200% DPI 和多屏验收。

**独立提交建议**：`feat: add screenshot tool group popovers`。

### W5：选区锚定、HUD 与布局快照

**目标**：让选区边缘、尺寸 HUD、放大镜、主工具栏、上下文样式栏和浮层在小选区、边缘选区及不同 DPI 下协同稳定。

**实现约束**：保留 `paint_selection_mask` 的遮罩、边框、拖拽手柄、尺寸和放大镜语义；只在 overlay paint 路径增加借鉴 Snow Shot
的边缘层次、checkerboard/阴影和安全间距，导出 compositor 不得读取这些装饰状态。新增 `WorkspaceLayoutSnapshot`，一次布局中
同时记录主行、样式行、More/工具组浮层、选区锚点和安全边距；按上方/下方空间选择放置方向，不足时 clamp 到可见工作区，相关行
必须一起移动。

布局计算统一经过 `PreviewTransform` 的逻辑/物理坐标转换，不在事件处理器里写全局固定位置。工具栏和浮层使用 occlude/等价机制
阻止鼠标事件穿透画布，但画布拖选、调整手柄、键盘微调和取消路径保持可用。

**验收**：覆盖零/极小选区、全屏选区、四边贴边选区、拖动、调整大小、标注和浮层打开；无 stale frame、布局重算抖动、控件重叠
或 HUD 遮挡；导出像素、尺寸、标注文档坐标、Copy/Save/Pin 和清理状态与 W1 前一致。

**本次进展（2026-09-19）**：提交 `55a3ba0` 新增 `WorkspaceLayoutSnapshot`，在一次布局计算中统一记录安全边距、`PreviewTransform`
选区锚点、主工具栏、标注工具栏、More 菜单、工具组浮层、尺寸 HUD、智能目标 HUD 和状态栏安全位置；渲染路径改为复用该快照，
避免主行、样式行和浮层分别按旧坐标重复计算。新增两个边缘/工具组确定性布局测试，`overlay::tests` 共 53 项通过。
同一提交的隔离 Release 又在 2560x1440、DPI 96 单屏环境完成右下 160x96 窄选区原生验收：真实打开并关闭 More 和 Mark，
报告中的请求选区、提交选区和源帧边界一致，结束后 `session_state=idle`、overlay/Pin/可见进程窗口均为 0、
`capture_teardown_pending=false` 且 `capture_preflight_ready=true`。报告位于
`target/overlay-interaction-acceptance/w5-layout-snapshot-narrow-edge-final3/session-1789809143852-28936/report.json`。
2026-09-24 的后续切片把物理选区到视图坐标的转换集中到 `WorkspaceSelectionAnchor::from_selection`，零面积选区不再生成
工具栏、标注行、浮层或尺寸 HUD。确定性矩阵覆盖横向/纵向零面积、1px、全屏以及顶部/底部/左侧/右侧贴边选区，逐项检查
主工具栏和尺寸 HUD 位于安全区域且互不重叠；`cargo test -p flash-shot-app --lib overlay::tests --all-features --locked` 的 55 项
全部通过。2026-09-25 的后续 Release 原生矩阵在单屏 2560x1440、DPI 96 环境逐项执行零高度、零宽度、1x1、全屏、顶部、底部、左侧和右侧
选区；所有正面积用例的提交选区、Copy sink 帧尺寸和边界一致，零面积保持无选区，8 个用例均完成 Escape/Copy 清理，状态复位为 Ready。
报告为 `target/overlay-interaction-acceptance/w5-boundary-matrix-20260925-codex-v16/session-1790304205107-22756/report.json`，schema 30、`status=passed`，
使用隔离进程内 sink，未修改系统剪贴板。该证据关闭 W5 的单屏 100% 原生边界子项；150%/200% DPI 与多屏继续按 D1 暂缓，完整双主题/双语状态矩阵进入 W6。

**独立提交建议**：`feat: anchor screenshot workspace layout to selection`。

### W6：双主题、双语、DPI 与真实输入验收

**目标**：用同一提交构建的 Windows Release 证明截图工作区在完整状态矩阵下可操作、可理解且不影响截图主链。

**状态矩阵**：无选区、普通选区、标注中、选中 Shape、Text/Watermark/Number、混合样式、More 打开、工具组打开、Copy busy、
识别进行中/成功/重试/失败，以及顶部/底部/左右边缘和极小选区。每个状态覆盖 English/简体中文、浅色/深色和 420x420、520x640、
980x760；在具备硬件时执行 100%/125%/150%/200% DPI，缺少真实硬件的矩阵继续明确记为 D1 暂缓。

**原生验收**：真实 Windows Release 使用鼠标和键盘完成拖选、调整、工具选择、样式修改、Copy/Save/Pin/Cancel、More、Escape、
外部关闭和快捷键；保存结构化报告、关键截图、导出 PNG/逐像素结果、DPI/显示器信息以及窗口、任务、按键、剪贴板和临时文件清理
结果。验证 action count、焦点回收、浮层无残留、长文案无截断，并复查 B1 失败恢复和 B4 标注回归没有被 UI 改造削弱。

**视觉改造设计与结果（2026-09-25）**：以用户提供的 Snow Shot 工作区截图作为视觉验收基准，收敛标注工具与样式区域而不扩大功能范围。标注工具采用紧凑图标入口，完整名称由双语无障碍名称和悬停提示提供；选中工具后仍通过既有工具组浮层选择具体工具。主工具行与 Copy/Save/Pin/Cancel/More 结果动作已合并为同一条横向浮动栏，样式栏在活动工具需要样式时作为下方上下文行出现；保持主动作强调、取消破坏性样式、既有 handler 和辅助技术名称不变。工具面板宽度按可见控件的自然需求计算，不再无条件占满 900px。84 组 Release 截图还发现 More 浮层的 QR、OCR 与可编辑项目中文标签超出固定按钮宽度；加宽对应按钮并复算菜单布局后，420x420 中文 More 截图已无标签互相覆盖。深浅主题下的截图覆盖层控制保持同一深色外观，应用其他页面主题不变。布局仍由 `WorkspaceLayoutSnapshot` 锚定选区并避开显示边界；物理像素、标注文档、快捷键、Copy/Save/Pin/Cancel、焦点与清理语义不变。

**补充视觉验收基准与结果（2026-09-25）**：用户再次提供的截图明确替代 W2 中“工具、样式、结果动作分行”的初始呈现；验收基准为横向单行结果工具栏，含一致的正方命中区、图标中心对齐、分组分隔和紧凑间距。最终 Release 原生 `tool-group` 会话在 `\\.\DISPLAY1`、DPI 96 完成拖选，More 点击/外部关闭，标注工具组打开、Escape 关闭、方向键选择 Watermark、Shape 外部关闭、鼠标选择 Rectangle 和退出清理；报告 `target/overlay-interaction-acceptance/w6-tool-group-snow-shot-unified-v2/session-1790318461798-30568/report.json` 为 `status=passed`，清理后 overlay/window 为 0 且 capture preflight ready。`02-tool-group-toolbar.png` 与 Watermark 的 `06-tool-group-shape-open.png` 已目视复核；同一源码生成的 English/简体中文 980x760 Release 图用于确认两种语言下的行内布局。该证据完成本次工具栏视觉切片和工具组交互，不代表剪贴板、Save、Pin 各动作原生主链或 W6 整体已完成。

**视觉矩阵验收（2026-09-25）**：最终单行工具栏源码 Release 的报告为 `target/ui-acceptance/w6-toolbar-single-row-final-20260925/matrix-report.json`，84/84 通过 DPI 96、scale 1.0、语言和窗口边界 metadata 校验；目视复核了 420x420、520x640、980x760 的代表截图，覆盖深浅主题、中英文、标注、工具组和小屏 More。该矩阵只证明渲染和布局；真实工具组鼠标/键盘行为由单独的 `tool-group` 原生报告证明；真实系统剪贴板或 Copy/Save/Pin/Cancel 完整动作主链仍未由本切片覆盖。

**视觉对齐修订（2026-09-26）**：按用户提供的 Snow Shot 截图，将已提交选区的默认状态改为直接显示完整横向编辑工具栏；选择/移动工具置于标注工具组之前，复制保持蓝色主动作，取消保持红色破坏性动作，工具栏背景收敛为中性黑灰。420x420、520x640、980x760 的 Release 截图分别为 `target/ui-acceptance/w6-toolbar-final-20260926-dark-420.png`、`w6-toolbar-final-20260926-dark-520.png` 和 `w6-toolbar-final-20260926-dark-980.png`，均已目视复核无重叠或截断；`overlay::tests` 57 项、工具组原生输入报告 `target/overlay-interaction-acceptance/w6-tool-group-final-20260926/session-1790419931266-11048/report.json` 均通过。Computer Use 当前会话仅暴露浏览器接口，未能发现 Windows 原生窗口；本次原生交互证据来自项目 Release runner，静态截图不替代未覆盖的高 DPI、多屏和完整系统剪贴板主链。

**工具栏图标对齐修订（2026-09-26）**：将截图工作区按钮从字体 Unicode 字符改为统一 16px 画布内的矢量线稿，统一笔画宽度、内边距和视觉中心；工具顺序固定为选择/移动、标注工具组、撤销/重做、Pin、Save、More、Cancel、Copy，Copy 仍为唯一蓝色主动作，Cancel 仍为红色破坏性动作。`target/ui-acceptance/w6-toolbar-icons-20260926/overlay-tool-group-{420,520,980}.png` 及同名 JSON 均由当前 Release 生成并目视复核，三种尺寸均无图标偏移、重叠或截断；布局测试同步更新为 559px 标注态工具栏宽度。

**W6 主动作验收（2026-09-26）**：当前 `main` 的 Release runner 已完成单屏 2560x1440、DPI 96 的完整 Capture -> 重捕获 -> Cancel -> Save -> Pin -> Copy -> Escape 清理链，报告 `target/overlay-interaction-acceptance/w6-standard-final-20260926/session-1790420532956-25568/report.json` 为 schema 30、`status=passed`；Save/Pin/隔离 Copy 均与源帧 `exact_match=true`，Cancel 对话框恢复选区，最终 overlay/window/task/input 全部清零。当前提交的真实系统剪贴板 Copy-only 报告 `target/overlay-interaction-acceptance/w6-copy-system-final-20260926/session-1790420623195-24696/report.json` 记录 PNG、CF_DIB 和独立消费者逐像素一致，`consumer_ready_before_click=true`、`consumer_observing_before_click=true`、`consumer_cleaned_up=true`。W6 在当前单屏 100% 范围的主动作证据完成；125%/150%/200% DPI、多屏和其他设备继续由 D1 暂缓，不把环境缺失写成通过。

**独立提交建议**：`test: verify screenshot workspace toolbar`。

**第一块已交付（2026-09-25）**：新增 `scripts/run-w6-workspace-matrix.ps1`，固定覆盖 2 个主题、2 种语言、3 种窗口尺寸和 7 个工作区 surface，共 84 个隔离 `settings-ui-acceptance` Release 会话。每个会话验证 PNG、相邻 JSON、locale、DPI/scale、物理窗口边界和退出码，并在全部用例结束后生成可复核的 `matrix-report.json`；`-DryRun` 可在不启动应用时检查矩阵规模和报告格式。

当前 Release 报告为 `target/ui-acceptance/w6-workspace-matrix-20260925-v2/matrix-report.json`，`status=passed`、`case_count=84`、`passed_count=84`、`failed_count=0`；English/简体中文和浅色/深色代表截图已目视复核。该切片是静态状态与布局证据，不包含真实鼠标/键盘、系统剪贴板、Copy/Save/Pin/Cancel 或高 DPI/多屏行为；这些验收继续由 W6 原生输入和 D1 负责，不能据此标记 W6 完成。

**W 路线执行顺序**：B1 外部失败恢复、B4 当前 HEAD 的真实标注输入、W1 工作区视觉基础与共享控件、W2 主工具栏重排、W3 上下文样式栏、W4 工具组与 More 浮层以及 W5 选区边界矩阵均已完成当前环境范围；W6 在当前单屏 100% 范围的状态矩阵、工具栏视觉、工具组和 Copy/Save/Pin/Cancel 主动作验收已完成，下一步转入 U1 动态文案迁移。高 DPI/多屏仍由 D1 暂缓。
W0 的路线文档切片已完成，随后严格按 W5、W6 顺序推进。W1-W4 新增的可见文案同时登记到 `UiText`；U1 继续负责非截图工作区的
动态文案，U2 继续负责 App/Library/Record 入口，U3 负责其他页面视觉矩阵，U4 负责 Pin 原生窗口。W6 通过后才开始 M1 的 runner/
overlay 职责拆分，避免在工作区行为尚未稳定时搬移大型模块。

**2026-09-25 后续执行顺序**：后续切片直接在 `main` 开发，不重建或经由 `dev` 分支；每个独立可验收功能先完成风险匹配的 Rust 检查和 UI 验收，再以一次独立 Conventional Commit 推送到 `main`。下一步继续 W6 的 100% 单屏真实鼠标/键盘输入与 Copy/Save/Pin/Cancel 验收；随后仅在具备真实硬件时补充 DPI 证据，再按 U1-U4、M1 最小职责拆分和 `0.2.0` 发布收尾的顺序推进；不得用确定性布局或单个静态探针替代原生验收。

**2026-09-26 后续执行顺序**：W6 当前单屏 100% 主动作验收已完成；继续在 `main` 上推进 U1，优先迁移截图 workflow 中仍直接拼接的用户可见动态状态，保持错误详情、路径和数量作为参数，不改变状态机、快捷键、报告字段或导出协议。每个 U1 子切片先做中英文参数化测试和 Release 静态/原生可达性检查，再独立提交并推送；D1 高 DPI/多屏证据在具备对应硬件后补齐。

### U1：完成动态文案迁移

**目标**：用户可见的动态状态全部由 `Locale`/`UiText` 提供，错误详情和路径作为参数保留。

**边界**：`workflow.rs`、`workflow/*.rs`、Record/OCR/更新/历史/Pin 状态和 UI 状态指示器；不修改业务状态机、
快捷键、报告字段或外部服务协议。

**验收**：English/简体中文的按钮、忙态、失败、重试、数量、耗时和路径均有资源键和参数化测试；静态盘点不再以旧
计数作为事实；缺少 OCR 或翻译配置不能阻塞截图主链。若文案变长造成溢出，转入 U2/U3 修复容器，不截断错误。

**当前进展**：提交 `c39572a` 将 `settings-ui-acceptance` 的 Record 状态种子改为使用活动 `Locale`/`UiText`，覆盖
Starting、Recording、Paused、Stopping、Failed 和 Cancelled；English 与简体中文的 520x640 Release 截图均已生成、
目视复核并登记。提交 `8356640` 又将打开图片时标注附属文件加载失败的路径、错误前缀和双语文案统一迁移到
`UiText`，并以 workflow 测试验证原始错误细节和 English/简体中文输出。2026-09-26 的第一组 U1 状态探针又覆盖
Record 暂停/失败、Translation 测试中和 Local OCR 检查中；`target/ui-acceptance/u1-dynamic-status-20260926/` 下四张
English/简体中文 Release 截图及同名 JSON 均通过 `scale_match=true`，并已目视复核无截断或重叠。该切片仍只完成局部
动态文案迁移，不代表生产录屏、真实翻译端点、OCR 依赖失败重试和其他 workflow 的动态文案已全部迁移；真实窗口中的
打开图片失败提示仍需后续原生验收。

随后在 `main` 上完成八个独立 U1 子切片：`9ca0790` 将 Library 条目的文件名、来源和相对时间改为
`LibraryEntryLabel` 参数模板；`ee2f014` 将截图覆盖层图层列表的序号和工具名称改为 `OverlayLayerLabel`；
`868b4f2` 将 Pin 不透明度按钮的百分比改为 `PinOpacityValue`；`690addc` 将标注样式栏的不透明度百分比改为
`AnnotationOpacityValue`；`3cd8d37` 将 Library 快速保存文件名预览改为 `LibraryFileNamePattern`；`93cc3af` 将标注样式栏的线宽和文字大小数值改为 `AnnotationWidthValue` 与 `AnnotationTextSizeValue`；`e67e4b9` 将选中序号标注的动态数字改为 `AnnotationNumberValue`；`1714ec3` 将录屏显示器的序号和物理尺寸改为 `RecordingDisplayLabel` 参数模板。八个切片均补充 English/简体中文参数化测试，并分别通过对应模块测试、
`cargo fmt --all -- --check` 与 `cargo clippy --workspace --all-targets -- -D warnings`。随后以当前 Release
运行 `settings-ui-acceptance` 生成 `target/ui-acceptance/u1-library-prefix-{en,zh}-20260926.*`、
`u1-overlay-marking-{en,zh}-20260926.*` 和 `u1-pin-{en,zh}-20260926.*` 六组 PNG/JSON；全部记录
`dpi=96`、`scale_factor=1.0`、`scale_match=true`，Library 中文和 Pin English 图已目视复核无截断或重叠。
`93cc3af` 又以当前 Release 生成 `u1-overlay-values-{en,zh}-20260926.*` 两组 PNG/JSON，均记录
`scale_match=true`，用于复核线宽、文字大小和透明度样式行的中英文紧凑布局。
`1714ec3` 又生成 `u1-record-display-{en,zh}-20260926.*` 两组 PNG/JSON，均记录
`scale_match=true`，用于复核 Record 页面中英文显示器选择布局。
提交 `9f31942` 补齐录屏目标、进度和停止状态的双语参数化测试，覆盖显示器、窗口和所选区域三类目标；
当前 Release 又生成 `u1-record-progress-{en,zh-CN}-20260926.*` 与
`u1-record-stopping-{en,zh-CN}-20260926.*` 四组 PNG/JSON，均记录 `dpi=96`、`scale_factor=1.0`、
`scale_match=true`，并目视复核状态栏和底部反馈无截断或重叠。该切片只增加确定性双语验收覆盖，
不宣称已完成真实录屏、真实窗口输入或高 DPI/多屏验证。
本次没有真实 Computer Use 原生窗口输入证据；U1 仍保持“部分完成”。

### U2：收敛入口和恢复动作

**目标**：Capture、Library、Record、App 各自只有一个最强主要动作，低频能力可找到且不与主流程竞争。

**边界**：GPUI 页面导航、设置快捷入口、Library 筛选/批量操作、Record 生命周期和诊断入口；截图工作区的主/次动作、样式栏和
工具组由 W2-W6 负责；不删除唯一可完成核心流程的入口。

**验收**：三种窗口尺寸均能完成截图、标注、Copy、Save、Pin、OCR 重试和 Record 取消/清理；主/次/破坏性/忙/错误/恢复
状态可区分；稳定语义 ID 不依赖显示文案；真实键鼠验收通过后才能关闭切片。

**当前切片（2026-09-09）**：Record 页将 `Record display`（录制或失败后的再次尝试）渲染为唯一强调动作，
`Check support` 保持次要按钮；录制目录和操作区继续使用可换行布局。当前仅完成动作层级和无输入 Release 截图验收，
App/Library 的重复入口收敛、真实键鼠重试和三种窗口尺寸的完整矩阵仍未完成，不能将 U2 标记为通过。

### U3/U4：完成视觉与 Pin 原生矩阵

**目标**：把已完成的 token 和隔离探针扩展到真实中英文输入与窗口生命周期。

**边界**：界面度量（ThemeMetrics）、按钮/工具栏命中区、焦点态、tooltip、Pin 复制/保存/关闭和其他页面的语言/主题组合；截图工作区
的新增 surface、布局和状态由 W1-W6 负责；不改变截图像素、快捷键和报告 schema。

**验收**：English/简体中文、浅色/深色、420x420/520x640/980x760 均无截断、重叠或状态遮挡；Pin 真实点击、关闭和
再次 Capture 可恢复；已打开窗口的语言切换策略明确，若不能安全刷新必须提示重新打开。

### M1：按职责拆分大型模块

**前置**：B1-B4、W1-W6、U1-U4 的行为证据稳定，且当前报告 schema 有固定回归样本。

**顺序**：先把 `overlay-interaction-acceptance` 按 Capture/Copy/Pin/Scroll/Recording 和共享 Windows 输入设施拆分，
再把 `overlay.rs` 按渲染、输入、选择变换、菜单/工具状态和导出生命周期拆分。每次只迁移一个职责，保持 CLI 参数、
输出目录、报告字段、快捷键和用户行为不变。出现差异立即停止，不与行为修复同一提交。

### R1：提取可复用截图核心 crate

**决策**：可行，但先提取平台无关的请求、截图帧、标注合成、导出和取消边界；不把 `flash-shot-app`、GPUI、Windows 窗口、
系统剪贴板、FFmpeg、历史和设置一起发布。完整边界、候选 API 和依赖关系见[开发设计思路](architecture.md)的“可复用截图核心
crate 评估”。

**前置条件**：B1-B4 的失败与清理语义稳定；`flash-shot-app::platform` 的迁移期兼容导出不再被核心流程直接依赖；
`flash-shot-domain`、`flash-shot-image` 的公共类型和 Cargo feature 已完成 API 审计；许可证、MSRV 和发布仓库策略已确认。

**交付顺序**：

1. 在现有 workspace 内提取私有 `capture-core` 模块，保持 Flash Shot 行为、报告 schema 和 Windows 适配器不变；
2. 为请求、后端 trait、结构化错误、取消检查点、原子导出和 mock backend 增加独立测试及一个最小示例；
3. 冻结公共 API 后创建候选 `flash-shot-capture-core` crate，按 Cargo feature 控制编码、字体和二维码等可选能力；
4. 只有跨平台编译、golden image、资源上限、失败恢复和许可证说明齐全后，才评估发布到 crates.io 或提供 C ABI/IPC。

**当前边界**：R0 只完成架构评估和文档决策，不新增公共 crate、不宣称第三方兼容、不改变 `0.1.3` 稳定版安装包。

### D1：真实单显示器高 DPI

在真实 150% 和 200% Windows 设备分别验证自由拖选、智能目标、键盘微调、标注、Copy/Save/Pin、滚动、录屏、
三种设置窗口尺寸和清理。报告必须记录实际 DPI/scale、显示器物理边界、导出像素和窗口/任务状态。双屏和翻译端点
仍是独立暂缓项，不能由 D1 推断通过。

### P3：可扩展插件平台（`0.3.0+`）

**版本定位**：`0.3.0` 的目标是交付可选能力宿主和第一批内置插件；`0.3.1+` 才评估进程外第三方插件兼容。
`0.3.0+` 是平台路线，不代表当前 `0.1.3` 或 `0.2.0` 已提供插件 API。插件平台必须降低核心安装包体积和维护耦合，
但不能把截图主链的窗口、输入、剪贴板和资源清理责任转移给插件。

**前置条件**：B1-B4、U1-U4 和 M1 的行为证据稳定；当前 Release 构建、安装器、便携包、manifest 和下载复核通过；
插件相关设计先落在本计划与[开发设计思路](architecture.md)，不先添加未实现的公共 Rust crate、动态库 ABI 或网络市场。

**范围边界**：主程序提供插件宿主、版本化插件清单、能力/权限声明、进度/取消/错误接口和有界帧流。录屏与 GIF
先作为内置插件验证；首期不承诺第三方市场、任意 Rust 动态库 ABI、网络插件、脚本插件或跨平台功能。插件不能直接操作
HWND、全局快捷键、系统剪贴板或 `CaptureSession` 内部状态，必须通过宿主接口申请。

#### P3.0：冻结插件接口约定

**交付物**：定义 `PluginManifest`、`PluginCapability`、`PluginPermission`、`FrameStream`、`ExportRequest`、
`PluginEvent` 和 `PluginError` 的 schema、版本策略、字段上下界和兼容规则；清单至少包含插件 ID、插件版本、API 版本、
入口、能力、权限、最大帧率/时长/尺寸/输出大小和本地化显示信息。

**验收条件**：未知 API 版本、缺失必填字段、超出资源限制或未声明权限时在启动前拒绝；同一清单重复加载不产生重复任务；
宿主能将取消、超时、崩溃和输出失败统一转换为可本地化的错误；截图主链在没有任何插件目录时启动结果不变。

#### P3.1：内置录屏插件适配

**交付物**：把现有 FFmpeg 录屏 workflow 适配为宿主管理的内置录屏插件，保留已有 `RecordingBackend`、Job Object、
启动/停止超时、stdout/stderr 限制和失败重试行为；录屏插件只接收宿主批准的目标、帧率和输出路径。

**验收条件**：FFmpeg 缺失、启动失败、运行中退出、停止超时、取消和输出目录失败均能回收进程、任务、临时文件和状态；
不安装 GIF 或第三方插件不影响录屏和截图主链；English/简体中文反馈、MP4 产物和清理报告来自同一 Release session。

#### P3.2：GIF 导出插件

**交付物**：实现消费 `FrameStream` 的内置 GIF 导出插件；宿主按固定帧率、最大时长、最大尺寸和内存预算提供带时间戳的
不可变 `CaptureFrame`，插件负责调色板、编码、临时文件和原子改名，不直接采集桌面。

**验收条件**：空流、单帧、时间戳倒退、超限、取消、磁盘不足和编码失败均有确定性测试；成功 GIF 的尺寸、帧数、帧间隔、
文件大小和像素抽样可核验；失败不会留下 `.tmp` 或半成品，未安装插件时核心安装包不携带 GIF 编码依赖。

#### P3.3：进程外插件宿主

**交付物**：以版本化 IPC/stdio 启动独立插件进程，宿主负责清单校验、能力授权、请求 ID、进度、取消、超时、标准输入输出
限制、子进程回收和诊断；首版只允许声明过的 `FrameStream`/导出能力，不开放任意文件或窗口访问。

**验收条件**：插件崩溃、无响应、协议错误、恶意大消息、取消竞态和宿主关闭都能在固定时间内结束；Job Object/等价进程组、
临时目录和句柄清零；协议不兼容时拒绝加载而不是降级执行；第三方插件不能阻塞截图主链或读取未授权内容。

#### P3.4：可选组件发布与兼容策略

**交付物**：定义插件安装目录、清单缓存、卸载/回滚、API 兼容矩阵和可选组件打包方式；先支持本地手工安装和宿主诊断，
市场、自动下载、签名和跨平台分发另立决策记录。主安装包、内置插件包和第三方插件包分别生成尺寸与依赖报告。

**验收条件**：无插件、仅内置插件、缺失插件、旧 API 插件、重复 ID、安装中断和回滚后的启动/截图主链均通过；发布资产、
SHA-256、manifest、English/简体中文说明和插件增量大小均可复核。未完成签名、市场或安全评估前，不宣称第三方生态可用。

**退出条件**：完成 P3.0-P3.2 才能标记 `0.3.0` 候选；P3.3 的进程隔离、权限和故障证据完成后才可开放第三方插件预览；
P3.4 的发布与兼容证据完成后，才评估正式插件 SDK 或市场。任何阶段未通过资源清理、权限边界或安装包增量检查，保持“部分完成”，
不把插件能力写入当前稳定版默认功能。

## 5. 统一验证与提交条件

每个代码切片按风险执行直接测试，再从 workspace 根目录执行：

```powershell
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --check
```

使用 `dev-tools` 特性的检查还需运行 `cargo check --workspace --all-targets --all-features --locked` 和
`cargo test --workspace --all-features --locked`；在 Windows 主机可用时再运行
`cargo check -p flash-shot-app --target x86_64-pc-windows-msvc --all-targets --all-features --locked`。
严格全特性 Clippy 已在提交 `1c41e8a` 通过；后续切片仍须重复执行该检查。

用户可见切片还必须使用同一提交构建的 Release 程序，在 [Windows 手工验收记录](windows-manual-acceptance.md) 中保存
结构化报告、关键截图、像素产物和清理结果。`--allow-input`、系统剪贴板和 FFmpeg 只在明确授权的可丢弃 Windows
会话使用；静态探针或旧报告不能替代真实输入。

W1-W6 还必须保留一组可重复的布局/状态测试，并在 W6 以同一 Release 完成截图工作区状态矩阵。视觉截图只证明 surface、几何、
文案和状态层级；Copy/Save/Pin/Cancel、标注文档坐标、导出像素、快捷键、焦点回收和窗口/任务/剪贴板/临时文件清理必须由对应
原生验收字段单独证明。Snow Shot 的公开测试文档可作为检查项来源，但不能替代 Flash Shot 的 Windows 证据。

验证失败、证据不足或只完成子集时，保持“部分完成/待执行”，不提交为完成状态。验证通过后只提交一个独立功能，
提交信息使用简洁 Conventional Commit，并立即推送当前分支；文档整理本身也只作为一个独立文档切片交付。

## 6. `v0.2.0` 退出条件

- B1-B4 的失败恢复和标注回归在当前代码上可重复，且没有孤儿窗口、进程、任务、按键或半成品文件；
- W1-W6 的截图工作区 surface、主工具栏、上下文样式栏、工具组/More 浮层和选区锚定在三种尺寸、双主题、双语及可用 DPI
  下通过布局与真实输入验收；没有新的固定颜色、固定总宽度、未登记 tooltip 或工作区硬编码文案；
- U1 完成后核心 UI 没有未登记的中英文硬编码；U2-U4 的非截图工作区三种尺寸、双主题和双语真实验收通过；
- M1 至少完成一个 runner 或 overlay 职责拆分，且必须建立在 W1-W6 行为稳定、报告和用户行为无回归之上；
- 已具备的 DPI 环境全部执行，未具备的矩阵仍显式标记为暂缓；
- CI、Release 构建、便携包/安装器、manifest、SHA-256 和下载复核通过；
- README、需求、架构、计划、Windows 验收、分发和 Linux 可行性文档之间无失效链接或相互矛盾的状态。

当前首个静态状态矩阵已完成，下一步继续 **W6 的双主题、双语和真实输入状态矩阵**；若当前 Windows 会话缺少所需 DPI 硬件，记录 D1 暂缓证据，
先完成当前 100% 单屏范围，不把环境缺失写成通过，也不越过 W6 扩展新功能。W6 通过后才进入 U1-U4 与 M1；
W1-W6 完成后才进入 M1。W0 只代表公开研究和路线文档已完成，W3/W5 的静态或单一场景 Release 证据也不替代 W6 的完整
真实输入验收。
