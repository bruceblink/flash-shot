# 开发流程与验收证据

本文档把 Flash Shot 的并行开发、真实输入验收和证据归档收敛为一条可复用的流程。它服务于
截图核心链路的可靠性、性能和 UI 人体工学验收；OCR、翻译等能力属于未来扩展，不因本流程而扩大当前
交付范围。

## 1. 先划边界，再并行

每个切片只解决一个用户可感知的小功能，并在开始前写清楚完成标准、风险、允许修改的文件和明确的
非目标。适合并行的职责通常包括：

- 核心行为与状态迁移：截图、选区、编辑、复制、保存、取消和资源生命周期；
- 原生交互与性能：真实键鼠输入、Release 延迟、窗口/显示器/DPI 矩阵；
- 视觉与证据：控件层级、截图比对、结构化报告和文档。

代理之间优先使用独立文件边界。共享文件必须指定唯一写入者，其余代理只做审查或提供补丁建议；不得
用事后冲突合并代替职责划分。每个代理回报变更文件、假设、命令和证据位置，发现边界外问题时单独登记，
不要顺手扩展范围。

## 2. 验证顺序和完成判定

按改动风险从近到远执行验证，且在验证通过前不提交该切片：

1. 先运行直接覆盖改动的单元测试、集成测试或静态检查；
2. 再运行 `cargo fmt --all -- --check`、受影响目标的 `cargo check`/`cargo clippy`，以及必要的回归测试；
3. 对用户可见流程使用 Release 程序，在一次性或明确授权的 Windows 桌面执行真实操作；
4. 保存关键步骤截图，做像素/几何和视觉布局比对；
5. 检查报告、截图和导出产物属于同一会话，确认输入释放、窗口清理和超时收敛；
6. 只有静态门禁与该切片所需的真实验收都通过，才把它标记为完成，并按一个小功能一次提交的规则提交。

“能编译”只说明代码路径可构建；“单次通过”只说明该环境的一次闭环成立；批量性能和多显示器/DPI
矩阵必须分别用对应证据声明，不能互相替代。

B0 的场景输入、失败恢复动作和可见状态来源登记在 [Bug 与可见文案基线](bug-ui-baseline.md)。运行
`powershell -NoProfile -File scripts/verify-bug-ui-baseline.ps1` 可重新生成基线报告；该报告只证明登记和
静态来源可重复，不替代真实 Windows 交互验收。

B1 当前已完成保存/快速保存、历史索引和录屏进程清理三个可独立验证的恢复切片：保存或快速保存失败时，
workflow 回到 `Selecting` 并保留原选区，允许用户选择其他位置后再次 Save；历史索引写入失败时，索引临时
文件会清理，历史新增、清空、删除和后台移除保持可重试状态；录屏启动阶段失败会终止并回收已创建的
子进程，运行中异常退出会产生 `Failed` 事件，停止超时会回收子进程和输出读取器。确定性回归测试包括
`save_failures_keep_the_existing_selection_available_for_retry`、
`stale_save_failures_explain_when_a_new_capture_is_required`、
`record_index_failure_keeps_the_capture_available_for_retry` 和
`forget_deleted_index_failure_preserves_entries_for_a_retry`，以及
`recording_worker_reports_an_abnormal_exit_without_panicking`、
`process_stop_timeout_terminates_and_reaps_the_child` 和
`startup_cleanup_terminates_a_child_before_process_construction`；真实只读目录、剪贴板和 FFmpeg 用户界面
故障注入仍必须在同一 Release 会话中补充证据。

## 3. 真实输入与剪贴板授权

生产 UI 验收必须从可见覆盖层开始，用原生鼠标和键盘注入完成选区、动作和清理，不得直接调用内部
复制函数、状态转换或测试专用快捷路径来代替操作链路。

`overlay-interaction-acceptance` 的授权边界如下：

- `--allow-input` 才允许真实输入注入；没有它的运行不能宣称完成 UI 交互验收；
- 默认 Copy 使用进程内观察器，不改写用户剪贴板；
- `--allow-system-clipboard` 只允许在可丢弃的 Windows 会话中使用生产 `SystemClipboard`，因为它会替换
  当前系统剪贴板；Copy 前必须启动无窗口的独立消费者；
- `--copy-trigger enter` 验证覆盖层的 `Enter` 键盘出口，`toolbar` 验证首屏 Copy 点击。两者都必须记录
  实际触发器，不能把工具栏结果错报成键盘结果。

系统剪贴板消费者必须先写入 ready 标记，再由父进程写入带基线序号的 start 标记；消费者进入单调
剪贴板等待循环后再写入 observing 标记。只有 observing 已出现、消费者读到新的序号并被成功回收，
才算“可读”；父进程随后等待结果、回收子进程，并在超时或异常时走有界终止。报告至少要能回答：

