# 主线开发计划

更新日期：2026-09-01
当前版本：`0.1.2`
目标版本：`0.2.0` 质量阶段

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
| 暂缓项 | Deferred Scope | 已明确不纳入当前版本、等待硬件/服务/产品决策的范围 | 不是已完成或默认支持 |

## 1. 代码复审结论

本次复审基于 `main` 提交 `a6f053e`（2026-09-01）以及 workspace 中的五个 crate。复审范围包括 Cargo
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
- `v0.1.2` 已发布，发布资产、安装器、便携包、manifest 和版本事实保持一致；已有 Windows 单显示器
  100% 主流程、历史流控、Pin 生命周期和设置/覆盖层视觉探针证据。

复审没有确认新的静态 P0，但发现以下未完成项。它们决定后续顺序，不能用单元测试或旧报告代替：

| 优先级 | 未完成项 | 当前事实 | 完成所需证据 |
| --- | --- | --- | --- |
| P0 | 外部失败恢复 | 保存、历史索引和录屏进程已有确定性恢复测试；`copy-cancellation-race` 的隔离观察器已在当前 Release 会话覆盖工具栏和 Enter 两个入口，但系统剪贴板争用、只读目录和 FFmpeg 用户界面失败仍未完整执行 | 真实失败触发、可理解反馈、再次操作成功、无窗口/进程/任务/临时文件残留 |
| P1 | 标注原生回归 | Text、Watermark、Line 和双向 Arrow 已接入 `annotation-regression` 场景；当前 HEAD 仍缺少真实鼠标/键盘和导出证据 | 当前 Release、单屏 100%、同会话 JSON、步骤截图、逐像素导出和清理报告 |
| P1 | 动态文案盘点 | `Locale`/`UiText` 已覆盖大部分设置、Capture、Library、Record、Pin 和 workflow；剩余动态状态需要重新盘点，不能沿用旧的 317 条计数 | 中英文资源覆盖、参数化模板测试、无未登记用户可见硬编码 |
| P1 | UI 信息层级 | 设置壳层、覆盖层/Pin token 和 Library/Record 尺寸已有部分复核；Record、App、诊断和错误恢复入口仍需收敛 | 420x420、520x640、980x760；中英文、深浅主题；真实入口可达且无重叠 |
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
- Linux/macOS 功能对等、协作/云同步、插件 API、浏览器或移动端；
- 未经性能和原生验收的 GPUI/Zed 跟随升级。

### 2.3 执行原则

1. 一个切片只解决一个可观察结果，先写边界和失败行为，再修改代码。
2. 行为、视觉、结构重构、依赖升级和文档各自独立提交；不把未完成的证据写成通过。
3. 静态测试、确定性测试、Release 原生输入和像素/清理报告分别说明，不能互相替代。
4. 每个异步操作都绑定操作代次或等价的资源所有者检查；失败必须释放 busy、窗口、输入、任务、进程和临时文件。
5. 继续保持唯一 `flash-shot` 二进制、版本化设置和现有报告/脚本参数兼容。

## 3. 当前状态

| 编号 | 主线切片 | 状态 | 说明 |
| --- | --- | --- | --- |
| A | `v0.1.2` 发布基线 | 已完成 | 标签、资产、安装器、便携包、manifest 和发布说明已核对 |
| B1 | 外部失败恢复 | 部分完成 | 保存/历史/录屏进程的确定性恢复已完成；`copy-cancellation-race` fixture 已接入，真实剪贴板、目录权限和 FFmpeg UI 注入待补 |
| B2 | Capture/Save/Pin/Close 生命周期 | 已完成（单屏 100%） | 操作代次、拆除屏障、输入释放和下一次 Capture 已有 Release 证据 |
| B3 | 历史异步流控 | 已完成（单屏 100%） | 300 条队列、失败/重试、删除、目录切换和窗口关闭已有资源证据 |
| B4 | 标注回归保护 | 部分完成 | 当前 runner 已覆盖 Text、Watermark、Line 和双向 Arrow；真实输入仍待执行 |
| U0 | 视觉设计基线 | 已完成（单屏 100%） | 语义颜色、几何 token、双主题和三种窗口尺寸已有设置/覆盖层探针 |
| U1 | 动态文案国际化 | 部分完成 | 继续清点 workflow、错误、忙状态和动态数量；资源键保持语言无关 |
| U2 | 信息架构与动作收敛 | 待开始 | 重点是 App、Library、Record 的重复入口、主动作和恢复动作 |
| U3 | 视觉 token 与布局 | 部分完成 | 设置、覆盖层、Pin、Library/Record 已使用部分 token；真实输入和高 DPI 未覆盖 |
| U4 | Pin 中英文实时输入 | 部分完成 | 四组合无输入 runner 已通过；真实鼠标/键盘与打开窗口切换待执行 |
| M1 | 模块职责拆分 | 待开始 | B1-B4、U1-U4 行为证据稳定后，先拆 runner，再拆 overlay |
| D1 | 单显示器高 DPI | 暂缓 | 需要真实 150%/200% Windows 硬件；未执行环境不得推断通过 |

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
overlay/Pin/任务/按键清零；截图与路径已登记在 [Windows 手工验收记录](windows-manual-acceptance.md)。B1 仍保持
“部分完成”。快速保存确定性 fixture 已在提交 `a4e5081` 覆盖损坏帧失败、最终保留名和 `.tmp` 清理、使用同一
时间戳/UUID 的再次保存、首选目录失效后的全屏 PNG 回退，以及录屏 worker 启动失败后的运行标记释放；下一步只处理
系统剪贴板争用、真实只读目录和 FFmpeg 用户界面失败恢复。

