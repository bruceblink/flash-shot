# Windows Distribution

## Portable package

Build a versioned, unsigned portable ZIP from a Windows MSVC Rust host:

```powershell
.\scripts\package-portable.ps1
```

The output is written to `dist\FlashShot-<version>-windows-<architecture>.zip` with a matching `.sha256` file. The archive contains `flash-shot.exe`, `LICENSE.txt`, `README.md`, and `PORTABLE.txt`.
The packaging script verifies the SHA-256 sidecar and this exact archive layout before reporting success. Re-check an existing archive independently with:

```powershell
.\scripts\verify-portable-package.ps1 -ArchivePath "dist\FlashShot-0.1.0-windows-x86_64.zip"
```

Before distributing a portable ZIP, run its release executable from a fresh temporary extraction and require it to stay alive for five seconds:

```powershell
.\scripts\smoke-portable-startup.ps1 -ArchivePath "dist\FlashShot-0.1.0-windows-x86_64.zip"
```

This is an artifact-startup preflight, not a substitute for manually testing on a clean Windows user profile.

The package intentionally does not include FFmpeg. Recording users must install a compatible FFmpeg build or set `FLASH_SHOT_FFMPEG` to its executable path. This keeps the application license boundary and FFmpeg distribution choice explicit.

Use `-SkipBuild` only after producing the matching release executable for the current Rust host target:

```powershell
.\scripts\package-portable.ps1 -SkipBuild
```

## Installer

The project includes an Inno Setup 6 installer definition for a standard per-machine Windows install, including a Start menu shortcut and uninstaller. Validate the definition without installing Inno Setup or building an executable:

```powershell
.\scripts\package-installer.ps1 -ValidateOnly
```

To produce an unsigned installer, install Inno Setup 6 and run:

```powershell
.\scripts\package-installer.ps1
```

To require an Authenticode signature for both the installed executable and setup program, make `signtool.exe` and a usable code-signing certificate available, then run:

```powershell
.\scripts\package-installer.ps1 -RequireSignature
```

`-RequireSignature` fails instead of silently publishing an unsigned artifact. The installer does not bundle FFmpeg.

## Release manifest

After building the ZIP and/or setup executable, generate a machine-readable manifest from the assets and their verified SHA-256 sidecars:

```powershell
.\scripts\release-manifest.ps1 -AssetDirectory dist
```

The generated `release-manifest.json` records the Cargo version, Windows platform, asset names, lengths, and SHA-256 values. Before uploading assets, re-verify the unchanged directory:

```powershell
.\scripts\release-manifest.ps1 -AssetDirectory dist -VerifyOnly
```

The tool rejects missing sidecars, malformed checksums, version-mismatched filenames, changed assets, and changed manifests.

## Manual update check

Flash Shot never downloads or installs updates. To enable the optional `Check Updates` button, configure an HTTPS endpoint that serves the generated `release-manifest.json`:

```powershell
$env:FLASH_SHOT_UPDATE_ENDPOINT = "https://releases.example.com/flash-shot/release-manifest.json"
```

The application makes no update network request until the user clicks the button. It accepts only schema-version-1 Windows manifests with nonempty, version-matched ZIP or EXE assets and valid SHA-256 metadata. The result tells the user whether a newer release exists and directs them to their configured release channel; downloading and installation remain manual.

## GitHub release workflow

The repository packages a Windows release when a `v<major>.<minor>.<patch>` tag is pushed, or when the `Release` workflow is manually run for an existing tag. The tag must exactly match the Cargo package version; for example, `Cargo.toml` version `0.1.0` requires tag `v0.1.0`:

```powershell
git tag v0.1.0
git push origin v0.1.0
```

The workflow runs the Rust gates, builds the unsigned portable ZIP and Inno Setup executable, generates and re-verifies `release-manifest.json`, then creates a **draft** GitHub Release with the ZIP, installer, their SHA-256 sidecars, and manifest. Publishing the draft is a deliberate operator action after clean-profile smoke testing. Releases are unsigned unless the packaging process has been separately extended with an available code-signing certificate; an unsigned release must remain identified as such.

## Release checks

Before publishing a draft, download the uploaded artifacts, re-check their SHA-256 sidecars, run the portable startup preflight, and manually smoke-test the portable ZIP and installer on a clean Windows profile. Check screenshot capture, annotation, save/copy, and the FFmpeg recording path when a compatible FFmpeg build is available. Code signing and installer production are separate release steps; an unsigned package must not be represented as signed.
