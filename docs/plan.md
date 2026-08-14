# 项目开发计划

每个编号项都是有明确边界的小功能。只有完成对应验证后才提交代码；不同功能不合并到同一次提交。

## 当前开发策略

当前阶段以截图软件的高频主流程为唯一优先级，不以功能数量作为完成度。已有 OCR 和翻译能力
继续保留为可选扩展，但不再扩充识别引擎、语言覆盖或在线服务，也不作为当前版本的交付阻塞项；
需要先把截图核心流程做到可靠、低延迟，再完善视觉一致性、信息层级、键鼠效率和符合人体工学的
操作顺序。具体优先级如下：

1. **核心可靠性**：优先保证快捷键触发、选区、捕获、编辑、复制、保存、取消和资源清理形成
   可重复的闭环，失败必须可恢复、可诊断，不能留下覆盖层、菜单或后台任务。
2. **性能**：保持物理像素坐标和最少像素复制，持续测量快捷键到覆盖层、捕获到编辑器以及导出
   的 Release 延迟、峰值内存和长会话稳定性；性能回归不能用新增功能抵消。
3. **视觉与人体工学**：主要动作保持首屏可见并具有稳定层级，窄窗口不溢出，键盘焦点顺序与
   鼠标操作顺序一致；减少重复确认、长距离指针移动和捕获目标被控件遮挡的情况。
4. **未来扩展**：OCR、翻译及其模型、服务和语言能力只在上述主流程通过静态门禁、真实 Windows
   输入和视觉验收后继续完善，并保持延迟加载、可关闭和不影响启动性能。

### 并行开发流水线

并行开发必须沿用同一条可审计流水线，代理数量不能替代边界和验收：

1. 先从本计划和产品边界拆出单一小功能，写明完成标准、风险、允许修改的文件和禁止扩大的范围。
2. 再按互不依赖的职责分配代理；优先让代理修改独立文件，必须共享文件时由一个代理拥有写权限，
   其他代理只提供审计结论，避免用事后冲突合并代替协作。
3. 每个代理先运行与改动直接对应的静态测试，再运行格式化、Clippy、编译和受影响范围的回归测试；
   静态通过只证明代码路径可构建，不证明原生交互已经可用。
4. 对用户可见流程使用 Release 程序和真实 Windows 键鼠输入验收，不以直接调用内部状态、测试专用
   快捷路径或进程成功启动代替操作链路。
5. 保存关键步骤截图，并对捕获像素、控件边界、文案截断、焦点/主次动作布局和残留窗口进行比对；
   结构化报告必须记录实际输入、状态迁移、输出产物和清理结果。
6. 只有静态门禁和该功能所需的真实验收都通过后，才以一个小功能一次提交的方式提交，并立即推送
   当前分支；未通过、不可测试或只完成一部分的改动不得标记完成或混入其他提交。

## 里程碑 0：工程基础

- [x] 创建 `bruceblink/flash-shot` Rust + GPUI 可运行应用壳。
- [x] 编写产品需求、架构边界和性能预算。
- [x] 增加格式化、编译、Clippy、测试和 CI 门禁。
- [x] 锁定当前官方 GPUI 源码提交，不引入第三方组件框架。
- [x] 增加结构化日志、panic 报告和应用目录。
- [x] 增加性能基准工具和机器可读报告。
- [x] 实现单实例和可控退出。

退出标准：应用壳能够稳定启动，CI 全部通过，失败可诊断，并且在实现截图前已经可以记录性能基线。

## 里程碑 1：Windows 截图技术验证

- [x] 枚举显示器物理边界、缩放、旋转和颜色信息。
- [x] 获取每个显示器的不可变截图帧。
- [x] 为每个显示器打开无边框覆盖窗口并正确转换物理/逻辑坐标。
- [x] 测量快捷键到覆盖层的延迟以及截图纹理上传行为。
- [x] 验证跨负坐标和正坐标显示器的混合 DPI 选区。

退出标准：技术报告证明 GPUI 渲染路径可行，并列出所有像素复制。如果 GPUI 公共 API 无法达到目标，必须先决定采用小范围 GPUI 扩展还是独立覆盖层渲染器，再开发产品界面。

## 里程碑 2：截图 MVP

- [x] 全局快捷键、托盘和截图会话生命周期。
- [x] 区域选择、缩放控制点、键盘微调、放大镜和尺寸显示。
- [x] 通过 Windows UI Automation 识别窗口和控件。
- [x] 复制 PNG、保存 PNG、取消和确定性资源清理。
- [x] 连续截图压力测试和延迟指标。

当前实现状态：核心截图到复制/保存流程、压力工具和对应自动化测试已完成。Release 默认环境的 20 次采样已达到启动 p95 233.21 ms、frame-ready p95 56.14 ms、覆盖层 p95 80.02 ms，均低于当前门槛；双屏混合 DPI 硬件采样已按 2026-08-10 用户范围暂缓，恢复多屏范围后再执行。

### 截图主流程打磨计划

Snow Shot 作为产品工作流参考，Flash Shot 保持原生 Rust/GPUI 实现，不迁移其
Tauri/WebView/Excalidraw 架构。后续优先级只围绕快捷键到截图导出的高频链路：

1. **选区精度与反馈**：在选区旁显示物理像素尺寸；支持拖动、移动、四角缩放、
   Shift 保持比例，以及 Alt 从中心缩放；确保反馈不会遮挡导出操作条。
