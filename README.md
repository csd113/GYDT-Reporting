# GYDT Sample Reporting

An offline desktop app for entering sample results, attaching JPEGs, and making
ZIPs for the GYDT Results workflow. The app does not upload ZIPs or send
client messages.

## Staff setup

1. Download the app for your platform from [GitHub Releases](https://github.com/csd113/GYDT-Reporting/releases) and extract it into its own folder.
2. Obtain your **GYDT Profile** folder privately from your site coordinator.
   Put that folder next to `GYDT Sample Reporting.exe` on Windows x64 or
   `GYDT Sample Reporting.app` on Apple Silicon.
3. Open the app and choose your export folder in Settings. If you are switching
   builds during an intake, confirm the next submission number and Visit
   before exporting. Existing app stores are not migrated.

The app refuses to start without a valid local profile. Keep the profile
private and out of the export folder and this repository. The app does not
read `GYDT_DATA_DIR` or reuse the previous demo or staff app folders. Keep
the app and the profile in a location accessible only to authorized staff.

Verify downloads against the release SHA-256 manifest. The desktop app is not
signed by a trusted publisher. If your verified download is blocked, see the
platform's per-app guidance without turning off system protections.
[Windows SmartScreen](https://learn.microsoft.com/windows/apps/package-and-deploy/smartscreen-reputation) ·
[macOS Open Anyway](https://support.apple.com/en-us/102445)

## Build and test

Use the pinned Rust toolchain, then run `cargo test --locked`. The committed
catalogue and draft templates are synthetic test inputs. They are not a
runtime profile. Do not replace them with private files.

Run `pwsh -File ./windows/build-app.ps1 -OutputPath ./GYDT-windows-x64.zip`
on Windows x64, or `./macos/build-app.sh ./GYDT-macos-arm64.zip` on Apple
Silicon. The resulting archives do not include a profile.

The [WordPress fixture source](wordpress/README.md) is for a disposable
local test site only. It is not a replacement for an installed staff
importer. The public synthetic importer is licensed under GPLv2 or later.
That license does not apply to the desktop app or to staff files.
