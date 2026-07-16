# Architecture

## Principles

1. GPUI is the presentation and interaction layer, not the domain model.
2. Platform APIs live behind narrow traits; platform details never leak into documents.
3. Captured pixels have explicit ownership and are not encoded or copied without a reason.
4. Long-running work uses background executors and cancellation-aware tasks.
5. Every user workflow is an explicit state machine with observable failure states.
6. Saved documents and settings are versioned from their first release.

## Engineering baseline

The application shell reuses selected proven conventions from the sibling `synchub-desktop` and
`hiposter` projects:

- the published `gpui` crate instead of a WebView or separate windowing stack;
- a Tokio runtime entered before the GPUI application loop;
- `build.rs` and a resources directory for native icons and packaging metadata;
- thin `main.rs`, application wiring in `lib.rs`, and feature state in focused modules.

Flash Shot intentionally does not depend on `gpui-component` or `gpui-component-assets`. Its
screenshot overlay and annotation controls need direct ownership of layout, input, painting,
focus, and frame behavior; a general component suite would add an unnecessary upgrade and styling
boundary. Reusable controls will remain small, product-specific GPUI modules.

The capture and annotation domain remains independent so that UI structure does not become a
platform dependency.

The baseline pins `gpui` and the official `gpui_platform` launcher to the same reviewed Zed commit.
`gpui_platform` is part of the current GPUI workspace and only constructs the native platform
implementation; it is not a widget or styling dependency. Upgrading the pin is an explicit
milestone with compile, interaction, and performance validation rather than an unreviewed moving
Git dependency.

## Intended workspace

The initial single crate will split only when working code establishes stable boundaries:

```text
flash-shot-app             GPUI composition and application lifecycle
flash-shot-core            capture sessions and application use cases
flash-shot-annotation      document, commands, geometry, hit testing, history
flash-shot-image           crop, filter, composite, encode, color conversion
flash-shot-platform        platform traits
flash-shot-platform-win    Windows capture, input, clipboard, tray, UI Automation
flash-shot-scroll          scrolling capture matching and composition
flash-shot-ocr             OCR provider abstraction and local implementation
flash-shot-recording       FFmpeg process and recording state machine
```

Premature crate splitting is avoided, but dependency direction is enforced immediately:

```text
GPUI app -> use cases -> domain/core <- platform implementations
                         |
                         +-> image/annotation algorithms
```

Core crates must not depend on GPUI, HWND, COM objects, FFmpeg processes, or a specific OCR
runtime.

## Capture pipeline

```text
global shortcut
  -> capture session
  -> acquire display frames
  -> upload/cache immutable textures
  -> overlay selection and annotation preview
  -> deterministic image composition
  -> clipboard/file/pin/OCR destination
```

The first technical spike must count CPU copies, GPU uploads, allocations, and frame lifetime.
A single screenshot upload is acceptable; uploading or decoding the full screenshot on every
frame is not.

## Annotation model

Annotation data uses logical image coordinates and stable IDs. Rendering converts document
coordinates to viewport coordinates. Export operates against original pixel dimensions rather
than taking a screenshot of the UI.

Operations are commands with inverses or before/after state sufficient for undo/redo. Pointer
movement may produce transient previews, but only committed operations enter history.

## Platform boundary

Candidate traits include:

- `CaptureBackend`
- `DisplayProvider`
- `GlobalShortcutService`
- `ClipboardService`
- `TrayService`
- `WindowInspector`
- `AutoStartService`
- `RecordingBackend`

Traits describe product operations and errors, not one-for-one wrappers around OS APIs.

## FFmpeg boundary

The first recording backend launches a bundled or user-selected FFmpeg executable. It owns:

- capability and device probing;
- argument construction;
- stdout/stderr draining and progress parsing;
- graceful finalize with bounded forced termination;
- Windows job-object/process-group cleanup;
- a typed idle/starting/recording/paused/stopping/failed state machine.

Direct libav integration is deferred until measurements show the process boundary is a material
limitation.

## Testing strategy

- Pure unit tests for geometry, documents, commands, state machines, naming, and configuration.
- Golden-image tests for composition and annotation output.
- Platform contract tests for coordinate conversion and resource cleanup.
- GPUI interaction tests for selection and toolbar behavior where practical.
- Manual mixed-DPI matrix until equivalent automation is available.
- Repeatable latency, frame-time, working-set, handle, and texture benchmarks.