2. **操作条信息层级**：将复制、保存、取消保持为首屏稳定动作；把低频的 OCR、
   滚动截图、项目文件和颜色相关操作收进次级菜单，并在窄屏时保持可达。
3. **智能选区与键盘流**：验证窗口/控件候选框、单击采用与自由拖动的优先级；
   保持 Enter 复制、Ctrl+S 标准保存、Shift+Enter 快速保存、Escape 取消和方向键物理像素微调的一致性。
4. **真实环境验收**：当前在单屏环境已验证拖动、选区移动、普通/Shift/Alt 角点缩放、导出和取消；混合 DPI 多屏环境已暂缓，恢复范围后执行；
   用 Release 压力脚本收集快捷键到覆盖层的 p95，定位并消除超过 100 ms 预算的开销。

边界：取色、OCR、翻译和录屏继续保留，但在上述主链路通过真实交互与性能验收前，
不再作为当前迭代重点。

### 已完成主链路：普通选区复制响应

这一项只打磨用户最常重复的“选中后复制”动作。用户已经完成取景和选区时，工具不应再要求
寻找菜单、等待不确定的后台状态，或把复制结果留在只能被应用内部观察器读取的测试路径中。
目标是让一次普通选区的 Copy 成为可预测的单步出口：动作完成后，用户可以立刻粘贴到任意常见
Windows 应用，而不是只看到应用自身的成功提示。

#### 快捷键与焦点契约

1. `Ctrl+S` 是与首屏 `Save` 按钮相同的标准保存：进入既有的原生保存对话框，由用户决定位置和文件名；
   它不得静默改写为快速保存，也不得绕过现有的取消与错误恢复路径。
2. `Shift+Enter` 保持快速保存：沿用已有的快速保存位置和命名策略，不弹出原生保存对话框。它服务于
   连续截图中的低摩擦导出，不能取代需要明确文件位置的标准保存。
3. `Tab` 与 `Shift+Tab` 继续保留给标注对象的前后导航，不能为了给操作条补传统焦点顺序而改变其语义；
   这能让密集标注时的键盘选择保持连续，避免焦点在工具栏和画布之间意外跳转。
4. 以上快捷键只在覆盖层可接受命令且没有文字编辑控件接管输入时生效。编辑文字时，输入控件优先，
   避免全局快捷键中断用户输入。

#### 性能目标、范围与非目标

- **目标**：在 Release 构建、真实 Windows 输入和普通静态选区条件下，从触发 Copy 到独立的系统
  剪贴板消费者能够读取并解码图像的端到端 p95 不高于 `250 ms`。
- **范围**：覆盖普通选区的首屏 Copy 动作及 `Enter` 快捷键的同一路径；输出必须写入生产
  `SystemClipboard`，并保持 PNG、CF_DIB 与常规消费者读取的像素正确性。
- **非目标**：本项不以滚动截图、录屏、保存对话框耗时、复杂标注合成或后台上传的性能替代普通 Copy
  指标；这些流程各自维持独立的验收和预算。
- **明确不扩展 OCR**：不新增识别引擎、模型、语言、联网服务或 OCR 入口，也不让 OCR 初始化进入
  Copy 的计时区间。OCR 与翻译仍是主链路稳定后才处理的可选扩展。

#### 验收证据

1. 用 Release 二进制执行至少 30 次同机采样，并先完成固定数量的预热；每次都经真实 Windows 键鼠
   触发普通选区和 Copy，不得直接调用内部复制函数或使用进程内模拟剪贴板替代生产路径。
2. 每次采样记录从 Copy 输入发出到外部消费者成功读取的完整耗时，并输出样本列表、p50、p95、失败数、
   选区尺寸、显示器 DPI、构建信息和时间边界定义。只有所有有效样本的 p95 小于或等于 `250 ms` 才通过。
3. 外部消费者必须分别读取 PNG、CF_DIB 和常规图像格式，解码后与导出源逐像素比较尺寸与指纹；
   仅确认剪贴板格式已注册、或只读取应用内观察器，均不能证明真实可粘贴性。
4. 保存选区完成、Copy 触发和结束清理的关键截图，以及关联的结构化报告、消费者读取结果和像素比对结果。
   Copy 成功后必须保持原选区和唯一覆盖层可继续编辑，随后由 runner 显式发送 Escape；证据还必须证明
   覆盖层、菜单、临时窗口和后台任务在 Escape 后没有残留，且控件没有遮挡选区或发生截断。

当前性能基础设施状态：`copy-performance` 提供显式授权的 Release **合成**系统剪贴板基准，默认执行
30 次 `1280x720` 生成帧的合成、裁切、生产剪贴板写入和同进程读回。报告 schema 2 会保存完整样本、失败
样本和 p50/p95，并明确标记 `measurement_mode=synthetic`、`real_ui=false`、未独立验证 PNG/CF_DIB
或外部消费者；有效样本少于 30、存在失败或 p95 超过 `250 ms` 均不会通过。该基准用于稳定的 CPU/剪贴板
回归门禁，不是本节要求的真实 UI 证据。

