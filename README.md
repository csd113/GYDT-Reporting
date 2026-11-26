# GYDT Sample Reporting

An offline Rust app for entering sample results and producing versioned ZIP exports.
The public demo uses fictional catalogues, site identifiers and message templates.
Use it with synthetic data only. It does not send messages or connect to a live
WordPress site. The [illustrated demo](docs/demo/index.html) shows the sample form.

## Downloads

Get the ZIP for your computer from [GitHub Releases](https://github.com/csd113/GYDT-Reporting/releases).
Extract it before running the app.

- **Windows x64** — open `GYDT Sample Reporting Demo.exe`.
- **macOS on Apple Silicon** — open `GYDT Sample Reporting Demo.app`.
- **WordPress** — the separately labeled local/synthetic importer is only for a
  disposable local test site. It is not the staff importer and is not compatible
  with the live site without separately maintained private configuration.
  See the [fixture requirements](wordpress/README.md).

The public executables are not signed by a trusted publisher. If your system
blocks a verified download, check the ZIP's SHA-256 against the release's
checksums before considering your system's per-app option. On macOS, see
[Apple's Open Anyway instructions](https://support.apple.com/en-us/102445).
On Windows, see
[Microsoft's guidance on SmartScreen reputation](https://learn.microsoft.com/windows/apps/package-and-deploy/smartscreen-reputation).
Do not turn off system protections. If your device policy disallows an
exception, leave the app blocked.

The demo always starts in its own per-user **GYDT Sample Reporting Demo** data
folder. It ignores an inherited `GYDT_DATA_DIR` and does not open or migrate
staff records. Export only synthetic samples to a folder you choose.

## Build from source

Use the pinned Rust toolchain. Run `cargo test --locked` to test the ordinary
build and `cargo test --locked --all-features` to also exercise the demo mode.

On Windows x64, run
`pwsh -File ./windows/build-app.ps1 -OutputPath ./GYDT-windows-x64-demo.zip`.
On Apple Silicon, run `./macos/build-app.sh ./GYDT-macos-arm64-demo.zip`.
Both packagers select demo mode and check the reviewed synthetic catalogue and
draft templates. The automated validation jobs do not upload the packages.

Staff builds use a separate private profile outside Git. Keep staff templates,
catalogues, site mappings, local records and binaries out of this checkout.
