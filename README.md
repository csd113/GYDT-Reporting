# GYDT Sample Reporting

An offline Rust prototype for entering sample results and producing versioned ZIP exports. The UI is illustrated in the [synthetic demo](docs/demo/index.html). The bundled catalogue, location identifiers, and message drafts are fictional test fixtures. Do not use this build to prepare or deliver real results.

This repository contains no production deployments, templates, or database exports. The WordPress prototype and any exports built from the example fixtures are for a disposable local test instance only. Do not install the example importer on a real site.

Build with the pinned Rust toolchain using `cargo build --locked`. Run `cargo test --locked` for automated checks. See the [local WordPress fixture notes](wordpress/README.md) for the plugin.

## Synthetic desktop packages

Use the same pinned toolchain for local validation. On Windows x64, run
`pwsh -File ./windows/build-app.ps1 -OutputPath ./GYDT-windows-x64-synthetic.zip`.
On Apple Silicon, run `./macos/build-app.sh ./GYDT-macos-arm64-synthetic.zip`.
Each packager checks the reviewed synthetic catalogue and draft template hashes
and includes only application files. Automated validation builds both packages
without publishing or uploading either.

Staff builds use a separate private profile outside Git. Keep staff templates,
catalogues, site mappings, local records and binaries out of this checkout.
Do not substitute those inputs into this project's synthetic package workflow.