真实 UI 的 30 次普通 Copy 由 Release `overlay-copy-batch` 批量采样完成。该 wrapper 为每个预热或
样本启动一个隔离的 `overlay-interaction-acceptance --allow-system-clipboard` 子进程，读取其独立
PNG/CF_DIB/常规消费者逐像素结果、cleanup 状态和 QPC 时间边界，再聚合样本列表、p50、p95 和失败数。
遇到超时、报告缺失、像素不等或 cleanup 无法证明时会停止后续全局输入；报告标记
`measurement_mode=real_ui`、`real_ui=true`，并记录显示器 DPI、构建路径/配置和 UTC Unix 时间边界。
已保存的一次 Release toolbar batch 证据位于：`target/overlay-copy-batch/release-toolbar-20260812/batch-report.json`
记录 1 次预热 + 30/30 有效样本，失败数为 0，p50 `79.0749 ms`、p95 `106.1236 ms`，显示器为
单屏 DPI 96/scale 1.0，`windows_qpc` 的 `button_down_batch_to_consumer_decoded_image` 边界，PNG、CF_DIB
和常规消费者逐像素校验均通过，所有子报告 cleanup 安全。该报告对应当时的 Release 二进制；源码或二进制
变更后必须重新生成当前证据。

2026-08-15 在提交 `d30d9c3` 的源码上重建 Release runner，并保存该阶段的 toolbar 证据到
`target/overlay-copy-batch/production-toolbar-github-final-current-20260815-002201/session-1786724521963-26900/batch-report.json`。
该会话完成 1 次预热 + 30/30 有效样本，失败数为 0，p50 `49.711 ms`、p95 `51.9642 ms`，最慢样本
`52.4163 ms`；runner SHA-256 为 `c19baef67d72aadcc1b5c0d2b42b8b46ce1ea960151bd3c896d6d11b5dc9af65`。
31 条迭代记录全部同时通过生产系统剪贴板、PNG/CF_DIB/常规消费者逐像素和 cleanup 断言，显示器仍为
单屏 DPI 96/scale 1.0。该记录作为后续剪贴板争用修复前的性能基线保留。

随后在包含剪贴板争用修复和默认图片命名更新的当前源码上，用同一个 Release runner 分别重跑 toolbar 和 Enter。
toolbar 报告位于
`target/overlay-copy-batch/current-source-toolbar-naming-20260815/session-1786734049398-20680/batch-report.json`，
完成 1 次预热 + 30/30 有效样本，失败数为 0，p50 `51.077 ms`、p95 `52.3159 ms`、最大值
`52.4555 ms`。Enter 报告位于
`target/overlay-copy-batch/current-source-enter-naming-20260815/session-1786734503147-32644/batch-report.json`，
同样完成 1 次预热 + 30/30 有效样本，失败数为 0，p50 `26.8764 ms`、p95 `28.5772 ms`、最大值
`29.6926 ms`。两份报告的 runner SHA-256 均为
`a702ba1c06add16f125e8246908aad97add30367d86e7ecabe193c32564442ce`，与磁盘 Release 二进制一致；
62 条迭代全部通过生产系统剪贴板、PNG/CF_DIB/常规消费者逐像素、编辑器保留、显式 Escape 和 cleanup
断言，显示器为单屏 DPI 96/scale 1.0。修复前一次 toolbar 会话在 26 个有效样本后因前台抢占安全停止，
另一次会话暴露应用 Copy 失败未及时传给消费者与外层超时过早的问题；修复后的第一次 Enter 会话仍因
第二次拖选前的外部前台抢占安全停止。以上失败均保留为 fail-closed 证据，没有合并进最终零失败样本。
当前源码的 toolbar/Enter 真实 UI 30 次门禁已完成；任何 `copy-performance` 结果仍不能替代这两份真实
键鼠端到端 p50/p95。

2026-08-12 的 Release UI 验收又通过真实输入执行了 `Ctrl+S -> Save 对话框 -> Escape -> 原选区恢复 ->
Ctrl+S -> 保存`。报告位于
`target/overlay-interaction-acceptance/ctrl-s-release-verified-retry/session-1786504134708-26760/report.json`；
取消前后两张选区截图的 SHA-256 均为 `FEF5BD6B8D0884473502E29FC392A351A87ECD2B7F755122A9542A04B09FC185`，
保存结果为 `1178x432`，源与解码结果像素一致，结束时 overlay、Pin 和可见进程窗口均为零。该 UI 证据
补齐了标准保存快捷键、取消恢复和视觉清理，但普通 Copy 的系统剪贴板端到端时延仍以当前源码的
`overlay-copy-batch` toolbar/Enter 各 30 次 Release 报告为准，不能互相替代。

## 里程碑 3：原生标注

- [x] 带版本的 `AnnotationDocument` 和命令历史。
- [x] 选择、矩形、椭圆、直线、箭头和自由画笔。
- [x] 支持 IME 和中英文混排验证的文字编辑。
- [x] 模糊/马赛克、高亮、水印和序号。
- [x] 图层、样式、命中测试、四角缩放控制点、90 度旋转、撤销和重做。
- [x] 带 golden image 测试的像素正确 CPU 合成。

当前实现状态：标注工具与 CPU 导出合成已经由模型、命令历史和像素指纹测试覆盖。GPU 仅用于界面预览，不承担导出合成；4K 交互帧时间仍需在目标硬件上做真实验收。

## 里程碑 4：效率工作流