- `trigger`、`sink` 和 `read_mechanism` 是什么；
- `clipboard_sequence_before`/`clipboard_sequence_after` 是否变化；
- `consumer_ready_before_click`、`consumer_observing_before_click`、`consumer_cleaned_up` 和
  `single_export_verified` 是否为真；
- `editor_retained_after_copy` 是否证明 Copy 完成后仍保留原选区与唯一覆盖层，
  `cleanup_after_escape` 是否证明随后由显式 Escape 完成清理；
- PNG、CF_DIB 与常规图像消费者是否都逐像素匹配源帧。

任何一项缺失、消费者未回收、键盘或鼠标按键未释放、或超时后仍有残留，都只能记录为失败/待执行，不能
通过“剪贴板格式已注册”或应用内提示降级为成功。

## 4. 截图、像素和视觉证据

每个真实场景至少保留以下三类产物，并使用同一会话目录和时间戳关联：

1. **结构化报告**：命令行参数、构建配置、显示器与 DPI、真实输入端点、状态步骤、耗时边界、导出路径、
   失败原因和清理结果；
2. **原生步骤截图**：选区就绪、关键动作前后、编辑器/菜单状态和结束清理后的桌面。截图用于检查控件
   是否遮挡捕获区域、按钮和文案是否截断、主次动作是否清晰、窗口是否溢出或重叠；
3. **像素产物**：源帧、PNG/CF_DIB/消费者读回或保存文件的尺寸、字节数和指纹。导出正确性以逐像素比较
   为准；视觉截图不能单独证明不可见状态，JSON 也不能单独证明布局质量。

比对时先确认物理像素边界和尺寸，再比较指纹；若采用容差，必须在报告中写明原因和阈值。截图、报告、
导出文件或消费者结果不属于同一会话时，证据无效。结束时应再次确认覆盖层、操作条、Pin、控制窗口、
验收 fixture、后台任务和可见进程窗口数量符合场景预期，并验证 `Ctrl`、`Alt`、`Shift`、`Enter`、
`Escape`、`Space` 和鼠标左键等注入按键均已释放。

Save -> Pin 或再次 Capture 还必须经过 teardown 屏障：应用在 deferred native HWND close callback 尚未完成时
暴露 `capture_teardown_pending=true`，runner 不得把 `overlay_count=0` 当作已经清理。只有该标志清零、
`capture_preflight_ready=true`，并且连续桌面采样在 settle 窗口内稳定后，才允许开始下一次全屏采集；Save
流程同时保存 `save-complete-clean.png`。这样可以把 GPUI deferred close、DWM 合成残留与下一张截图的源帧
隔离开，避免“应用内像素相等”掩盖桌面残影。

PNG 文件 Save 使用逐行 BGRA 到 RGBA 的流式编码，并通过原子临时文件替换目标路径；Copy 仍保留内存
编码契约。验收要同时检查解码后像素恒等、目标文件完整存在、成功后临时文件消失，以及编码或同步失败
时不会留下目标半成品。该优化只覆盖导出缓冲和文件写入，不把 PNG 编码耗时混入普通 Copy 的 QPC 指标。

## 5. 性能样本和矩阵边界

延迟结论必须说明计时起点和终点。例如普通 Copy 的端到端指标从真实 Copy 输入发出开始，到独立系统
剪贴板消费者成功读取并解码结束；PNG 编码基准、保存对话框耗时和录屏耗时不能混入该指标。批量门禁应
记录预热数、有效样本数、p50、p95、最大值和失败数；单次报告只能作为单次闭环证据。

单屏 100% 与 150%/200%、多屏混合 DPI、窗口遮挡/最小化等属于不同环境矩阵。未执行的矩阵保持“待执行”，
不要从单屏结果推断平台普遍正确。任何性能回归都优先修复核心链路，不能用新增低频功能抵消预算。

历史 Library 的缩略图属于低优先级后台工作。展开长列表时应使用有界 FIFO 解码队列，固定并发
上限并在历史删除、切换目录或异步结果返回时重新检查路径归属；不能因为一次渲染就为所有记录
启动无界 PNG 解码任务。该门禁与截图到复制/保存的端到端延迟分开记录。

历史流控切片的通过条件是：300 条请求的解码中任务始终不超过 2，队列保持 FIFO，重复渲染不增加
重复任务，历史删除后没有陈旧结果写回；单元回归只能证明这些不变量，不能替代 Release 资源样本。
资源样本须记录构建 profile、显示器/DPI、默认 5 条与显式 300 条展开、首批缩略图时间、峰值私有字节、
截图和报告路径。当前实现已通过代码/全库回归；Release 单屏样本已完成并归档在
`target/history-resource-acceptance/release-gate-clean-20260813/report.json`，默认预览条数保持不变。
新的资源探针会在指定输出目录下创建独立的 `session-<timestamp>-<pid>` 目录，并将报告、基线截图、
设置和指标写入其中；启用 `--exercise-failures` 时还会追加三张故障/恢复/目录切换截图。临时 history fixture
同样受该目录约束并在清理阶段删除，避免并行或重试会话覆盖彼此的证据。

