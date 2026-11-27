# WordPress fixture prototype

For the staff Results workflow, download the [production WordPress importer v1.0.2](https://github.com/csd113/GYDT-Reporting/releases/download/v1.0.2/GYDT-Sample-Results-Importer-v1.0.2.zip) and verify it against the [release SHA-256 checksums](https://github.com/csd113/GYDT-Reporting/releases/download/v1.0.2/SHA256SUMS.txt).
New drafts use the exporting computer's timestamp, displayed in Vancouver local
time. Collection and analysis dates remain separate. The checked-out fixture
described below is for local synthetic tests only.

This is source for an importer for a synthetic and disposable WordPress test site. It is inert unless WordPress reports a local environment and the local fixture sets the Boolean `GYDT_ENABLE_SYNTHETIC_FIXTURE` opt-in. It has no production mappings or content.

The fixture requires compatible synthetic Results and Substances post types, sample ACF choices matching the source contract, and matching synthetic catalogue IDs/slugs. Use the desktop prototype and WordPress fixture with the same synthetic catalogue. Do not substitute an actual site, catalogue, or staff template into a public checkout.

To build locally from the repository root, run `sh wordpress/build-plugin.sh /path/to/new-fixture.zip`. Plugin installation and runtime are not covered by this document.

Within the enabled fixture, import uses normal Results create/edit rights. A
ZIP with a photo also requires normal WordPress media upload permission. A
successful import creates a draft with a native Featured Image. Review, editing,
publication, withdrawal and deletion use the ordinary editor and Media Library.
As with a hand-entered fixture, the original may be accessible by its direct
media URL even if the Result is not published.

The public synthetic importer is licensed under the GNU General Public
License, version 2 or later. The ZIP includes the [full license text](gydt-sample-results-importer/LICENSE.txt).
This license applies only to this importer, not to the desktop app or any
staff or production files.