- [x] 贴图窗口。
- [x] 本地截图历史及保留策略。
- [x] 打开 PNG 并进入已有的标注、复制、保存和贴图工作流。
- [x] 对当前截图选区进行本地二维码识别。
- [x] 延迟调用本地 Tesseract OCR，临时截图在任务结束后删除。
- [x] 可选 HTTPS 翻译服务边界；未配置端点时不产生网络请求。

退出标准：可选模型和历史记录不影响启动速度、空闲资源和隐私。

## 里程碑 5：滚动截图

- [x] 从 Snow Shot 提取重叠匹配和合成思路，不迁移 Tauri 类型。
- [x] 实现手动滚动捕获、重叠检测、错位恢复、预览和导出。
- [x] 在平台行为可靠时提供辅助滚动。
- [x] 增加长页面内存与正确性测试。
- [x] 用 Release 真实输入完成滚动结果的系统剪贴板复制与保存 PNG 双出口验收。

### 滚动截图 roundtrip 完成标准

滚动截图不能仅凭重叠算法、状态机或窗口启动测试判定完成，必须在 Release 程序中满足以下闭环：

1. 通过真实 Windows 键鼠输入依次打开 `More`、选择 `Scroll shot`、采集至少第二帧、执行 `Finish`，
   并确认拼接结果进入生产编辑器；验收不得直接调用滚动状态或编辑器打开函数跳过界面。
2. 从编辑器分别完成复制和保存出口；如果一个出口会关闭编辑器，则运行两次完整 roundtrip。剪贴板
   PNG 与保存 PNG 都必须可解码，尺寸和像素指纹与编辑器中的拼接结果一致。
3. 拼接图像不得包含 `More` 菜单、滚动控制条、验收窗口或其他 Flash Shot 控件像素；关键截图还要
   证明控制条没有遮挡捕获区域，按钮、状态文案和主要动作在目标窗口尺寸内没有溢出或重叠。
4. 每次运行生成结构化报告，至少记录真实输入端点/前台窗口、选区、帧数、滚动会话状态、拼接尺寸、
   编辑器内容、剪贴板/保存产物校验，以及结束后的菜单、控制条、覆盖层和后台任务残留数量。
5. 保存 `More` 菜单、第二帧就绪、`Finish` 后编辑器和复制/保存结果等关键步骤截图，并将报告与截图
   作为同一次验收的关联产物；报告、像素比对和截图三者一致后，roundtrip 才可标记完成。

### 双出口验收实现思路

1. Copy 成功后保留拼接编辑器，runner 会先验证原选区仍可编辑，再显式发送 Escape；Save 成功后关闭
   编辑器。`--scroll-export copy|save` 仍启动两个独立进程，每个进程都重新执行完整真实输入链路，
   不能复用第一次会话的拼接状态。
2. 普通验收默认仍把 Copy 路由到进程内观察器；只有标准场景显式提供
   `--allow-system-clipboard`，或滚动场景同时提供 `--scroll-export copy` 与该授权时，应用才使用
   生产 `SystemClipboard`，避免普通测试意外覆盖用户剪贴板。生产路径会在点击前启动一个加入
   Job Object 的无窗口独立消费者，通过 ready/start/result 文件协议同步，并在报告中记录 PNG、CF_DIB、
   常规图像读回、QPC 单调延迟和有界清理结果。
3. 点击出口前先固定编辑器中的拼接源帧。Copy 同时读取注册 PNG、CF_DIB 和常规消费者图像，分别
   解码并逐像素比对；Save 必须经过唯一归属的原生文件对话框，再从隔离路径重新打开 PNG 比对。
4. 两次会话还要得到相同的初始帧、第二帧和拼接源指纹，并在结束时证明滚动帧、后台捕获、菜单、
   标注面板、控制窗口、覆盖层和 Pin 全部归零。截图复核属于门禁的一部分，不能由 JSON 代替。
5. 本轮截图复核发现控制条动作虽未溢出却被省略号截断，随后独立修正按钮宽度与短文案；最终
   `Scroll down + capture`、`Capture view`、`Finish`、`Cancel` 在 520px 控制条内完整显示。

当前 Release 双出口已通过。Copy 报告位于
`target/overlay-interaction-acceptance/scroll-copy-release-ui-final/session-1786492775987-21556/report.json`，
Save 报告位于
`target/overlay-interaction-acceptance/scroll-save-release-ui-final/session-1786492902079-27312/report.json`。
两次均从 1484x380 拼接到 1484x663，源指纹均为 `f21a929cfd054de1`；系统 PNG、CF_DIB、消费者
读取和保存 PNG 均逐像素一致，结束后所有滚动状态与可见探针窗口为零。空标注整帧导出的冗余
合成与裁切已经在后续独立性能切片中消除；PNG 的整帧 RGBA 与压缩输出缓冲仍保留为下一项工作。

2026-08-12 又以标准单屏场景完成一次真实生产剪贴板 Copy：
`target/overlay-interaction-acceptance/standard-system-clipboard-final-2/session-1786507020308-21492/report.json`。
该历史报告为 schema 12，序号 `353 -> 358`，PNG、CF_DIB 和常规消费者三路逐像素一致，消费者在点击前
ready、已回收，最终没有可见窗口。该证据当时只覆盖一次工具栏点击；后续 toolbar/Enter 各 30 次的
当前源码证据已在本节前述 2026-08-15 记录中完成，仍不能由 `copy-performance` 的内部合成基准替代。

