# Production WordPress importer

The importer ZIP is separate from the desktop app. Download the matching
installer and checksum from [GYDT Sample Reporting Releases](https://github.com/csd113/GYDT-Reporting/releases).
Desktop v1.0.4 or newer requires production importer v1.0.4 or newer to upload ZIPs containing
OPUS spectra. Older ZIPs without spectra remain supported.

Starting with v1.0.5, new OPUS imports use the existing **Spectra File** field on the Test
Result and appear in the Media Library like manual uploads. They are not
staff-only by default. If the site allows direct access to media URLs, a
spectrum link can work even while its Result is a draft. The importer does not
add file types to the site’s allowlist or give anyone upload permission.
For spectra previously imported by v1.0.4, the **Legacy OPUS spectrum** box
keeps the existing staff-only download available on those Results only.

## Upgrade the existing importer

Leave v1.0.2 installed and active. On WordPress 5.5 or newer, use
**Plugins → Add New Plugin → Upload Plugin**, select the verified production
v1.0.6 ZIP, then choose **Replace current with uploaded**. Do not deactivate or
delete the installed plugin first. The replacement uses the same plugin folder.

After installing v1.0.4 or newer, on WordPress 5.8 or newer, an administrator can use
**Plugins → Installed Plugins → Update now** for later stable GYDT releases.
The importer checks the public GYDT GitHub repository and accepts only the
exact production ZIP and its verified release SHA-256. This does not enable
automatic updates. Sites that block outbound GitHub access can keep using the
verified manual ZIP replacement. An unavailable update service does not stop
imports.

## Build and validate a candidate locally

Download the [released production importer v1.0.2](https://github.com/csd113/GYDT-Reporting/releases/download/v1.0.2/GYDT-Sample-Results-Importer-v1.0.2.zip).
The builder verifies the exact release checksum, applies the reviewed patches
in order, and writes a new ZIP. It does not publish or install anything.

```sh
./wordpress/build-production-plugin.sh /path/to/GYDT-Sample-Results-Importer-v1.0.2.zip /path/to/GYDT-Sample-Results-Importer-v1.0.6.zip
./wordpress/tests/check-production-plugin.sh /path/to/GYDT-Sample-Results-Importer-v1.0.6.zip
```

The package check requires PHP CLI with mysqli and Node.js. Full upload,
permission, and publishing checks belong on a local WordPress clone with
synthetic data. Never add site backups, staff records, or credentials to this
repository.
