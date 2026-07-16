# 架构设计

## 1. 设计原则

1. GPUI 是界面与交互层，不是业务数据模型。
2. 平台 API 必须隐藏在小而明确的接口之后，平台细节不能进入文档模型。
3. 截图像素必须具有明确的所有权，不做无意义的编码和复制。
4. 长耗时工作运行在后台执行器中，并支持取消。
5. 每条用户工作流使用显式状态机，失败状态必须可观察。
6. 设置和持久化文档从首次发布起就带版本。

## 2. 工程基线

应用壳选择性复用同级项目 `synchub-desktop` 和 `hiposter` 中已验证的工程习惯，并结合 Zed 官方当前示例：

- 使用锁定到具体 Zed 提交的 `gpui` 和官方 `gpui_platform`；
- 在 GPUI 事件循环启动前进入 Tokio 多线程运行时；
- 通过 `build.rs` 和 `resources` 目录管理原生图标与打包资源；
- `main.rs` 保持精简，在 `lib.rs` 完成应用装配，功能状态拆入独立模块；
- 使用 `Context`、`Entity`、`WeakEntity`、`spawn` 和 `notify` 组织 UI 状态与异步更新。

Flash Shot 不依赖 `gpui-component` 或 `gpui-component-assets`。截图覆盖层和标注工具需要直接控制布局、输入、绘制、焦点和帧行为；通用组件库会增加不必要的升级和样式边界。可复用控件将以小型、产品专用的 GPUI 模块实现。

`gpui_platform` 属于当前官方 GPUI workspace，只负责构造对应操作系统的原生平台实现，不是组件或样式依赖。`gpui` 与 `gpui_platform` 必须锁定到同一提交。升级提交是一个需要独立编译、交互和性能验证的功能，不能无审查地跟随 Git 主线。

## 3. 目标模块

初期维持单 crate，只有在实际代码证明边界稳定后才拆分 workspace：

```text
flash-shot-app             GPUI 界面装配与应用生命周期
flash-shot-core            截图会话与应用用例
flash-shot-annotation      文档、命令、几何、命中测试、历史
flash-shot-image           裁剪、滤镜、合成、编码、颜色转换
flash-shot-platform        平台无关接口
flash-shot-platform-win    Windows 截图、输入、剪贴板、托盘、UI Automation
flash-shot-scroll          滚动截图匹配与合成
flash-shot-ocr             OCR 接口与本地实现
flash-shot-recording       FFmpeg 进程与录屏状态机
```

依赖方向从第一天开始执行：

```text
GPUI 应用 -> 应用用例 -> 领域/核心 <- 平台实现
                          |
                          +-> 图像与标注算法
```

核心模块不得依赖 GPUI、HWND、COM 对象、FFmpeg 进程或具体 OCR 运行时。

## 4. 截图管线

```text
全局快捷键
  -> 创建截图会话
  -> 获取各显示器帧
  -> 上传并缓存不可变纹理
  -> 覆盖层选择与标注预览
  -> 确定性图像合成
  -> 剪贴板 / 文件 / 贴图 / OCR
```

第一个技术验证必须统计 CPU 内存复制、GPU 上传次数、分配次数和帧生命周期。单次截图上传一次纹理可以接受；每帧重新上传或解码整张截图不可接受。

## 5. 标注文档

标注数据使用逻辑图像坐标和稳定 ID。渲染阶段将文档坐标转换为视口坐标；导出阶段使用原图尺寸进行合成，而不是截取应用界面。

操作通过命令模型表达。命令保存逆操作或足以完成撤销/重做的前后状态。鼠标移动可以产生临时预览，但只有正式提交的操作才能进入历史记录。

## 6. 平台边界

候选接口包括：

- `CaptureBackend`
- `DisplayProvider`
- `GlobalShortcutService`
- `ClipboardService`
- `TrayService`
- `WindowInspector`
- `AutoStartService`
- `RecordingBackend`

接口描述产品操作和错误，不应逐个映射操作系统 API。

## 7. FFmpeg 边界

首个录屏后端启动随应用分发或由用户选择的 FFmpeg 可执行文件，并负责：

- 能力和设备探测；
- 参数构建；
- 持续消费 stdout/stderr 并解析进度；
- 优先正常结束封装，超时后才强制终止；
- Windows Job Object 或进程组清理；
- `idle/starting/recording/paused/stopping/failed` 类型化状态机。

只有测量证明进程边界成为实质瓶颈后，才考虑直接集成 libav。

## 8. 测试策略

- 几何、文档、命令、状态机、命名和配置使用纯单元测试。
- 图像合成与标注输出使用 golden image 测试。
- 坐标转换和资源释放使用平台契约测试。
- 选区和工具栏交互在可行时使用 GPUI 交互测试。
- 在自动化成熟前维护真实硬件混合 DPI 测试矩阵。
- 建立可重复的延迟、帧时间、工作集、句柄和纹理基准。
