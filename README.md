# Flash Shot

Flash Shot is a fast, native screenshot and screen recording application built with Rust and
[GPUI](https://www.gpui.rs/). It takes the practical workflow coverage of Snow Shot as product
research while replacing the Tauri, WebView, and customized Excalidraw architecture.

The project is Windows-first. Performance, stability, mixed-DPI correctness, and predictable
resource ownership are product requirements rather than later optimization work.

## Status

Milestone 0 is in progress. The engineering shell borrows the thin entry point, module layout,
Tokio background runtime, and native resource packaging patterns from `synchub-desktop` and
`hiposter`. The UI itself intentionally uses GPUI directly, without `gpui-component`.
The initial baseline pins GPUI and its official `gpui_platform` launcher to the same reviewed Zed
commit. No crates.io GPUI release or third-party component suite is used.

The repository currently contains:

- a runnable GPUI application shell;
- platform-independent roadmap types with tests;
- product requirements, architecture decisions, and a staged delivery plan;
- local and CI quality gates.

It does not capture the screen yet.

## Run

Requirements:

- Rust 1.92 or newer;
- the native build prerequisites required by GPUI on the target platform.

```bash
cargo run
```

## Validate

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Documentation

- [Product requirements](docs/requirements.md)
- [Architecture](docs/architecture.md)
- [Delivery plan](docs/plan.md)

## License

Licensed under the MIT license.