**顺序**：确定性 fault fixture 已完成；接着执行可丢弃桌面上的真实输入、系统剪贴板和 FFmpeg 场景。
任何一类无法清理都保留失败报告并停止该切片。

### B4：执行当前 HEAD 标注回归

**目标**：把已存在的标注编辑与导出测试落实为当前 Release 的真实输入证据。

**边界**：固定单屏 100%、真实拖选、Text、Watermark、Line、正向/反向 Arrow、Quick Save 和清理；不新增工具，
不改变标注文档坐标、Undo/Redo 或导出规则。

**验收**：每个步骤的类型、内容、起终点与注入记录一致；第二个箭头不受第一个影响；导出 PNG 尺寸与选区物理尺寸
一致且像素发生预期变化；`.tmp`、窗口、任务和按键清理。报告、截图和导出文件必须来自同一 Release session。

### U1：完成动态文案迁移

**目标**：用户可见的动态状态全部由 `Locale`/`UiText` 提供，错误详情和路径作为参数保留。

**边界**：`workflow.rs`、`workflow/*.rs`、Record/OCR/更新/历史/Pin 状态和 UI 状态指示器；不修改业务状态机、
快捷键、报告字段或外部服务协议。

**验收**：English/简体中文的按钮、忙态、失败、重试、数量、耗时和路径均有资源键和参数化测试；静态盘点不再以旧
计数作为事实；缺少 OCR 或翻译配置不能阻塞截图主链。若文案变长造成溢出，转入 U2/U3 修复容器，不截断错误。

### U2：收敛入口和恢复动作

**目标**：Capture、Library、Record、App 各自只有一个最强主要动作，低频能力可找到且不与主流程竞争。

**边界**：GPUI 页面导航、设置快捷入口、Library 筛选/批量操作、Record 生命周期和诊断入口；不删除唯一可完成核心流程的入口。

**验收**：三种窗口尺寸均能完成截图、标注、Copy、Save、Pin、OCR 重试和 Record 取消/清理；主/次/破坏性/忙/错误/恢复
状态可区分；稳定语义 ID 不依赖显示文案；真实键鼠验收通过后才能关闭切片。

### U3/U4：完成视觉与 Pin 原生矩阵

**目标**：把已完成的 token 和隔离探针扩展到真实中英文输入与窗口生命周期。

**边界**：界面度量（ThemeMetrics）、按钮/工具栏命中区、焦点态、tooltip、Pin 复制/保存/关闭和语言/主题组合；不改变截图像素、
快捷键和报告 schema。

**验收**：English/简体中文、浅色/深色、420x420/520x640/980x760 均无截断、重叠或状态遮挡；Pin 真实点击、关闭和
再次 Capture 可恢复；已打开窗口的语言切换策略明确，若不能安全刷新必须提示重新打开。

### M1：按职责拆分大型模块

**前置**：B1-B4、U1-U4 的行为证据稳定，且当前报告 schema 有固定回归样本。

**顺序**：先把 `overlay-interaction-acceptance` 按 Capture/Copy/Pin/Scroll/Recording 和共享 Windows 输入设施拆分，
再把 `overlay.rs` 按渲染、输入、选择变换、菜单/工具状态和导出生命周期拆分。每次只迁移一个职责，保持 CLI 参数、
输出目录、报告字段、快捷键和用户行为不变。出现差异立即停止，不与行为修复同一提交。

### D1：真实单显示器高 DPI

在真实 150% 和 200% Windows 设备分别验证自由拖选、智能目标、键盘微调、标注、Copy/Save/Pin、滚动、录屏、
三种设置窗口尺寸和清理。报告必须记录实际 DPI/scale、显示器物理边界、导出像素和窗口/任务状态。双屏和翻译端点
仍是独立暂缓项，不能由 D1 推断通过。

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

验证失败、证据不足或只完成子集时，保持“部分完成/待执行”，不提交为完成状态。验证通过后只提交一个独立功能，
提交信息使用简洁 Conventional Commit，并立即推送当前分支；文档整理本身也只作为一个独立文档切片交付。

## 6. `v0.2.0` 退出条件

- B1-B4 的失败恢复和标注回归在当前代码上可重复，且没有孤儿窗口、进程、任务、按键或半成品文件；
- U1 完成后核心 UI 没有未登记的中英文硬编码；U2-U4 的三种尺寸、双主题和双语真实验收通过；
- M1 至少完成一个 runner 或 overlay 职责拆分，报告和用户行为无回归；
- 已具备的 DPI 环境全部执行，未具备的矩阵仍显式标记为暂缓；
- CI、Release 构建、便携包/安装器、manifest、SHA-256 和下载复核通过；
- README、需求、架构、计划、Windows 验收、分发和 Linux 可行性文档之间无失效链接或相互矛盾的状态。

下一步固定从 **B1 外部失败恢复** 开始；若当前 Windows 会话缺少所需权限、消费者或 FFmpeg，记录阻塞证据，
先完成可执行的确定性测试，不把环境缺失写成通过，也不越过 B1 扩展新功能。
