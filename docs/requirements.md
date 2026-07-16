# Product Requirements

## 1. Product intent

Flash Shot is a native desktop capture tool optimized for the short path from intent to result:

1. invoke a global shortcut;
2. acquire the correct pixels immediately;
3. select a window, control, display, or free region;
4. annotate without visible input latency;
5. copy, save, pin, recognize, or record the result reliably.

Snow Shot is a source of validated workflows and edge cases, not a codebase or internal format
that Flash Shot must reproduce. Flash Shot must not inherit Tauri, WebView, React, or Excalidraw
as runtime dependencies.

## 2. Users and primary scenarios

Primary users are developers, designers, support staff, educators, and other desktop power users
who capture and communicate visual information many times per day.

The highest-frequency scenarios are:

- capture a free region and copy it;
- select a window or UI control automatically;
- add arrows, shapes, text, blur, highlight, or sequence numbers;
- pin a capture above other windows;
- revisit recent captures;
- extract text or a QR code;
- capture a scrolling surface;
- record a display, window, or region with optional audio.

## 3. Functional scope

### 3.1 Capture

- Global configurable shortcuts.
- Display, window, UI-control, and free-region selection.
- Correct behavior with multiple displays, negative origins, and mixed DPI.
- Magnifier, pixel color, dimensions, cursor-aware selection, and keyboard adjustment.
- Copy to clipboard, save to file, quick save, and configurable output naming.
- Optional cursor inclusion and capture delay.

### 3.2 Annotation

- Selection, rectangle, ellipse, line, arrow, freehand, and text.
- Blur/mosaic, highlight, watermark, and sequence-number tools.
- Stroke, fill, width, font, opacity, and layer controls.
- Move, resize, rotate, duplicate, delete, undo, and redo.
- An engine-neutral document model with a versioned serialization format.
- Pixel-correct export independent of viewport scale.

### 3.3 Productivity

- Pin one or more images as lightweight always-on-top windows.
- Capture history with explicit retention and privacy controls.
- OCR, QR recognition, and optional translation providers.
- Open an existing local image in the annotation editor.

### 3.4 Advanced capture

- Scrolling screenshot with manual and assisted capture modes.
- Screen recording backed initially by an isolated FFmpeg process.
- Display, window, and region recording with microphone and system audio where supported.
- Pause, resume, stop, progress reporting, and recoverable process failure.

### 3.5 Desktop integration

- Tray menu, single instance, optional startup, notifications, and updater.
- Portable and installed configurations.
- Structured logs and opt-in crash diagnostics without captured-image leakage.

## 4. Non-functional requirements

### 4.1 Performance budgets

Initial Windows targets on a representative modern desktop:

| Metric | Target |
| --- | --- |
| Warm shortcut to usable overlay | p95 <= 100 ms |
| Cold process start to ready | p95 <= 500 ms |
| Idle CPU | effectively 0%, excluding explicit background work |
| Idle working set | <= 80 MiB before optional OCR models are loaded |
| 4K annotation interaction | p95 frame time <= 16.7 ms |
| Simple capture copied to clipboard | p95 <= 250 ms from selection commit |
| Repeated 100-capture resource growth | no unbounded handles, threads, textures, or memory |

Budgets must be measured in automated or repeatable benchmarks. They are hypotheses until the
first platform spike records a baseline.

### 4.2 Stability

- No panic or process termination for recoverable platform, I/O, model, or FFmpeg errors.
- Capture and recording sessions use explicit state machines and deterministic cleanup.
- Background work never blocks the GPUI foreground executor.
- External processes belong to a lifecycle boundary and cannot remain orphaned after exit.
- Persisted settings and documents use versioned schemas and safe migrations.

### 4.3 Privacy and security

- Captured pixels remain local unless the user invokes an explicitly networked feature.
- History is optional, visible, bounded, and clearable.
- Logs never contain image data, recognized text, tokens, or private file contents.
- Translation and update traffic clearly identifies its provider and failure behavior.

## 5. Platform strategy

Windows is the first production platform. Use Windows Graphics Capture/Desktop Duplication,
Windows UI Automation, native clipboard and window APIs behind platform traits. macOS follows
with ScreenCaptureKit and Accessibility APIs. Linux is evaluated after the Windows workflow is
stable because Wayland portal and compositor behavior require a separate product plan.

## 6. Explicit non-goals for the first release

- General-purpose collaborative whiteboarding.
- Excalidraw document compatibility.
- Browser or mobile versions.
- Plugin APIs before the core capture workflow is stable.
- Full feature parity across all operating systems on day one.

## 7. MVP acceptance

The first usable release must allow a Windows user to invoke a shortcut, select a correct region
across mixed-DPI displays, annotate it with core tools, undo/redo, and copy or save a pixel-correct
PNG. It must meet measured latency and repeated-use resource checks before advanced features are
declared complete.
