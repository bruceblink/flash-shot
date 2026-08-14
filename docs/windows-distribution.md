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

The smoke script sets `FLASH_SHOT_PROFILE_DIR` to a disposable directory and verifies that config,
data, cache, and history are initialized there. This is an isolated profile preflight, not a
substitute for manually testing on a clean Windows user account.

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

To produce an unsigned installer, install Inno Setup 6, fetch the pinned official Simplified
Chinese messages, and pass the verified absolute path to the packager:

```powershell
$chineseMessages = .\scripts\fetch-inno-language.ps1
.\scripts\package-installer.ps1 -ChineseMessagesFile $chineseMessages
```

Exercise the actual setup, installed executable, isolated profile, and uninstaller without changing
the default per-machine installation policy. The smoke script uses Inno Setup's explicit
current-user command-line override and installs only into a unique temporary directory:

```powershell
.\scripts\smoke-installer.ps1 -InstallerPath "dist\FlashShot-0.1.0-windows-setup.exe"
```

Pass `-RequireSignature` on a release machine so the setup and the executable read back from the
installation directory must both have valid Authenticode signatures.

The fetch script uses Git to read `ChineseSimplified.isl` from pinned Inno Setup source commit
`3cfb0e5632828e0dd9b49400a185834e8f1ab570`, verifies SHA-256
`e0b0b350e2245f3c5e65586dfe43d574f6e7f06f2261149aba284954b3fc9a8d`, and caches the file under
`target\inno-languages`. The translation is listed on the
[official Inno Setup translations page](https://jrsoftware.org/files/istrans/) for version 6.5.0 and
newer; the upstream [Inno Setup license](https://github.com/jrsoftware/issrc/blob/main/license.txt)
permits redistribution while retaining its notices. Pinning both commit and bytes prevents a changed
network response from silently entering a release. If the selected Inno Setup installation already
contains the same language file, `package-installer.ps1` can still use that local compiler resource
without `-ChineseMessagesFile`.

The packaging step validates the Chinese locale identity and checks every compiler-relative
`MessagesFile` before it signs the executable. A missing local translation now has an actionable path
through `fetch-inno-language.ps1`; it does not require modifying the Inno Setup installation.

To require an Authenticode signature for both the installed executable and setup program, make `signtool.exe` and a usable code-signing certificate available, then run:

```powershell
.\scripts\package-installer.ps1 -RequireSignature
```

Before building on a release machine, validate the signing prerequisites without producing or
signing an artifact:

```powershell
.\scripts\package-installer.ps1 -ValidateOnly -RequireSignature
```

The preflight requires `signtool.exe` and a currently valid code-signing certificate with a private
key in `Cert:\CurrentUser\My`. Use `-SignToolPath <absolute-path>` when SignTool is not on `PATH`,
`-CertificateThumbprint <sha1>` to select a specific certificate, and `-TimestampUrl <absolute-http-or-https-url>`
to use an approved RFC 3161 endpoint. The default is `http://timestamp.digicert.com`, which is the
endpoint used for both signature operations. The same validated certificate is then used for both
`flash-shot.exe` and the setup executable. This preflight validates the endpoint format but does not
contact the timestamp service or replace the final signature verification performed during actual packaging.
`-RequireSignature` fails instead of silently publishing an unsigned artifact. The installer does
not bundle FFmpeg.

## Release manifest

After building the ZIP and/or setup executable, generate a machine-readable manifest from the assets and their verified SHA-256 sidecars:

```powershell
.\scripts\release-manifest.ps1 -AssetDirectory dist
```

The generated `release-manifest.json` records the Cargo version, Windows platform, asset names, lengths, and SHA-256 values. Supported asset names are `FlashShot-<version>-windows-<architecture>.zip` for a portable package and `FlashShot-<version>-windows-setup.exe` for the installer. Manifest generation requires exactly one of each artifact, matching the download verifier's release contract. Before uploading assets, re-verify the unchanged directory:

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

Before triggering a release, configure these repository Actions secrets:

- `WINDOWS_SIGNING_CERTIFICATE_BASE64`: the complete base64 encoding of the production PFX;
- `WINDOWS_SIGNING_CERTIFICATE_PASSWORD`: the PFX password.

`package-github-release.ps1` imports the PFX only into the workflow user's `CurrentUser\My` store,
selects a valid private Code Signing certificate, and removes every new certificate imported there plus the
temporary PFX in `finally`. Missing, malformed, expired, or wrong-usage credentials fail before
packaging. The workflow does not provide an unsigned fallback.

The workflow runs the Rust gates and completes the Release build before importing the PFX. It then
signs `flash-shot.exe` plus the installer, creates the
portable ZIP from that same signed executable, then performs portable startup and real
current-user install/start/uninstall smoke tests on the fresh GitHub runner. It verifies both release
signatures, exact asset inventory, SHA-256 sidecars, and `release-manifest.json` before creating a
**draft** GitHub Release. Publishing the draft remains a deliberate operator action after downloading
and verifying the uploaded assets. The already published `v0.1.0` predates this required-signing gate
and remains an explicitly unsigned historical release.

## Release checks

Before publishing a draft, download and verify every uploaded asset, its SHA-256 sidecar, and `release-manifest.json`; the command also performs the portable startup preflight:

```powershell
$tag = "v0.1.1" # Replace with the actual new signed draft tag.
.\scripts\verify-github-release.ps1 -Tag $tag -RequireDraft -RequireSignature
```

With `-RequireDraft`, it also rejects a release that has already been published, keeping this check as an explicit pre-publish gate. It deletes its temporary download directory after a successful or failed check. Pass `-OutputDirectory target\release-v0.1.1` to retain the downloaded assets for manual inspection. The GitHub runner proves signed setup, install, startup, and uninstall on its fresh account; the operator still manually checks screenshot capture, annotation, save/copy, and the FFmpeg recording path before publishing the draft.
