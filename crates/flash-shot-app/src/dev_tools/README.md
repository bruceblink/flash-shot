# Flash Shot Development Tools

## Real recording failure recovery

Run the recording retry scenario from a disposable, interactive Windows desktop with one display at
100% scaling:

```powershell
.\scripts\run-dev-tool.ps1 -Release overlay-interaction-acceptance `
  --allow-input --capture-scenario recording-failure-retry `
  --output-dir target\overlay-interaction-acceptance\recording-failure-retry
```

The scenario opens the real Record page, starts one opt-in recording startup failure, verifies the
failure message and cleared lifecycle, then clicks the same primary action to retry. It completes
Pause, Resume, and Stop, and uses FFprobe plus a decoded-frame comparison to verify the finalized
H.264 MP4. The run does not access the system clipboard. Its `report.json` and screenshots are
written below the generated `session-<timestamp>-<pid>` directory.

The existing area and window recording flows remain separate:

```powershell
.\scripts\run-dev-tool.ps1 -Release overlay-interaction-acceptance `
  --allow-input --record-target area `
  --output-dir target\overlay-recording-interaction\area
.\scripts\run-dev-tool.ps1 -Release overlay-interaction-acceptance `
  --allow-input --record-target window `
  --output-dir target\overlay-recording-interaction\window
```

The failure injection is enabled only by the `dev-tools` feature and is consumed once per process.
It does not change normal application recording behavior.

## Settings interaction acceptance

`settings-interaction-acceptance` uses real Windows mouse and keyboard input against the settings
navigation shell. It clicks Capture, Library, Record, and App, then walks the same destinations
with the layout-appropriate arrow key. Each step is read back through the GPUI command channel and
stores a native screenshot plus `report.json`; input is disabled unless `--allow-input` is present.

```powershell
.\scripts\run-dev-tool.ps1 -Release settings-interaction-acceptance `
  --allow-input --output-dir target\settings-interaction-acceptance
```

Run it only in a disposable Windows desktop session. The report proves settings-section state and
input cleanup; it does not cover the screenshot overlay's Copy/Save/Pin workflow.