标准 Copy runner 现支持 `--copy-trigger toolbar|enter`；两种触发均共享同一个生产导出状态断言、
独立消费者和像素 oracle。`toolbar` 真实 Release 证据已归档；`enter` 的单次真实 Release 证据已归档于
`target/overlay-interaction-acceptance/standard-system-clipboard-enter-final/session-1786512571193-22568/report.json`：
序号 `368 -> 373`、Windows QPC 端到端延迟 `84.1912 ms`，三路内容逐像素一致并完成消费者清理。
该记录当时只关闭单次键盘出口验收；后续 Enter 30 次真实 UI 样本、p50/p95 和零失败门禁已在本节
前述 2026-08-15 记录中完成，不能由 `copy-performance` 的内部合成基准替代。

### Save、Pin 与再次 Capture 的 teardown 屏障

生产关闭覆盖层使用 GPUI deferred callback，不能只用“窗口句柄不可见”判定桌面已经干净。应用现在以
`OverlayInteractionCaptureState.capture_teardown_pending` 暴露仍在等待 native close callback 的窗口批次，
并在最后一个窗口回调后清零；Capture preflight、Pin 和验收 runner 都以该状态作为硬门禁。真实 runner 在
Save 完成后还等待 `capture_preflight_ready`、没有可见对话框/覆盖层/Pin，并连续采样稳定桌面，保存
`save-complete-clean.png` 后才开始下一次 Capture 或 Pin。该屏障解决了 Save As 关闭后 DWM/GPUI 残影进入
下一张截图的问题；它不扩大当前单屏 100% 验收范围。

当前 schema 13 Release 证据位于
`target/overlay-interaction-acceptance/save-pin-quiescence-release-9/session-1786529894490-5828/report.json`；
旧的 schema 12 Copy/Enter 报告仍作为历史证据保留。

后台 Copy 的新报告契约使用 schema 14：`editor_retained_after_copy=true` 才证明 Copy 完成时会话仍为
`selecting`、原选区不变且只有一个覆盖层；`cleanup_after_escape=true` 才证明后续显式 Escape 将会话、
覆盖层和 teardown 屏障全部清理。旧 schema 12/13 证据不具备这两个字段，不能替代当前源码的重跑。

2026-08-14 的当前源码 Release 回归位于
`target/release-acceptance/copy-retained-current-20260814-221237/session-1786716757097-24128/report.json`。
该会话在单屏 `2560x1440`、DPI 96 下通过真实工具栏 Copy，固定选区为 `1178x432`，源与结果指纹均为
`acf69e150b9e2f90`。报告的 `copy_editor_retained` 截图和结构化断言确认 Copy 完成后编辑器仍可操作，
随后 `copy_escape_cleanup` 确认会话回到 idle、overlay/Pin/可见进程窗口均为零且 capture preflight 恢复。
本次仅使用进程内 `isolated_observer`，系统剪贴板序号保持 `36 -> 36`；它证明当前后台 Copy 的 UI 保留与
清理契约，不替代需要 `--allow-system-clipboard` 的 toolbar/Enter 各 30 次外部消费者性能批次。

### Pin 帧准备的 UI 线程边界

Pin 的大像素工作现在统一在 `background_executor` 完成。选区 Pin 在后台执行标注合成、裁切和
`render_image_from_capture`；剪贴板 Pin 在后台读取并准备图像；全屏 Pin 在后台捕获并准备图像；历史
Pin 在后台解码 PNG 并准备图像。UI 线程只接收 `PreparedPinnedFrame`，创建原生 Pin 窗口并注册生命周期，
不会再因为 4K 帧的整图渲染而同步阻塞。`operation_generation`、`claim_idle_completion` 和选区的
teardown 等待仍保留，晚到任务只能被丢弃，不能覆盖更新的状态。

该切片的代码门禁已完成：`cargo fmt --all -- --check`、`cargo check --locked --all-targets`、严格
Clippy、全库 `cargo test --locked --lib --no-fail-fast`（448 tests）和 Release 构建均通过。真实桌面
验收仍必须在可控前台桌面重跑；本轮两次尝试在输入注入前被 `ShellExperienceHost` 抢占前台而安全失败，
报告分别位于 `target/release-acceptance/current-core-pin-20260813/` 和
`target/release-acceptance/current-core-pin-20260813-retry/`。这两份报告只证明 runner 能正确拒绝
不安全的输入环境，不构成 Pin 截图或端到端延迟的通过证据。

### 长图导出准备性能优化思路

1. 先用独立的 Release `export-stress` 固定测量 `composite_annotations(...).crop(frame.bounds)`，
   默认样本为 1440x6000、30 次计时和 2 次预热。报告同时记录 p50/p95、额外 CPU 复制、复制
   流量估算、`Arc` 是否复用、像素恒等和稳定指纹；它不包含 PNG 编码、文件同步或剪贴板写入。
2. `CaptureFrame` 的像素由不可变 `Arc<[u8]>` 持有，因此空 `AnnotationDocument` 可以安全返回
   frame clone。整帧 crop 仅在交集等于源 bounds、宽高一致且 stride 已紧密排列时复用像素；带
   padding 的帧仍执行原有裁切，以保留“裁切结果紧密排列”的契约。
