# GYDT Sample Reporting

An offline desktop app for entering sample results, attaching JPEGs, and making
ZIPs for the GYDT Results workflow. The app does not upload ZIPs or send
client messages.

## Get started

1. Download and extract the ZIP for your platform from [GitHub Releases](https://github.com/csd113/GYDT-Reporting/releases).
2. Open `GYDT Sample Reporting.exe` on Windows x64 or
   `GYDT Sample Reporting.app` on Apple Silicon. The approved profile is
   built in. There is no separate profile file to install.
3. Choose your export folder in Settings if you haven't already set it.

Version 1.0.2 uses the same local store as 1.0.1 when it used the approved
staff profile. Keep the app and exported results in locations accessible
only to authorized staff.

Download the matching production WordPress importer for the staff Results workflow from [GitHub Releases](https://github.com/csd113/GYDT-Reporting/releases).
New drafts use the exporting computer's timestamp, displayed in Vancouver local
time. Collection and analysis dates remain separate.

Verify downloads against the [release SHA-256 checksums](https://github.com/csd113/GYDT-Reporting/releases/download/v1.0.2/SHA256SUMS.txt). The desktop app is not
signed by a trusted publisher. If your verified download is blocked, see the
platform's per-app guidance without turning off system protections.
[Windows SmartScreen](https://learn.microsoft.com/windows/apps/package-and-deploy/smartscreen-reputation) ·
[macOS Open Anyway](https://support.apple.com/en-us/102445)

### Optional OPUS spectra

In Settings, choose an OPUS folder and turn on **Attach OPUS spectrum to
exports**. This is off by default, and the folder choice is remembered either way.
Export looks for the exact finalized submission number followed by `.0` and
copies the same bytes into the ZIP and beside it. Use a separate export folder.
If OPUS denies access, unload the named file and retry. A file can still export
while open if the operating system allows the app to snapshot it.

The current WordPress importer v1.0.2 rejects ZIPs containing a spectrum.
Leave this setting off for WordPress uploads until the importer supports it.

## Build and test

Use the pinned Rust toolchain, then run `cargo test --locked`. The approved desktop
inputs are in `data/embedded-profile/`. The separate synthetic catalogue
and templates are test fixtures. Do not add staff records or local settings
to this repository.

Run `pwsh -File ./windows/build-app.ps1 -OutputPath ./GYDT-windows-x64.zip`
on Windows x64, or `./macos/build-app.sh ./GYDT-macos-arm64.zip` on Apple
Silicon. Neither build needs a separate profile folder.

See the [production WordPress importer instructions](wordpress/README.md).
