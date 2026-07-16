# Delivery Plan

Each numbered item is a bounded feature milestone. A milestone is committed only after its own
validation passes.

## Milestone 0: Foundation

- [x] Create `bruceblink/flash-shot` with a runnable Rust + GPUI shell.
- [x] Record product requirements, architecture boundaries, and performance budgets.
- [x] Add formatting, compile, lint, test, and CI gates.
- [x] Pin current official GPUI sources without a third-party component framework.
- [ ] Add structured tracing, panic reporting, and application directories.
- [ ] Add a benchmark harness and machine-readable performance report.
- [ ] Implement single-instance behavior and controlled shutdown.

Exit criteria: the shell starts reliably, CI is green, failures are diagnosable, and performance
measurements can be recorded before the capture implementation begins.

## Milestone 1: Windows capture spike

- Enumerate displays with physical bounds, scale, rotation, and color metadata.
- Capture an immutable frame for each display.
- Open one borderless overlay per display with correct physical/logical mapping.
- Measure shortcut-to-overlay latency and screenshot texture upload behavior.
- Prove mixed-DPI selection across negative and positive display coordinates.

Exit criteria: a technical report demonstrates a viable GPUI rendering path and identifies every
pixel copy. If public GPUI APIs cannot meet the target, decide between a narrow GPUI extension and
a dedicated overlay renderer before adding product UI.

## Milestone 2: Capture MVP

- Global shortcut, tray, and capture-session lifecycle.
- Region selection, resize handles, keyboard adjustment, magnifier, and dimensions.
- Window and control detection through Windows UI Automation.
- Copy PNG, save PNG, cancel, and deterministic cleanup.
- Repeated-capture soak test and latency metrics.

Exit criteria: the complete capture-to-copy workflow is useful without annotation and meets the
agreed performance baseline.

## Milestone 3: Native annotation

- Versioned `AnnotationDocument` and command history.
- Selection, rectangle, ellipse, line, arrow, and freehand.
- Text editing with IME and mixed-language validation.
- Blur/mosaic, highlight, watermark, and sequence numbers.
- Layer ordering, style controls, hit testing, transform handles, undo, and redo.
- Pixel-correct CPU/GPU composition with golden-image tests.

Exit criteria: 4K editing remains responsive and exported output matches the document model.

## Milestone 4: Productivity workflows

- Pinned image windows.
- Local capture history and retention controls.
- Open and annotate an existing image.
- QR recognition.
- Local OCR with lazy model loading.
- Optional translation provider boundary.

Exit criteria: optional models and history do not degrade startup, idle resource use, or privacy.

## Milestone 5: Scrolling screenshots

- Extract the useful image matching and composition ideas from Snow Shot without Tauri types.
- Manual scroll capture, overlap detection, mismatch recovery, preview, and export.
- Assisted scrolling where platform behavior is reliable.
- Long-page memory and correctness tests.

## Milestone 6: Screen recording

- Isolated FFmpeg discovery and capability probing.
- Display/window/region recording.
- Microphone and supported system-audio selection.
- Pause, resume, graceful finalize, progress, and failure recovery.
- Job-object/process-group cleanup and orphan-process tests.

## Milestone 7: Distribution and additional platforms

- Signed Windows installer, portable build, updater, and release verification.
- macOS capture/platform implementation and permission UX.
- Linux feasibility spike for Wayland portals and X11 before committing to parity.

## Risk register

| Risk | Early mitigation |
| --- | --- |
| GPUI external texture limitations | Milestone 1 copy/upload spike before editor development |
| GPUI API churn | Keep domain/platform crates GPUI-free and pin reviewed releases |
| Mixed-DPI coordinate errors | Use physical-pixel canonical coordinates and a hardware matrix |
| Native text editing complexity | Validate GPUI IME early, before advanced annotation tools |
| Feature-parity scope growth | Require milestone exit criteria and defer low-frequency options |
| OCR runtime size/startup cost | Lazy-load an optional provider outside the core process path |
| FFmpeg process/file corruption | Typed lifecycle, graceful finalize, timeout, and job cleanup |

## Environment notes

- The first Windows smoke launch on 2026-07-16 opened a responsive `Flash Shot` window, but GPUI
  logged DirectX error `0x887A002D` indicating a missing or mismatched Windows SDK/graphics
  component on that development machine. Compilation and tests pass; GPU rendering must be
  visually verified after repairing the local DirectX/SDK environment and before Milestone 0 is
  closed.
