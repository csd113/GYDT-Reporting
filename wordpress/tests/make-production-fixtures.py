"""Generate scratch-only JPEG and OPUS package fixtures without any sample data."""

import hashlib
import json
import struct
import sys
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile


def jpeg():
    """A baseline JPEG of one procedurally generated mid-gray pixel."""
    def segment(marker, content):
        return bytes([0xFF, marker]) + struct.pack(">H", len(content) + 2) + content

    huffman = bytes([1, *([0] * 16)])  # One one-bit symbol, with value zero.
    return b"".join((
        b"\xff\xd8",
        segment(0xDB, bytes([0] + [1] * 64)),
        segment(0xC0, bytes.fromhex("08 0001 0001 01 01 11 00")),
        segment(0xC4, bytes([0]) + huffman + bytes([0x10]) + huffman),
        segment(0xDA, bytes.fromhex("01 01 00 00 3f 00")),
        b"\x3f\xff\xd9",
    ))


def main(directory, catalogue_path):
    catalogue = "sha256:" + hashlib.sha256(catalogue_path.read_bytes()).hexdigest()
    for number, opus in enumerate((False, True), start=1):
        title = f"Synthetic-import-check-{number:03}"
        payload = {
            "submission_number": title, "visit": 1, "client": "SYNTHETIC-TEST",
            "collection_date": "2020-01-01", "analysis_date": "2020-01-01",
            "site": "gydt", "expected_substance": "unknown",
            "results": [{"substance": "unknown", "percentage": "blank"}],
            "colour": "White", "texture": "Powder", "pre_post_consumption": "n/a",
            "adverse_events": "na",
            **dict.fromkeys(("fent_strip", "benzo", "medetomidine", "lysergamide_strip"), "Not Used"),
        }
        files = {
            "test_data.json": json.dumps(payload).encode("ascii"),
            title + (".jpg" if opus else ".JPEG"): jpeg(),
        }
        if opus:
            files[title + ".0"] = b"\x00synthetic-opaque-check\xff"
        manifest = {
            "format_version": 1, "catalogue_version": catalogue, "exported_at": "2020-01-01T15:00:00Z",
            "sample_id": f"00000000-0000-4000-8000-{number:012}",
            "files": {name: {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}
                      for name, data in files.items()},
        }
        name = "synthetic-opus-and-jpg.zip" if opus else "synthetic-legacy-jpg.zip"
        with ZipFile(directory / name, "x", ZIP_DEFLATED) as archive:
            archive.writestr("manifest.json", json.dumps(manifest).encode("ascii"))
            for member, data in files.items():
                archive.writestr(member, data)


if __name__ == "__main__":
    main(*map(Path, sys.argv[1:]))