3. 同机 30 次 Release 对比中，1440x6000 源图为 34,560,000 bytes。优化前 p95 为
   `60.0825 ms`、额外复制 `2` 次、复制流量估算 `69,120,000` bytes、`Arc` 不复用；优化后
   p95 为 `0.0001 ms`、额外复制和复制流量均为 `0`、`Arc` 复用。两次像素指纹均为
   `15936792589717756389`，像素恒等门禁通过。报告位于
   `target/export-prep-baseline-30.json` 与 `target/export-prep-optimized-30.json`。
4. 非空标注仍必须真实合成。4K、4 个标注的 30 次 Release 压力样本 p95 为 `39.9048 ms`，
   指纹为 `10401088384397431893`；局部裁切、负坐标和带 padding stride 继续由像素测试覆盖。
5. 优化后的 Copy/Save 分别重新执行完整滚动截图 roundtrip；PNG、CF_DIB、普通剪贴板消费者和
   保存 PNG 仍与拼接源 `f21a929cfd054de1` 逐像素一致。优化前后的初始帧、第二帧、控制条和
   拼接编辑器四张证据 PNG 的 SHA-256 分别完全相同，证明性能快路径没有改变 UI 或导出内容。

### 历史缩略图解码流控

展开 Library 时，缩略图只是辅助信息，不应抢占截图、复制或保存所需的后台资源。当前实现以
FIFO 队列收集可见记录，最多同时解码两个 PNG；同一路径在等待或执行中不会重复排队。每个任务
完成后才补充下一个，历史删除或切换目录会同步丢弃不再保留的待处理路径，异步完成结果也会再次
确认路径仍属于当前历史后才写入缓存。该切片不改变缩略图尺寸、历史排序或默认仅显示五条的 UI
策略，目标是让 300 条历史展开时的 CPU、磁盘和工作集增长保持有界。

#### 验收标准、风险和当前状态

- [x] 行为门禁：固定 300 条请求仍保持 FIFO，最多 2 个解码中任务，重复渲染不重复排队，删除/换目录
      后的待处理路径不会再启动。
- [x] 回归门禁：`cargo test --locked --lib app::workflow::images::tests` 覆盖去重、上限、FIFO、删除清理；
      `cargo test --locked --lib --no-fail-fast` 覆盖全库。
- [x] Release 资源样本：同一单屏 Release 会话已记录默认 5 条预览和显式展开 300 条的首批缩略图时间、
      峰值私有字节、DPI、截图和结构化报告。Release 报告位于
      `target/history-resource-acceptance/release-gate-clean-20260813/report.json`：默认阶段为
      `5/0/0`（cached/loading/pending），展开阶段为 `300/0/0`，峰值 loading 为 `2`、pending 为 `293`，
      首批缩略图耗时 `258.2723 ms`，私有提交增长 `88,408,064` bytes，DPI 为 `96`。报告的
      `cleanup.fixture_files_removed=true` 且 `history_root_exists=false`；对应截图为
      `default-5-preview.png` 和 `expanded-300-preview.png`。

风险是磁盘较慢或 PNG 损坏会让队列长时间处于等待；失败项会记入失败集合而不阻塞后续任务。非目标是
改变默认预览条数、缩略图尺寸/排序、历史保留上限，以及 OCR、翻译或云同步。当前代码、静态回归和
Release 资源样本已完成；单屏以外的 DPI/多屏矩阵仍按本计划后续执行。

### PNG 流式编码设计与验收

#### 问题与边界

`CaptureFrame` 来自 Windows 时通常是 BGRA 字节序，而 PNG 导出契约是 8-bit RGBA。旧的实现如果先把
整张图转换成一个新的 RGBA `Vec`，再一次性交给 PNG 编码器，长图会同时保留源 BGRA、完整的已转换
RGBA 和最终 PNG 字节。这里的“流式”首先解决中间 RGBA 整帧副本，而不是把一切都变成零内存：当前
`encode_png` 仍需返回最终 PNG `Vec` 以供 Copy 共用，因此 Copy 的最终编码结果仍会驻留在内存中；文件 Save
走独立的流式写入路径，不再为了落盘保留第二份完整 PNG `Vec`。

本切片的目标是只复用一行 RGBA 缓冲完成颜色转换，并让 PNG 文件 Save 直接流入原子临时文件，保持现有
Copy、Save、滚动截图和像素契约不变。它不改变截图采集、标注合成、PNG 色彩管理或压缩参数；
`encode_png` 仍为需要内存结果的 Copy 路径保留，文件 Save 则不再构造完整编码 `Vec`。OCR、翻译、模型加载和在线服务继续是未来可选扩展，不参与
当前导出主链路的性能目标或验收门禁。

#### 逐行转换方案

1. 先执行 `CaptureFrame` 的尺寸、stride、像素长度和像素格式校验，并用受检的 `width * 4` 计算一行
   RGBA 所需字节数。这样异常帧和整数溢出会在编码前以错误返回，而不会产生越界切片。
2. 创建 `png::Encoder`，固定输出为 `Rgba` / `Eight`，写入 PNG 头后取得 `stream_writer`。它允许编码器
   在接收数据时逐步压缩，而调用方不必准备完整 RGBA 图像。
3. 只分配一次 `rgba_row`。对每个长度为 `stride` 的源行，只读取前 `width * 4` 个有效像素，按四字节
   像素将 `B, G, R, A` 写成 `R, G, B, A`，然后立即通过 `write_all` 交给 stream writer。若 stride
   大于有效行宽，尾部 padding 必须跳过，绝不能进入 PNG。
