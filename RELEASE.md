# GYDT release checklist

Every GYDT release includes all three functional installers at the same
version, even when a component has no code changes. Use the tag `vX.Y.Z`
and these exact artifact names.

- `GYDT-Sample-Reporting-vX.Y.Z-macos-arm64.zip` (Apple Silicon app)
- `GYDT-Sample-Reporting-vX.Y.Z-windows-x64.zip` (one Windows app EXE with the camera helper embedded)
- `GYDT-Sample-Results-Importer-vX.Y.Z.zip` (full production WordPress plugin)
- `SHA256SUMS.txt` (one lowercase SHA-256 and two spaces before each exact ZIP name)

## Before publication

1. Build from the intended release source. Match the Cargo version, macOS
   bundle version, Windows executable version, and WordPress plugin Version.
   Use the build entry points in [README.md](README.md) and the
   [production importer instructions](wordpress/README.md). Test both native
   apps and the importer. Do not substitute a demo, QA build, source ZIP, or
   installer from an earlier release.
2. Put only the three ZIPs and checksum manifest in a fresh directory. Run
   `python3 scripts/release_preflight.py vX.Y.Z --directory /path/to/release-files`.
   This checks each archive's layout, actual platform/plugin version and
   identity, and hash. Resolve any mismatch before continuing.
3. Keep the GitHub release **draft**. Upload exactly the four checked files.
   Re-download and verify them from the draft with
   `python3 scripts/release_preflight.py vX.Y.Z --draft` using an authenticated
   GitHub CLI. If this command fails, leave the release as a draft and fix it.
   Inspect the final release notes and publish only with explicit authorization.

## After an authorized stable publication

Run `python3 scripts/release_preflight.py vX.Y.Z --published`. This uses public
download URLs without authentication and rechecks the actual archive bytes.
The release-event rollover workflow also checks the published asset inventory
and anonymously downloads the complete set before it can modify a branch.
Rollover remains conditional on the exact dev tip being fully incorporated,
unchanged, and unambiguous. If a check fails, leave the refs untouched and
investigate.

There is no automated publication job in this repository. The release-event
workflow handles human dashboard or CLI publication. If a future publishing
job uses the default `GITHUB_TOKEN`, make the same guarded rollover a dependent
step after public verification. Do not depend on a separate release event or
add credentials solely to trigger one.
