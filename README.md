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

Download the [production WordPress importer v1.0.2](https://github.com/csd113/GYDT-Reporting/releases/download/v1.0.2/GYDT-Sample-Results-Importer-v1.0.2.zip) for the staff Results workflow.
New drafts use the exporting computer's timestamp, displayed in Vancouver local
time. Collection and analysis dates remain separate.

Verify downloads against the [release SHA-256 checksums](https://github.com/csd113/GYDT-Reporting/releases/download/v1.0.2/SHA256SUMS.txt). The desktop app is not
signed by a trusted publisher. If your verified download is blocked, see the
platform's per-app guidance without turning off system protections.
[Windows SmartScreen](https://learn.microsoft.com/windows/apps/package-and-deploy/smartscreen-reputation) ·
[macOS Open Anyway](https://support.apple.com/en-us/102445)

## Build and test

Use the pinned Rust toolchain, then run `cargo test --locked`. The approved desktop
inputs are in `data/embedded-profile/`. The separate synthetic catalogue
and templates are test fixtures. Do not add staff records or local settings
to this repository.

Run `pwsh -File ./windows/build-app.ps1 -OutputPath ./GYDT-windows-x64.zip`
on Windows x64, or `./macos/build-app.sh ./GYDT-macos-arm64.zip` on Apple
Silicon. Neither build needs a separate profile folder.

The [WordPress fixture source](wordpress/README.md) is for a disposable
local test site only. It is not a replacement for an installed staff
importer. The public synthetic importer is licensed under GPLv2 or later.
That license does not apply to the desktop app.
