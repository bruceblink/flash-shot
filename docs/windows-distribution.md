# Windows Distribution

## Portable package

Build a versioned, unsigned portable ZIP from a Windows MSVC Rust host:

```powershell
.\scripts\package-portable.ps1
```

The output is written to `dist\FlashShot-<version>-windows-<architecture>.zip` with a matching `.sha256` file. The archive contains `flash-shot.exe`, `LICENSE.txt`, `README.md`, and `PORTABLE.txt`.

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

## Release checks

Before publishing a portable package or installer, run the repository validation gates, verify the checksum, and manually smoke-test it on a clean Windows profile. Code signing and installer production are separate release steps; an unsigned package must not be represented as signed.