4. 所有行写完后先完成 stream writer，再完成 PNG writer，确保压缩流和 PNG 结尾块都被写出；任一步的
   错误必须继续返回给原有 Copy/Save 调用方。PNG 文件的压缩块边界可以变化，因此验收比较解码后的
   像素，不要求新旧 PNG 二进制逐字节相同。

#### 正确性与性能验收

1. 添加或保留图像单元测试：普通 BGRA 样本的通道顺序、至少两行且带 padding stride 的样本、以及
   流式输出与原编码路径解码后逐像素一致。异常格式、短像素缓冲和非法 stride 仍应保持既有的失败
   契约。
2. 运行与此切片对应的 `cargo test --lib image::tests::png`，再运行格式检查、全目标测试和严格 Clippy。
   这些门禁证明转换、错误传播和调用范围可构建；它们不替代真实 Windows 导出。
3. 使用独立的 Release `png-stress` 在相同输入、预热次数和迭代次数下记录编码延迟分位数、源图字节数、
   最终 PNG 大小和解码像素恒等。只有同机、同输入、同配置的编码基线才能与流式版本比较；
   `export-stress` 只覆盖合成与裁切准备，不能用来证明 PNG 编码变快。文件 Save 还应检查临时文件
   在成功替换后消失，并在编码/同步失败时不留下半成品。
4. 用 Release `overlay-interaction-acceptance --capture-scenario scroll-roundtrip` 分别执行 Copy 和 Save
   两个独立会话。每个会话都走真实 `More -> Scroll shot ->` 采集第二帧 `-> Finish -> Copy/Save` 链路，
   因为任一导出动作可能关闭编辑器，不能复用第一次会话的内存结果。
5. Copy 要解码注册 PNG、CF_DIB 和普通消费者读到的图像；Save 要从隔离路径重新打开 PNG。两条路径均与
   导出前的拼接源逐像素比较，并记录尺寸、指纹、会话清理状态和输出路径。保存 More 菜单、第二帧就绪、
   拼接编辑器以及 Copy/Save 结果的截图，复核工具栏没有遮挡捕获区域、文字未截断、菜单没有混入导出图像。
   结构化报告、解码像素比较和截图必须相互对应，不能以其中任一项单独宣告通过。

本轮当前源码的单次 Enter 系统剪贴板闭环也已重新执行：报告位于
`target/overlay-interaction-acceptance/current-enter-streaming-retry2/session-1786536827730-20476/report.json`
（schema 13）。触发器为 `enter_key`，PNG、CF_DIB 和常规消费者均逐像素匹配，指纹为
`c97b8733199d4db9`，QPC 端到端耗时 `82.7588 ms`，消费者在点击前已 observing 且已回收；
最终 teardown、overlay、Pin 和可见进程窗口均清零。该单次报告不替代 30 次批量门禁。

#### 已知测量边界

流式转换从结构上移除了完整中间 RGBA 副本，但 Windows 的 Working Set、采样频率和分配器缓存可能看不出
短暂分配，不能仅凭一次进程峰值就声明具体内存节省百分比。若需要内存门禁，应另行使用可重复的进程
私有字节或分配器统计，并明确测量区间是否包含 PNG 输出 `Vec`、文件同步和剪贴板写入。PNG 编码基准也
不代表 Save 的磁盘延迟或系统文件对话框耗时；这些属于单独的端到端测量和真实交互验收范围。

## 里程碑 6：录屏

- [x] 隔离的 FFmpeg 发现和能力探测。
- [x] 显示器、窗口和区域录制。
- [x] 麦克风和受支持的系统声音选择。
- [x] 暂停、恢复、正常结束、进度和失败恢复。
- [x] Job Object/进程组清理和孤儿进程测试。

当前实现状态：录制后端已具备 FFmpeg 探测、显示器/窗口/区域目标、可选单一音频源、暂停/恢复、进度、正常结束和 Windows Job Object 清理。本机用户级 FFmpeg 已配置并支持 `gdigrab`；生产录屏后端的显示器、窗口、区域以及暂停/恢复/停止流程已通过 `recording-acceptance` 探针生成并由 `ffprobe` 验证的 MP4。单屏 100% 环境下，显示器、窗口与区域的真实应用内录屏入口均已完成开始、暂停、恢复、停止、保存与 H.264 MP4 校验；窗口录制另以独立原生 fixture 验证源边界在目标移动、缩放、遮挡和最小化时保持固定，并从 MP4 时间线验证对应桌面合成像素。Record 页可持久化选择、恢复默认、检查和打开 MP4 目录，录屏启动请求会使用该目录并保留环境变量最高优先级；同毫秒生成的 MP4 名称会追加编号，FFmpeg 也被明确要求不覆盖已有文件。150%/200% 与已暂缓的多屏矩阵仍按 [Windows 手工验收记录](windows-manual-acceptance.md) 保持待执行。

## 里程碑 7：分发与其他平台

- [x] Windows 便携版构建与校验。
- [x] Windows 安装包构建与签名校验脚本。
- [x] 发布资产清单与 SHA-256 验证。
- [x] 更新发布策略与本地预发布验收（便携包、隔离 profile 启动冒烟、manifest、SHA-256、fixture 和安装器配置门禁均已通过）。
- [ ] 发布时真实验收（受信签名、真实安装/卸载和干净 Windows 用户账户仍需在发布环境执行）。

