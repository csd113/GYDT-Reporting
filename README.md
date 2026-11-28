# GYDT Sample Reporting

An offline desktop app for entering sample results, attaching JPEGs, and making
ZIPs for the GYDT Results workflow. The app does not upload ZIPs or send
client messages.

## Get started

1. Download and extract the ZIP for your platform from [GitHub Releases](https://github.com/csd113/GYDT-Reporting/releases).
2. Open `GYDT Sample Reporting.exe` on Windows x64 or
   `GYDT Sample Reporting.app` on Apple Silicon. The approved profile is
   built in. There is no separate profile file to install.
3. Choose your export folder for the current Vancouver work day. The app
   remembers it until the next work day, when you must confirm a folder again.

Version 1.0.2 uses the same local store as 1.0.1 when it used the approved
staff profile. Keep the app and exported results in locations accessible
only to authorized staff.

Download the matching production WordPress importer and SHA-256 checksums
from [GitHub Releases](https://github.com/csd113/GYDT-Reporting/releases).
New drafts use the exporting computer's timestamp, displayed in Vancouver local
time. Collection and analysis dates remain separate.

The desktop app is not
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

WordPress uploads with OPUS require production importer v1.0.4 or newer.
Starting with v1.0.5, new imports fill the existing **Spectra File** field and
follow the same access rules as manually uploaded media.
Leave OPUS off if the site still runs importer v1.0.2.

### Optional webcam on Windows

Turn on **Enable webcam capture** in Settings. Activate the sample photo box
or choose **Take Image** in the new-sample prompt to open the live preview.
The app starts your saved camera or the only available color camera. To choose
or switch cameras, use **Camera** next to **Enable webcam capture** in Settings.
The app won't switch to a different camera if your saved choice is disconnected.
Reconnect it and retry, or select a different camera in Settings. **Use photo** attaches
the captured JPEG, while **Cancel** leaves any existing photo unchanged.
**Browse JPG…** and drag-and-drop still work without starting the camera.
The video is not recorded or uploaded. The Windows ZIP contains
one app executable. On first use, the app installs its bundled camera helper
inside its local data folder, in the `camera` subfolder. It does not use an old
helper beside the app. No webcam setting appears on macOS.
To build the Windows ZIP, install the Visual C++ x64 tools and Windows SDK.

## Build and test

Use the pinned Rust toolchain, then run `cargo test --locked`. The approved desktop
inputs are in `data/embedded-profile/`. The separate synthetic catalogue
and templates are test fixtures. Do not add staff records or local settings
to this repository.

Run `pwsh -File ./windows/build-app.ps1 -OutputPath ./GYDT-windows-x64.zip`
on Windows x64, or `./macos/build-app.sh ./GYDT-macos-arm64.zip` on Apple
Silicon. Neither build needs a separate profile folder.

See the [production WordPress importer instructions](wordpress/README.md)
and the [release checklist](RELEASE.md).
