# Flash Shot

<p align="center">
  <img src="resources/branding/logo-horizontal.svg" alt="Flash Shot" width="560">
</p>

[中文](#中文) | [English](#english)

## 中文

Flash Shot 是一款使用 Rust 和 [GPUI](https://www.gpui.rs/) 构建的高性能原生截图与录屏工具。项目以 Snow Shot 已验证的实用工作流作为需求参考，但不继承其 Tauri、WebView、React 和定制 Excalidraw 架构。

项目优先支持 Windows。启动速度、截图延迟、混合 DPI 正确性、稳定性以及可预测的资源生命周期，都是产品验收要求，而不是后期优化项。

### 当前状态

里程碑 0 正在进行。工程骨架参考了 `synchub-desktop` 与 `hiposter` 中经过验证的精简入口、模块组织、Tokio 后台运行时和原生资源打包方式。界面直接使用原生 GPUI，不依赖 `gpui-component`。

当前基线将 `gpui` 和官方 `gpui_platform` 启动模块锁定到同一个经过验证的 Zed 提交，不使用 crates.io 上较旧的 GPUI 版本，也不引入第三方组件库。

仓库目前包含：

- 可运行的 GPUI 原生应用壳；
- 与 UI 框架无关的路线图领域模型及测试；
- 产品需求、架构设计和分阶段开发计划；
- 本地与 CI 质量门禁。

目前尚未实现屏幕捕获。

### 运行

环境要求：

- Rust 1.92 或更高版本；
- Visual Studio 2022 MSVC 工具链与 Windows SDK；
- GPUI 在目标平台所需的原生构建环境。

```powershell
cargo run
```

### 验证

```powershell
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

连续截图资源与延迟门禁会真实捕获并编码虚拟桌面 100 次，输出机器可读 JSON；性能基线应使用 release 构建：

```powershell
cargo run --release --bin capture-stress -- --output target/capture-stress.json
```

开发时可使用较小轮数快速验证工具链：

```powershell
cargo run --bin capture-stress -- --iterations 5
```

### 文档

- [产品需求](docs/requirements.md)
- [架构设计](docs/architecture.md)
- [开发计划](docs/plan.md)

## English

Flash Shot is a high-performance native screenshot and screen recording application built with Rust and [GPUI](https://www.gpui.rs/). It uses Snow Shot's proven workflows as product research without inheriting its Tauri, WebView, React, or customized Excalidraw architecture.

The project is Windows-first. Startup time, capture latency, mixed-DPI correctness, stability, and predictable resource ownership are acceptance requirements rather than later optimization work.

### Status

Milestone 0 is in progress. The engineering shell borrows the thin entry point, module layout, Tokio background runtime, and native resource packaging patterns proven in `synchub-desktop` and `hiposter`. The UI uses GPUI directly without `gpui-component`.

The baseline pins `gpui` and the official `gpui_platform` launcher to the same reviewed Zed commit. It uses neither the older crates.io GPUI release nor a third-party component suite.

The repository currently contains:

- a runnable native GPUI application shell;
- framework-independent roadmap domain types with tests;
- product requirements, architecture decisions, and a staged delivery plan;
- local and CI quality gates.

Screen capture is not implemented yet.

### Run

Requirements:

- Rust 1.92 or newer;
- Visual Studio 2022 MSVC toolchain and Windows SDK;
- native build prerequisites required by GPUI on the target platform.

```powershell
cargo run
```

### Validate

```powershell
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The repeat-capture resource and latency gate captures and encodes the virtual desktop 100 times and emits machine-readable JSON. Use a release build for performance baselines:

```powershell
cargo run --release --bin capture-stress -- --output target/capture-stress.json
```

Use fewer iterations for a quick development smoke test:

```powershell
cargo run --bin capture-stress -- --iterations 5
```

### Documentation

- [Product requirements (Chinese)](docs/requirements.md)
- [Architecture (Chinese)](docs/architecture.md)
- [Delivery plan (Chinese)](docs/plan.md)

## License

GNU Affero General Public License v3.0 only (`AGPL-3.0-only`).
