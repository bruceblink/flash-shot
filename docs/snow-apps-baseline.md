# Snow Apps 参考基线

更新时间：2026-10-05

## 固定源码

- 仓库：[`mg-chao/snow-apps`](https://github.com/mg-chao/snow-apps)
- 固定提交：[`850d27c113b2f81b43950b839b6180af50e82d13`](https://github.com/mg-chao/snow-apps/commit/850d27c113b2f81b43950b839b6180af50e82d13)
- 固定时的提交标题：`feat(pinned): add lock mode for pinned screenshots (#1698)`
- 本地源码：`F:/project/snow-apps`，2026-10-05 快进同步到该提交，工作树干净。

Flash Shot 的 Snow Apps 对齐工作固定使用上述完整提交号。后续上游 `main`、`origin/main`、新标签或新提交不会自动改变此基准；只有用户明确选定新的提交后，才更新本文件和 [`docs/plan.md`](plan.md)。开始每个开发切片时无需追逐上游版本，只需确认对照源码仍可由固定提交号检出。

## 对齐范围

对照 Capture、Print、Library、Record、Pin、App 和截图工作区的用户可观察界面与核心流程，包括主要操作、忙/成功/失败状态、恢复动作、实际结果和窗口清理。固定提交中的 Pin 锁定模式进入 U4 的核心行为评估：提供清楚的锁定/解锁操作与状态，并在锁定时阻止 Pin 窗口的几何变化。

该基准用于选择和适配 `0.2.0` 的核心功能，不要求完整复刻 Snow Apps，也不要求移植其 Rust/C++/Qt/Tauri/Web 技术结构、源码分层或所有产品能力。

## 设置面板边界

Snow Apps 设置面板只用于了解某个核心功能是否需要用户配置。Flash Shot 不复制其面板结构、页面流程、分组方式或设置项集合；设置项按 Flash Shot 已有的简化布局、配置键兼容和产品范围单独判断。固定提交中的锁定边框颜色设置属于设置面板参考，不自动纳入实现范围。

## 基准更新

需要更换基准时，由用户指定新的 Snow Apps 完整提交号。更新前记录新旧提交和变化范围，重新核对受影响的核心流程，再同步修改本文件与开发计划。没有这项明确更新时，所有 UI 和核心功能差异仍以本文件固定的提交为准。