2026-08-29 当前源码的 Release 资源样本位于
`target/history-resource-acceptance/release-normal-20260829/session-1787983224655-16012/report.json`：
报告为 `passed=true`，默认 5 条与展开 300 条均完成，`peak_loading=2`、`thumbnails_cached=300`、`thumbnails_failed=0`，并确认
`fixture_files_removed=true`、`history_root_exists=false`。Windows runner 会在发送 GPUI Quit 前写入报告并删除
隔离 fixture，因为 GPUI 的退出路径会直接结束进程；该顺序保证报告和清理证据不会丢失。

故障恢复样本位于
`target/history-resource-acceptance/release-fault-20260829/session-1787982960787-31984/report.json`：
`--exercise-failures` 在同一窗口中验证损坏/缺失文件、两个条目重试和 3 条新目录切换，报告的
`failures_2`、`recovered_300`、`directory_switch_3` 均收敛，原始与切换目录均清理成功。

历史删除专项使用 `--exercise-deletions`。runner 会切换到隔离的 6 条记录目录，调用生产单项删除流程移除 1 条，
再调用生产批量清理流程移除 2 条；`deletion_initial_6`、`deletion_single_5`、`deletion_batch_3` 会同时等待
历史索引、缩略图队列和文件读取/删除互斥状态收敛，并为两个结果保存同一会话的窗口截图。报告的
`deletion_scenario` 还会记录实际删除路径和隔离目录清理结果。窗口关闭专项使用 `--exercise-window-close`：runner
会在 60 条缩略图仍处于加载/排队时沿生产隐藏路径关闭 Library，等待隐藏期间队列收敛，再重新打开窗口并保存恢复截图；
报告的 `window_close_scenario` 会记录关闭时状态、隐藏期间收敛状态、重开后的状态、窗口可见性和清理结果。两个专项
都证明异步结果不会写回陈旧条目，可与 `--exercise-failures` 组合运行：

```powershell
.\scripts\run-dev-tool.ps1 history-resource-acceptance -Release --exercise-failures --exercise-deletions
```

窗口关闭专项单独运行：

```powershell
.\scripts\run-dev-tool.ps1 history-resource-acceptance -Release --exercise-window-close
```

常规视觉切片选择“拖选 -> Save 取消/保存 -> Pin -> Copy -> 清理”主流程。通过条件是同一 Release
会话同时具备真实 `--allow-input` 报告、关键步骤原生截图、Save/Pin/Copy 逐像素结果和最终
`capture_teardown_pending=false`、overlay/Pin/可见窗口为零；Save 完成后必须经过桌面稳定屏障再开始下一次采集。
风险包括前台抢占、热键冲突、DWM 延迟和非 100% DPI。每次源码或 runner 变化后都必须重建对应 Release
证据，旧报告只能作为历史对照，不得归因于新二进制。

Pin 的性能边界单独记录：选区、剪贴板、全屏和历史四个生产入口都先在后台 executor 完成像素合成、
解码或 `render_image_from_capture`，UI 线程只负责打开已准备的原生窗口。验证必须同时覆盖 generation
门禁、错误回收、像素恒等和窗口清理；不能只用“窗口出现”推断后台任务已经完成。前台窗口不属于验收进程、
输入按键未释放或 watchdog 超时，都必须 fail-closed 并保留失败报告。

## 6. OCR 与后续扩展

当前版本的验收对象是截图核心链路：触发、选区、捕获、编辑、复制、保存、取消、清理，以及相关性能和
视觉证据。OCR、翻译、模型、语言覆盖和在线服务不属于本轮 Copy/Enter 或 UI 核心验收；不要新增识别引擎，
也不要把 OCR 初始化计入 Copy 计时区间。

后续启用 OCR 时，应单独建立切片和验收报告，保持按需加载、可关闭、无端点时不联网，并证明它不会增加
启动或核心复制路径的延迟。OCR 通过自己的识别准确率、失败恢复、临时文件清理和隐私检查后，才能单独
标记为完成；它不能覆盖或替代核心截图证据。

## 7. 证据清单模板

提交前在切片记录中逐项填充：

- [ ] 目标、非目标、文件边界和代理负责人已写明；
- [ ] 直接测试、格式、编译、Clippy 和回归结果已记录；
- [ ] Release 真实输入命令、授权开关、前台窗口和显示器/DPI 已记录；
- [ ] 关键步骤截图、源/结果像素比对和 `report.json` 路径已关联；
- [ ] Copy 的 `trigger`/`sink`、剪贴板序号、消费者 ready/cleanup 和单次导出断言已核对；
- [ ] 按键释放、窗口/进程/任务清理和全流程超时结果已核对；
- [ ] 未执行环境、样本不足或 OCR 延后事项明确标为“待执行”，没有写成已完成；
- [ ] `git diff --check` 通过后，才提交单一功能。
