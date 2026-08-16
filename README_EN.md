# Flash Shot

<p align="center">
  <img src="resources/branding/logo-horizontal.svg" alt="Flash Shot" width="560">
</p>

[中文](README.md) | **English**

Flash Shot is a high-performance native screenshot and screen recording application built with Rust and [GPUI](https://www.gpui.rs/). It uses Snow Shot's proven workflows as product research without inheriting its Tauri, WebView, React, or customized Excalidraw architecture.

The project is Windows-first. Startup time, capture latency, mixed-DPI correctness, stability, and predictable resource ownership are acceptance requirements rather than later optimization work.

## Status

The current mainline implements native Windows capture, annotation, scrolling capture, and an FFmpeg-based recording workflow. The engineering shell borrows the thin entry point, module layout, Tokio background runtime, and native resource packaging patterns proven in `synchub-desktop` and `hiposter`. The UI uses GPUI directly without `gpui-component`.

The baseline pins `gpui` and the official `gpui_platform` launcher to the same reviewed Zed commit. It uses neither the older crates.io GPUI release nor a third-party component suite.

The repository currently includes:

- virtual-desktop capture, mixed-DPI selection, window/control inspection, magnification, and keyboard nudging on Windows;
- delayed capture, optional system-cursor compositing, copy/save, pinning, and screenshot history;
- native rectangle, ellipse, line, arrow, pen, text, blur, mosaic, highlight, watermark, and sequence annotations;
- manual, assisted, and scroll-then-auto-capture workflows, QR recognition, optional local OCR, HTTPS translation, and in-overlay retries after a failure;
- display, window, and region recording with audio selection, pause/resume, progress, and Job Object cleanup;
- repeatable performance/resource stress tooling, structured diagnostics, and local quality gates.

The recording backend and the display, window, and region UI lifecycles have passed acceptance on
one 100%-scaled display. Fixed-source desktop-composition semantics after moving, resizing,
occluding, and minimizing a recorded window are also covered by a native Release probe. Producing
an MP4 requires an FFmpeg build that supports `ddagrab` or `gdigrab`; 150%/200% scaling remains in
the real-environment matrix.

## Download and security notice

Prebuilt binaries are distributed only through [GitHub Releases](https://github.com/bruceblink/flash-shot/releases).
The Windows installer and portable build are intentionally unsigned, so the GitHub Release workflow
does not require or use `WINDOWS_SIGNING_CERTIFICATE_BASE64` or
`WINDOWS_SIGNING_CERTIFICATE_PASSWORD`. Windows may show a SmartScreen or unknown-publisher warning.
Download only from the page above, verify the accompanying SHA-256 sidecar and release manifest, and
continue only if you accept that risk.

## Run

Requirements:

- Rust 1.95 or newer;
- Visual Studio 2022 MSVC toolchain and Windows SDK;
- native build prerequisites required by GPUI on the target platform.

```powershell
cargo run
```

The workspace root selects the `flash-shot` package in `crates/flash-shot-bin` as its only default
member, which declares the only application binary, so no `--bin` argument is needed.
Stress and native acceptance programs remain available as opt-in `dev-tools` library modules via
`scripts\run-dev-tool.ps1`; they do not add executables to release packages or alter normal
`cargo run` startup.

The source is split into small responsibility-focused workspace crates: `flash-shot-domain` owns pure
product models, `flash-shot-image` owns immutable capture frames and image processing,
`flash-shot-infra-windows` owns display enumeration, Windows screen capture, global shortcuts, the tray, the clipboard, auto-start, directory opening, process lifecycle, and window control, `flash-shot-app` owns
application workflows plus GPUI and the remaining platform composition, and `crates/flash-shot-bin` is the
only startup entry point. Every other crate is a library and is never started as an additional program by
`cargo run`.

On first launch, the interface follows Windows' preferred UI language: supported Simplified
Chinese regional tags use the Simplified Chinese catalog and all other languages fall back to
English. A language selected later in Settings is persisted as the user's preference and is not
replaced by a subsequent system-language change.

Recording uses a local or bundled FFmpeg. Videos go to `Videos\Flash Shot` by default and fall back to Flash Shot's application-data directory when that folder is unavailable. Set `FLASH_SHOT_FFMPEG` and optionally `FLASH_SHOT_RECORDING_DIRECTORY` to override the executable and output directory. The `Audio` control discovers supported local FFmpeg inputs on demand and cycles between automatic configuration, off, DirectShow microphones, and available WASAPI system audio. `auto` preserves the environment-variable behavior documented above.

The Record page's `Video folder` control shows the active MP4 destination and lets users choose, reset, verify, or open it. A chosen folder is persisted; `FLASH_SHOT_RECORDING_DIRECTORY` remains the authoritative override for managed or portable environments. When timestamped names collide, Flash Shot appends a suffix and FFmpeg refuses to overwrite an existing MP4.

The `Display` control cycles recordable monitors in primary-first order. Both display and audio discovery happen only after the respective control is clicked, keeping startup free of FFmpeg probing.

The capture shortcut defaults to `Ctrl+Shift+Print Screen`. Set `FLASH_SHOT_CAPTURE_HOTKEY` before launch to use a different safe global combination, for example `Ctrl+Alt+S`, `Shift+F12`, or `Ctrl+PrintScreen`. It must include `Ctrl`, `Alt`, or `Shift`, plus one letter, `F1` through `F24`, or `PrintScreen`; invalid values fall back to the default.

For a deterministic Release regression check of the clipboard path, run the opt-in synthetic gate below. It replaces
the current system clipboard, measures 30 generated `1280x720` frames by default, records every valid and failed
sample, and requires at least 30 valid samples with no failures and p95 at or below 250 ms. This command does not
inject real UI input or qualify the required external-consumer benchmark; use the Windows acceptance runner for that
separate evidence:

```powershell
.\scripts\run-dev-tool.ps1 -Release copy-performance --allow-system-clipboard --output target/copy-performance/release.json --metrics-dir target/copy-performance/metrics
.\scripts\run-dev-tool.ps1 -Release performance-report --input target\copy-performance\metrics\performance.jsonl --minimum-samples 30 --copy-only --output target\copy-performance\summary.json
```

For the required real Windows UI batch, run the isolated development-tool module in a disposable
desktop session. It starts one `flash-shot` child per warmup/sample and dispatches that child to
the `overlay-interaction-acceptance` module, authorizes the production system clipboard explicitly, and stops the
batch after any session whose cleanup cannot be proven. The default is two warmups plus 30 real
toolbar Copy samples. Each child uses the focused `copy-only` scenario, which still opens a real
overlay, commits a real mouse selection, invokes the requested production Copy gesture, validates
an external consumer, and closes the editor with Escape without spending the batch on unrelated
Save or Pin setup. `batch-report.json` contains every child report path, sample list, p50/p95,
failures, display DPI, build path/profile, and QPC timing boundary. These commands change the
current Windows clipboard and inject global input:

```powershell
.\scripts\run-dev-tool.ps1 -Release overlay-copy-batch --allow-input --allow-system-clipboard --copy-trigger toolbar --warmup 1 --copy-iterations 30 --output-dir target\overlay-copy-batch\toolbar
.\scripts\run-dev-tool.ps1 -Release overlay-copy-batch --allow-input --allow-system-clipboard --copy-trigger enter --warmup 1 --copy-iterations 30 --output-dir target\overlay-copy-batch\enter
```

The batch report is `measurement_mode=real_ui` and `real_ui=true`. Current-source Release evidence
contains 1 warmup plus 30/30 valid samples for both toolbar and Enter Copy, with zero failures on a
96-DPI display:

- toolbar: `target/overlay-copy-batch/final-58a93df-toolbar-20260816/session-1786847742458-11292/batch-report.json`
  (p50 `27.0499 ms`, p95 `27.901 ms`, maximum `28.3013 ms`);
- Enter: `target/overlay-copy-batch/final-58a93df-enter-20260816/session-1786847841195-14420/batch-report.json`
  (p50 `27.4856 ms`, p95 `28.8618 ms`, maximum `29.6429 ms`).

All 62 warmup/sample iterations passed the production clipboard, PNG/CF_DIB/normal-consumer pixel,
editor-retention, explicit-Escape, and cleanup checks. Both reports use runner SHA-256
`5d33d90d5f395555240d409829df5896a4eb85fa29820b179f68da03ccc22014`, matching the rebuilt
Release executable for commit `58a93df`. Re-run both batches after source or binary changes;
historical artifacts are not substitutes for current-source evidence. The synthetic
`copy-performance` report must not be used as a substitute for this evidence. Use
`--copy-trigger enter` to measure the keyboard route, or `--copy-iterations 30` to make the minimum
sample count explicit.

The global capture shortcut can be disabled from the tray `System` menu or Capture settings without changing its configured key combination. The preference persists across restarts, while the tray capture commands remain available.

Capture settings can configure separate global shortcuts for region capture, full-screen capture,
and focused-window capture. They are registered as one native set; duplicate combinations are
rejected, and setting a secondary action to `Off` releases that key.

Use `Capture > Focused window` to hide Flash Shot, resolve the current external foreground window,
and open that physical-pixel rectangle as an editable selection. If the target is partly outside the
virtual desktop, the selection is clipped to captured pixels rather than producing an invalid crop.

`Start with Windows` configures only the current user's sign-in entry. It does not require elevation, and it never removes a `Flash Shot` startup entry that points to a different executable.

Use `Files > Pin clipboard image` from the tray menu to turn the current Windows clipboard image into an always-on-top reference without opening the settings window or starting a new capture.

Set `FLASH_SHOT_UPDATE_ENDPOINT` to an HTTPS URL serving a verified `release-manifest.json` to enable the optional `Check Updates` button. No update request, download, or installation happens unless the user explicitly clicks the button; see [Windows distribution](docs/windows-distribution.md#manual-update-check) for the manifest contract.

## Validate

```powershell
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

The repeat-capture resource and latency gate captures and encodes the virtual desktop 100 times and emits machine-readable JSON. Use a release build for performance baselines:

```powershell
.\scripts\run-dev-tool.ps1 -Release capture-stress --output target/capture-stress.json
```

Use fewer iterations for a quick development smoke test:

```powershell
.\scripts\run-dev-tool.ps1 capture-stress --iterations 5
```

The app also retains the latest 500 startup and shortcut-to-overlay samples at
`<application data directory>\metrics\performance.jsonl`. Summarize a representative release
run and enforce the default p95 thresholds with:

```powershell
.\scripts\run-dev-tool.ps1 -Release performance-report --input "<application data directory>\metrics\performance.jsonl" --output target/performance-summary.json
```

The command accepts only Release-profile samples by default, so Debug runs and legacy unmarked
records cannot affect a release p95 decision. Its JSON reports `release_gate_applied: true` when
at least one enabled p95 gate uses Release samples, and `release_qualified: true` only when every
enabled gate has enough samples and passes. It exits with status `2` when a threshold fails, and
with status `1` when samples are malformed or insufficient. `--no-gate` and
`--include-nonrelease` produce exploratory reports with both fields set to `false`.

Collect an isolated twenty-startup Release baseline without mixing older samples into the result:

```powershell
.\scripts\measure-release-startup.ps1
```

This starts the Release executable twenty times, writes a time-windowed startup-only p95 report to
`target\release-startup-performance.json`, and stops on a startup crash, a missing current sample,
or a startup p95 above 500 ms. It deliberately does not claim full release qualification because
the shortcut-to-overlay metrics need their own real interaction sampling.

Use `-SkipBuild` only when the production Release application was built from the current source;
the script always checks the isolated development-tool reporter protocol before it starts sampling.

Collect an isolated shortcut-to-overlay baseline with the dedicated `Ctrl+Alt+F12` hotkey:

```powershell
.\scripts\measure-release-capture.ps1
```

The script starts one Release application, triggers and cancels twenty real capture overlays, and
gates the current window's frame-ready and overlay p95 values at 100 ms. It requires an
interactive Windows desktop and no existing Flash Shot process.

Exercise the complete real single-display screenshot path in an isolated profile. The default mode
drags and nudges a selection, expands and collapses More, recaptures and cancels, drives the native
Save dialog into an isolated path, opens and closes a Pin, then clicks Copy through a process-local
image sink. It maps the actual pointer path through the measured client area, checks the committed
physical selection, compares Save/Pin/Copy pixels with their pre-click source frames, and proves
the Windows clipboard sequence did not change:

```powershell
.\scripts\run-dev-tool.ps1 -Release overlay-interaction-acceptance --allow-input --output-dir target/overlay-interaction-acceptance
```

To exercise the production system-clipboard export for the same ordinary selection Copy path,
authorize it explicitly in a disposable Windows desktop session:

```powershell
.\scripts\run-dev-tool.ps1 -Release overlay-interaction-acceptance --allow-input --allow-system-clipboard --output-dir target/overlay-interaction-system-clipboard-acceptance
```

This mode replaces the current system clipboard and is intended only for a disposable acceptance
session. Before Copy is clicked, it starts a separate windowless consumer. The consumer reads the
result through PNG, CF_DIB, and normal-image paths; the parent then compares each result with the
selection source frame pixel by pixel. `report.json` records the clipboard sequence number,
monotonic timing, format artifacts, pixel fingerprints, consumer observation state, and child
cleanup state. The default command still uses an in-process observer and does not modify the user
clipboard. Both toolbar Copy and one real `Enter` shortcut input have single-display Release
reports. `copy-performance` covers only the compositing regression baseline (report schema 2,
`real_ui=false`); p50/p95 and failure counts for at least 30 real UI samples remain governed by the
follow-up acceptance items in the [delivery plan](docs/plan.md).

To validate the keyboard export in the same standard scenario, switch the trigger to `Enter`:

```powershell
.\scripts\run-dev-tool.ps1 -Release overlay-interaction-acceptance --allow-input --allow-system-clipboard --copy-trigger enter --output-dir target/overlay-interaction-system-clipboard-enter-acceptance
```

Use the opt-in narrow-edge scenario to open the real 420x420 minimum Settings client, drag a
160x96 selection against the bottom-right display edge, and click More/Less and Mark open/close.
It requires one 100%-scaled display, records the exact selection pixels and UI states, and remains
mutually exclusive with recording modes:

```powershell
.\scripts\run-dev-tool.ps1 -Release overlay-interaction-acceptance --allow-input --capture-scenario narrow-edge --output-dir target/overlay-interaction-narrow-edge-acceptance
```

Use the opt-in Pin coexistence scenario to create three 360x240 Pins through real toolbar clicks,
arrange them without overlap, move one through the native image drag surface, and trigger Capture
from a focused Pin. It requires one 100%-scaled display, proves all three source frames and HWND
bounds survive the overlay and Cancel, then closes every Pin with Escape without using the system
clipboard:

```powershell
.\scripts\run-dev-tool.ps1 -Release overlay-interaction-acceptance --allow-input --capture-scenario pins-coexist --output-dir target/overlay-interaction-pins-coexist-acceptance
```

To exercise the real Pin system-clipboard export in the three-Pin coexistence workflow, add
`--allow-system-clipboard` explicitly. This replaces the current system clipboard through
`Ctrl+C` on the first Pin and should run only in a disposable Windows session. The report records
the clipboard sequence number, PNG/CF_DIB formats, consumer readback, and pixel-by-pixel validation
under `pins_coexist.system_clipboard_copy`. Omitting the flag keeps the isolated in-process path and
does not modify the user clipboard:

```powershell
.\scripts\run-dev-tool.ps1 -Release overlay-interaction-acceptance --allow-input --capture-scenario pins-coexist --allow-system-clipboard --output-dir target/overlay-interaction-pins-system-clipboard-acceptance
```

Use the opt-in selection-transform scenario to perform real committed-selection gestures in one
overlay: move from the selection interior, resize the bottom-right corner, resize with Shift to
preserve the aspect ratio, and resize with Alt around the original center. It requires one
100%-scaled display, records the actual pointer endpoints and committed physical rectangles, and
cleans up the overlay and Settings controller without touching the system clipboard:

```powershell
.\scripts\run-dev-tool.ps1 -Release overlay-interaction-acceptance --allow-input --capture-scenario selection-transform --output-dir target/overlay-interaction-selection-transform-acceptance
```

The recording modes click `Record area` or `Record window`, then the real Pause, Resume, and Stop
buttons, and accept the run only after FFprobe validates the finalized H.264 MP4. These commands
move the global pointer, so run them only in a disposable interactive desktop session. The runner
also requires pause progress to remain frozen, compares the app's source bounds with an independent
area/window oracle, and decodes a video frame for content comparison with a desktop reference. It
forces output into its disposable session and clears inherited recording-audio overrides:

```powershell
.\scripts\run-dev-tool.ps1 -Release overlay-interaction-acceptance --allow-input --record-target area --output-dir target/overlay-recording-interaction-acceptance
.\scripts\run-dev-tool.ps1 -Release overlay-interaction-acceptance --allow-input --record-target window --output-dir target/overlay-recording-interaction-acceptance
```

On a single-display interactive desktop, run the isolated three-Pin lifecycle gate without
registering a tray icon or global shortcuts and without writing to the system clipboard. The gate
uses measured native bounds for DPI-aware layout, verifies that Show all preserves focus, and
bounds the complete native lifecycle with a watchdog:

```powershell
.\scripts\check-pin-lifecycle-acceptance.ps1
```

For a sustained three-Pin observation, opt into a 60-second soak. It rotates the production Solo
and Show all paths while continuously checking the registry, immutable source frames, native
bounds, foreground focus, and Capture preflight, and records working-set/private-commit samples.
It still injects no global input and does not write the system clipboard:

```powershell
.\scripts\check-pin-lifecycle-acceptance.ps1 -SoakMilliseconds 60000 -TimeoutMilliseconds 90000 -SettleMilliseconds 500 -OutputDirectory target\pin-lifecycle-soak-60s
```

## Documentation

- [Product requirements (Chinese)](docs/requirements.md)
- [Architecture (Chinese)](docs/architecture.md)
- [Delivery plan (Chinese)](docs/plan.md)
- [Linux platform feasibility validation (Chinese)](docs/linux-platform-validation.md)

## License

GNU Affero General Public License v3.0 only (`AGPL-3.0-only`).