当前发布预检会在 `-ValidateOnly -RequireSignature` 下同时要求可执行的 SignTool 和当前用户证书库中
带私钥、未过期且含代码签名用途的证书；实际签名固定使用该次预检选中的证书。2026-08-13 本机已
验证版本化 Windows SDK SignTool、用户级 Inno Setup 路径、RFC 3161 时间戳参数和签名预检回归；短期
自签名证书只用于脚本闭环并已清理。2026-08-14 又将官方简体中文消息固定到 Inno Setup 源提交
`3cfb0e5` 和 SHA-256 `e0b0b350...b3fc9a8d`，获取器只从该 Git blob 导出并在交给打包器前验证
`LanguageID=$0804`。隔离的当前源码 Release 构建随后由 Inno Setup 6.7.3 生成未签名安装器
`target/installer-acceptance-20260814/dist/FlashShot-0.1.0-windows-setup.exe`，大小 `6,593,265` bytes，
SHA-256 `48515393bc660d8b0442f87301ab494517e7531a66dd2656c4c61b6387a79772`，sidecar 匹配且
Authenticode 状态明确为 `NotSigned`。这关闭了本机语言资源和安装器编译阻塞，但生产信任证书、实际
安装/卸载和干净 Windows 用户账户仍必须在发布环境执行，不将发布时真实验收标记为完成。

2026-08-15 新增 `smoke-installer.ps1`，以 Inno 明确的 current-user 命令行覆盖安装到唯一临时目录，
真实验证安装文件、Start 菜单快捷方式、卸载注册、隔离 profile 启动和卸载后零残留。本机当前源码
安装器 `target/installer-smoke-current-20260815/FlashShot-0.1.0-windows-setup.exe` 已完成该生命周期，
SHA-256 为 `c04504be...f8fc87b`，但仍明确为 `NotSigned`。GitHub Release 工作流现在不再提供未签名
回退：它要求生产 PFX secrets，在导入证书前完成 Release 构建，随后签名 EXE/安装器、从同一已签名
EXE 生成便携包，并在新的 GitHub runner 上执行签名回读和真实安装/启动/卸载。当前仓库尚未配置生产
PFX secrets，也没有新的受信签名 workflow 报告，因此本项继续保持未完成。

2026-08-15 又以当前 `372f9da` 在隔离 `CARGO_TARGET_DIR` 下完成 `0.1.1` Release 发布前预检，证据目录为
`target/release-preflight-v0.1.1-20260815/`，机器报告为 `preflight-report.json`。便携包启动 5 秒、
current-user 安装、隔离 `config/data/cache/history` 启动、静默卸载和零残留均通过；安装器和便携包的
manifest/sidecar 也通过，安装器 SHA-256 为 `864bde2f...5aeaf566`，便携包 SHA-256 为
`de3b6ba8...14e4046`。该构建的 Authenticode 仍为 `NotSigned`，所以它加强了版本与生命周期证据，
但不能替代生产 PFX、受信签名和 GitHub runner 的发布时真实验收；本项仍保持未完成。
- macOS 截图与平台实现，以及权限交互。
- 在承诺 Linux 功能对等前验证 Wayland portal 和 X11 可行性。

## 风险清单

| 风险 | 前置缓解措施 |
| --- | --- |
| GPUI 外部纹理能力不足 | 在编辑器开发前完成复制/上传技术验证 |
| GPUI API 变化频繁 | 核心与平台模块不依赖 GPUI，并锁定经过评审的提交 |
| 混合 DPI 坐标错误 | 以物理像素作为规范坐标并维护真实硬件矩阵 |
| 原生文本编辑复杂 | 在高级标注工具前验证 GPUI IME |
| 功能对等导致范围膨胀 | 使用里程碑退出标准，延后低频选项 |
| OCR 增加体积与启动时间 | 使用核心路径之外的延迟加载可选实现 |
| FFmpeg 残留或文件损坏 | 类型化生命周期、正常结束、超时和 Job 清理 |

## 当前环境记录

- 2026-07-16 首次 Windows 启动冒烟打开了可响应的 `Flash Shot` 窗口，但 GPUI 记录了 DirectX 错误 `0x887A002D`，表示该开发机缺少或不匹配 Windows SDK/图形组件。编译和测试已经通过；修复本机 DirectX/SDK 环境并完成视觉验证之前，里程碑 0 不能关闭。
- 2026-07-18 的 [Windows 截图技术验证报告](windows-capture-validation.md) 记录了 Release 快捷键采样、像素复制清单和当前限制。单次覆盖层首帧为 113.85 ms，不足以证明 100 ms p95 预算达标；需要真实混合 DPI 硬件矩阵和多次热启动采样。
- 2026-08-11 的 `overlay-interaction-acceptance --capture-scenario selection-transform` 在单屏
  2560x1440、DPI 96 环境完成真实选区移动、普通角点缩放、Shift 等比缩放和 Alt 中心缩放；
  报告与五张原生截图记录实际指针端点、提交矩形、不变量和 Cancel 后零残留。详见
  [Windows 手工验收记录](windows-manual-acceptance.md)中的
  `current-selection-transform-single-100`。
