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

## Release checks

Before publishing a portable package, run the repository validation gates, build the release package, verify the checksum, and manually smoke-test the extracted executable on a clean Windows profile. Code signing and installer production are separate release steps; an unsigned package must not be represented as signed.
