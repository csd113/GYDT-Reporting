# GYDT Sample Reporting

An offline Rust app for entering sample results and producing versioned ZIP exports.
The public demo uses fictional catalogues, site identifiers and message templates.
Use it with synthetic data only. It does not send messages or connect to a live
WordPress site. The [illustrated demo](docs/demo/index.html) shows the sample form.

## Download status

The v1.0.0 demo downloads have been withdrawn. No supported public download
is available yet. The WordPress fixture source is for disposable local
testing only and is not a staff installer.

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
