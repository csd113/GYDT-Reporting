# Production WordPress importer

The importer ZIP is separate from the desktop app. Download the matching
installer and checksum from [GYDT Sample Reporting Releases](https://github.com/csd113/GYDT-Reporting/releases).
Desktop v1.0.4 requires production importer v1.0.4 to upload ZIPs containing
OPUS spectra. Older ZIPs without spectra remain supported.

## Build and validate a candidate locally

Download the [released production importer v1.0.2](https://github.com/csd113/GYDT-Reporting/releases/download/v1.0.2/GYDT-Sample-Results-Importer-v1.0.2.zip).
The builder verifies the exact release checksum, applies the reviewed patches
in order, and writes a new ZIP. It does not publish or install anything.

```sh
./wordpress/build-production-plugin.sh /path/to/GYDT-Sample-Results-Importer-v1.0.2.zip /path/to/GYDT-Sample-Results-Importer-v1.0.4.zip
./wordpress/tests/check-production-plugin.sh /path/to/GYDT-Sample-Results-Importer-v1.0.4.zip
```

The package check requires PHP CLI with mysqli and Node.js. Full upload,
permission, and publishing checks belong on a local WordPress clone with
synthetic data. Never add site backups, staff records, or credentials to this
repository.
