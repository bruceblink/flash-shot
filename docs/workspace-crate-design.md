# Workspace Crate Migration Design

Updated: 2026-08-16

## Terminology And Naming

| Chinese name | English / abbreviation | Responsibility boundary | It is not |
| --- | --- | --- | --- |
| Workspace root | Cargo workspace root | Coordinates shared package metadata, locked dependencies, default package selection, and repository-wide checks. | A runnable application package or a second executable entry point. |
| Domain crate | `flash-shot-domain` | Owns screenshot product value types, geometry, selection, capture-session state, and annotation documents. | A GPUI view, a Windows API wrapper, or an image capture implementation. |
| Application crate | `flash-shot-app` | Owns use cases, persistent product policy, and interfaces consumed by UI and infrastructure. | A Windows service implementation or a GPUI entity. |
| Windows infrastructure crate | `flash-shot-infra-windows` | Implements Windows capture, clipboard, shortcuts, tray, inspection, process, and file-system boundaries. | The application composition root or a reusable domain model. |
| UI crate | `flash-shot-ui` | Owns GPUI state, overlays, Pin windows, settings views, and presentation-only interaction code. | The place where Windows services are constructed. |
| Acceptance crate | `flash-shot-acceptance` | Provides opt-in native acceptance and stress runners as library code called by the main executable. | An independently published or user-facing executable. |
| Binary crate | `flash-shot-bin` | Composes concrete services, starts the application, and embeds Windows resources. | A home for domain rules, use cases, or alternate command-line programs. |

All subsequent diagrams and text use these exact names. Standard Cargo, GPUI, Windows, and FFmpeg names retain their standard capitalization.

## Decision

Adopt a small Cargo workspace modeled on Ramag's composition pattern, not its number of feature crates. Flash Shot already has a stable `src/domain/` boundary, platform-facing service types, and a single desktop startup path. The current largest units (`src/app/overlay.rs` and the overlay interaction acceptance runner) are large enough that package-level ownership will make future changes easier to review and test.

The migration must preserve these product contracts:

1. The workspace contains exactly one binary target, named `flash-shot`.
2. The virtual workspace root declares `default-members = ["crates/flash-shot-bin"]`, so `cargo run` from the repository root launches that binary without `--bin flash-shot`.
3. All reusable layers, including native acceptance runners, are library crates. Development tools continue to dispatch through the one binary with the existing opt-in feature and environment selector.
4. No behavior, persisted settings schema, release asset name, or Windows integration contract changes merely because source files move.
5. The dependency graph is directional. The binary is the only composition root.

```text
flash-shot-bin
  -> flash-shot-ui
  -> flash-shot-app
  -> flash-shot-domain

flash-shot-bin
  -> flash-shot-infra-windows
  -> flash-shot-app

flash-shot-acceptance
  -> flash-shot-ui
  -> flash-shot-app
  -> flash-shot-domain
  -> flash-shot-infra-windows
```

The Windows infrastructure crate supplies concrete implementations to the binary crate. The binary crate passes application interfaces into the UI crate; the UI crate must not construct `SystemClipboard`, global shortcuts, the tray, or Windows capture services by itself.

## Why This Fits

The existing architecture documentation already distinguishes domain, platform, application workflow, and GPUI rendering. The domain modules depend only on standard-library and serialization types, which makes them ready for the first crate extraction. Conversely, directly splitting every existing `platform` module would be premature because several files currently mix contracts with Windows implementations. The staged plan below separates those responsibilities before moving them.

Ramag is useful here because its `ramag-domain`, `ramag-app`, infrastructure libraries, UI library, and `ramag-bin` make the composition root explicit. Flash Shot should retain that benefit without introducing one crate for each screenshot feature. Capture, Pin, OCR, recording, and history share one short-lived desktop workflow and should not become separate packages until a real dependency or release boundary appears.

## Migration Stages

### Stage 1: Establish The Workspace And Domain Crate

Create the virtual workspace, centralize compatible dependency versions, extract `src/domain/` into `flash-shot-domain`, and update every caller to use the new library. Keep the existing behavior unchanged. This is the first implementation slice because it has no GPUI or Windows API dependency.

Validation: domain tests, repository-wide formatting, strict Clippy, full workspace tests, and a root-level development-tool dispatch that proves Cargo selected the only executable without `--bin`.

### Stage 2: Define Application Interfaces

Move product-policy modules into `flash-shot-app` only after their platform-facing contracts are explicit. Capture-frame data, image composition, persistent settings, history policy, recording state, OCR, translation, update, and scroll workflows must depend on interfaces rather than `System*` implementations.

Validation: existing unit and golden-image tests remain in their owning crate; settings compatibility and managed-history behavior receive cross-crate regression coverage.

### Stage 3: Extract Windows Infrastructure

Move concrete Win32, clipboard, shortcut, tray, capture, display, and process implementations into `flash-shot-infra-windows`. Keep shared traits and product errors in `flash-shot-app` or `flash-shot-domain`, depending on whether they represent a use-case contract or a value type.

Validation: platform contract tests, existing Windows-only tests, and release acceptance scripts must remain green. No test may use a production process merely because a trait moved packages.

### Stage 4: Extract UI And Acceptance Libraries

Move GPUI rendering, overlay state, Pin windows, settings views, and UI-facing workflows into `flash-shot-ui`. Move the optional native acceptance and stress runners into `flash-shot-acceptance`; it stays a library enabled by the binary's `dev-tools` feature.

Validation: GPUI unit tests, native screenshot fixtures, and all existing command-line argument contracts stay unchanged.

### Stage 5: Create The Single Binary Composition Root

Move `main.rs`, `build.rs`, and Windows resource embedding to `flash-shot-bin`. Keep `flash-shot` as its sole `[[bin]]` target. Set the workspace default member and update packaging, release, and developer scripts only where Cargo package selection requires it.

Validation: `cargo run` from the workspace root, `cargo run --features dev-tools` through the existing dispatcher, Release build, packaging checks, full workspace tests, and strict Clippy.

## Non-Goals

- Do not create a binary for each stress or acceptance runner.
- Do not change GPUI, FFmpeg, Windows API, release, or installer dependencies during this migration.
- Do not turn crate names into user-facing product terminology.
- Do not perform a mechanical all-at-once move that leaves circular dependencies or weakens existing acceptance evidence.
