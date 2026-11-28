#!/usr/bin/env python3
"""Release preflight tests. All generated binaries and metadata are synthetic."""

import hashlib
import plistlib
import struct
import tempfile
import unittest
import warnings
import zipfile
from pathlib import Path
from unittest.mock import patch

import release_preflight as gate


def pe(version=None):
    data = bytearray(320)
    data[:2] = b"MZ"
    struct.pack_into("<I", data, 60, 64)
    data[64:70] = b"PE\0\0\x64\x86"
    struct.pack_into("<HHHH", data, 70, 1, 0, 0, 0)
    struct.pack_into("<H", data, 84, 8)
    struct.pack_into("<H", data, 88, 0x20B)
    if version:
        head = "VS_VERSION_INFO\0".encode("utf-16le")
        record = bytearray(struct.pack("<HHH", 0, 52, 0) + head)
        record.extend(b"\x00" * (-len(record) % 4))
        major, minor, patch_no = map(int, version.split("."))
        a, b = major << 16 | minor, patch_no << 16
        record.extend(struct.pack("<13I", 0xFEEF04BD, 0x10000, a, b, a, b, *([0] * 7)))
        record.extend((gate.APP_EXE + "\0").encode("utf-16le"))
        struct.pack_into("<H", record, 0, len(record))
        data[96:104] = b".rsrc\0\0\0"
        struct.pack_into("<II", data, 112, len(record), 160)
        data[160:160 + len(record)] = record
    return data


class ReleaseTest(unittest.TestCase):
    TAG = "v1.0.5"

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name)
        self.mac, self.win, self.wp = gate.names(self.TAG)
        self.package()
        self.sums()

    def package(self, mac="1.0.5", win="1.0.5", wp="1.0.5", catalogue=None, profile=None, camera="bundled"):
        if profile is None:
            profile = (gate.PROFILE / "profile.json").read_bytes()
        def app_info(version):
            return ("Version " + version).encode("ascii") + profile
        with zipfile.ZipFile(self.home / self.mac, "w") as z:
            z.writestr(gate.MAC + "Info.plist", plistlib.dumps({
                "CFBundleIdentifier": "com.getyourdrugstested.sample-reporting.profile",
                "CFBundleDisplayName": gate.APP, "CFBundleExecutable": "gydt-results-compiler",
                "CFBundleVersion": mac, "CFBundleShortVersionString": mac,
            }))
            z.writestr(gate.MAC + "MacOS/gydt-results-compiler", bytes.fromhex("cffaedfe0c000001") + app_info(mac))
            z.writestr(gate.MAC + "Resources/AppIcon.icns", b"synthetic")
        with zipfile.ZipFile(self.home / self.win, "w") as z:
            helper = bytes(pe())
            embedded = b""
            if camera in ("bundled", "tampered"):
                expected = hashlib.sha256(helper).hexdigest().encode("ascii")
                if camera == "tampered":
                    helper = helper[:-1] + b"x"
                embedded = gate.CAMERA_RECORD + struct.pack("<I", len(helper)) + expected + helper
            z.writestr(gate.APP_EXE, pe(win) + app_info(win) + embedded)
            if camera == "loose":
                z.writestr("GYDT-camera.exe", helper)
        approved = Path(gate.__file__).resolve().parents[1] / "data/embedded-profile/substances-catalog.xml"
        with zipfile.ZipFile(self.home / self.wp, "w") as z:
            for f in gate.PLUGIN_FILES:
                value = {
                    gate.PLUGIN + ".php": f"<?php\n * Plugin Name: GYDT Sample Results Importer\n * Version: {wp}\n * Update URI: {gate.ROOT}\n",
                    "substances-catalog.xml": approved.read_bytes() if catalogue is None else catalogue,
                }.get(f, "synthetic")
                z.writestr(gate.PLUGIN + "/" + f, value)

    def sums(self):
        path = self.home / gate.MANIFEST
        path.write_text("".join(
            f"{hashlib.sha256((self.home / n).read_bytes()).hexdigest()}  {n}\n" for n in (self.mac, self.win, self.wp)
        ))
        return path

    def test_complete(self):
        gate.check(self.home, self.TAG)

    def test_legacy_1_0_4_has_side_by_side_helper(self):
        self.package(win="1.0.4", camera="loose")
        gate.check_zip(self.home / self.win, 1, "1.0.4")

    def test_reject_missing_tampered_or_loose_helper_for_1_0_5(self):
        for kind in ("missing", "tampered", "loose"):
            with self.subTest(camera=kind), self.assertRaises(gate.Hold):
                self.package(camera=kind)
                self.sums()
                gate.check(self.home, self.TAG)

    def test_missing_asset(self):
        (self.home / self.win).unlink()
        with self.assertRaises(gate.Hold):
            gate.check(self.home, self.TAG)

    def test_extra_demo_asset(self):
        (self.home / "demo.zip").write_bytes(b"synthetic")
        with self.assertRaises(gate.Hold):
            gate.check(self.home, self.TAG)

    def test_mismatched_metadata_even_with_current_hashes(self):
        for part in ("mac", "win", "wp"):
            with self.subTest(package=part), self.assertRaises(gate.Hold):
                self.package(**{part: "1.0.6"})
                self.sums()
                gate.check(self.home, self.TAG)

    def test_duplicate_manifest_row(self):
        manifest = self.sums()
        original = manifest.read_text()
        manifest.write_text(original + original.splitlines()[0] + "\n")
        with self.assertRaises(gate.Hold):
            gate.check(self.home, self.TAG)

    def test_duplicate_member(self):
        with warnings.catch_warnings(), zipfile.ZipFile(self.home / self.win, "a") as z:
            warnings.simplefilter("ignore", UserWarning)
            z.writestr(gate.APP_EXE, pe("1.0.5"))
        self.sums()
        with self.assertRaises(gate.Hold):
            gate.check(self.home, self.TAG)

    def test_hash_mismatch(self):
        (self.home / self.win).write_bytes(b"changed")
        with self.assertRaises(gate.Hold):
            gate.check(self.home, self.TAG)

    def test_qa_profile_rejected(self):
        self.package(profile=b'{"profile_id":"disposable-qa"}')
        self.sums()
        with self.assertRaises(gate.Hold):
            gate.check(self.home, self.TAG)

    def test_synthetic_catalogue_rejected(self):
        self.package(catalogue=b"demo catalogue")
        self.sums()
        with self.assertRaises(gate.Hold):
            gate.check(self.home, self.TAG)

    def test_published_event_requires_complete_stable_set_before_download(self):
        assets = [{"name": name} for name in (*gate.names(self.TAG), gate.MANIFEST)]
        event = {"action": "published", "repository": {"full_name": "csd113/GYDT-Reporting"},
                 "release": {"tag_name": self.TAG, "draft": False, "prerelease": False, "assets": assets}}
        with patch.object(gate, "check_published") as download:
            gate.check_published_event(event)
            download.assert_called_once_with(self.TAG)
            download.reset_mock()
            for overrides in ({"assets": assets[:-1]}, {"draft": True}, {"prerelease": True}):
                with self.assertRaises(gate.Hold):
                    gate.check_published_event(event | {"release": event["release"] | overrides})
            download.assert_not_called()


if __name__ == "__main__":
    unittest.main()
