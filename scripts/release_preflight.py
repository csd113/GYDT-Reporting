#!/usr/bin/env python3
"""Validate the complete GYDT release set without writing to GitHub."""

import argparse
import hashlib
import json
import plistlib
import re
import struct
import subprocess
import tempfile
import urllib.error
import urllib.parse
import urllib.request
import zipfile
from pathlib import Path

TAG = re.compile(r"v(0|[1-9][0-9]{0,4})\.(0|[1-9][0-9]{0,4})\.(0|[1-9][0-9]{0,4})")
ROOT = "https://github.com/csd113/GYDT-Reporting"
MANIFEST = "SHA256SUMS.txt"
MAX_BYTES = 64 * 1024 * 1024
APP = "GYDT Sample Reporting"
APP_EXE = APP + ".exe"
CAMERA_RECORD = b"GYDT-CAMERA-BUNDLE-1\0"
MAC = APP + ".app/Contents/"
PLUGIN = "gydt-sample-results-importer"
PROFILE = Path(__file__).resolve().parents[1] / "data/embedded-profile"
PLUGIN_FILES = {
    "acf-contract.php", "contract.php", PLUGIN + ".php", "import-history.php", "import.php",
    "queue.css", "queue.js", "spectrum.php", "submission-lock.php", "substances-catalog.xml",
    "updater.php", "zip.php",
}


class Hold(ValueError):
    """A release asset is missing or does not match the release."""


def require(condition, detail):
    if not condition:
        raise Hold(detail)


def names(tag):
    match = TAG.fullmatch(tag)
    require(match is not None and all(int(i) < 65536 for i in match.groups()), "Use a stable vMAJOR.MINOR.PATCH tag.")
    return (
        f"GYDT-Sample-Reporting-{tag}-macos-arm64.zip",
        f"GYDT-Sample-Reporting-{tag}-windows-x64.zip",
        f"GYDT-Sample-Results-Importer-{tag}.zip",
    )


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def pe_offset(data):
    require(data[:2] == b"MZ" and len(data) >= 64, "Expected a Windows PE executable.")
    offset, = struct.unpack_from("<I", data, 60)
    require(offset + 26 <= len(data) and data[offset:offset + 6] == b"PE\0\0\x64\x86"
            and data[offset + 24:offset + 26] == b"\x0b\x02", "Expected an x64 PE32+ executable.")
    return offset


def windows_version(data, version):
    pe = pe_offset(data)
    count, = struct.unpack_from("<H", data, pe + 6)
    optional_size, = struct.unpack_from("<H", data, pe + 20)
    start = pe + 24 + optional_size
    require(0 < count <= 96 and start + count * 40 <= len(data), "Invalid Windows sections.")
    resources = []
    for i in range(count):
        off = start + i * 40
        if data[off:off + 8] != b".rsrc\0\0\0":
            continue
        size, ptr = struct.unpack_from("<II", data, off + 16)
        require(ptr + size <= len(data), "Invalid Windows resource section.")
        resources.append(data[ptr:ptr + size])
    require(len(resources) == 1, "The Windows app has no version resource.")
    resource = resources[0]
    key = "VS_VERSION_INFO\0".encode("utf-16le")
    pos = resource.find(key)
    require(pos >= 6 and resource.find(key, pos + len(key)) == -1, "Ambiguous Windows version resource.")
    length, value, kind = struct.unpack_from("<HHH", resource, pos - 6)
    end = pos - 6 + length
    fixed = (pos + len(key) + 3) & ~3
    require(value == 52 and kind == 0 and fixed + 52 <= end <= len(resource), "Invalid Windows version record.")
    signature, structure, file_ms, file_ls, product_ms, product_ls = struct.unpack_from("<6I", resource, fixed)
    major, minor, patch = map(int, version.split("."))
    target = (major << 16 | minor, patch << 16)
    require(signature == 0xFEEF04BD and structure == 0x00010000
            and (file_ms, file_ls) == target and (product_ms, product_ls) == target
            and (APP_EXE + "\0").encode("utf-16le") in resource[fixed:end],
            "Windows executable identity or version does not match.")


def functional_app(binary, version):
    require((PROFILE / "profile.json").read_bytes() in binary
            and ("Version " + version).encode("ascii") in binary,
            "Desktop binary is missing the approved profile or matching app version.")


def macos(z, version):
    plist = plistlib.loads(z.read(MAC + "Info.plist"))
    require(plist.get("CFBundleIdentifier") == "com.getyourdrugstested.sample-reporting.profile"
            and plist.get("CFBundleDisplayName") == APP
            and plist.get("CFBundleExecutable") == "gydt-results-compiler"
            and plist.get("CFBundleVersion") == plist.get("CFBundleShortVersionString") == version,
            "macOS bundle identity or version does not match.")
    binary = z.read(MAC + "MacOS/gydt-results-compiler")
    require(binary[:8] == bytes.fromhex("cffaedfe0c000001"), "Expected a native Apple Silicon Mach-O.")
    functional_app(binary, version)


def wordpress(z, version):
    main = z.read(PLUGIN + "/" + PLUGIN + ".php")
    fields = {
        b"Plugin Name": b"GYDT Sample Results Importer",
        b"Version": version.encode("ascii"),
        b"Update URI": ROOT.encode("ascii"),
    }
    for field, expected in fields.items():
        pattern = rb"^\s*\* " + field + rb": ([^\r\n]+)\r?$"
        require(re.findall(pattern, main[:8192], re.M) == [expected], "Production importer identity or version does not match.")
    approved = PROFILE / "substances-catalog.xml"
    catalogue = hashlib.sha256(z.read(PLUGIN + "/substances-catalog.xml")).hexdigest()
    require(catalogue == sha(approved) and b"GYDT_ENABLE_SYNTHETIC_FIXTURE" not in main,
            "Importer does not contain the approved functional catalogue.")


def camera_payload(binary):
    """Find and verify the helper PE stored in the app, not loose ZIP bytes."""
    valid = []
    offset = 0
    while (offset := binary.find(CAMERA_RECORD, offset)) != -1:
        offset += len(CAMERA_RECORD)
        size_raw = binary[offset:offset + 4]
        expected = binary[offset + 4:offset + 68]
        if len(size_raw) != 4 or not re.fullmatch(rb"[a-f0-9]{64}", expected):
            continue
        size = int.from_bytes(size_raw, "little")
        if not 64 <= size <= 8 * 1024 * 1024 or offset + 68 + size > len(binary):
            continue
        helper = binary[offset + 68:offset + 68 + size]
        if hashlib.sha256(helper).hexdigest().encode("ascii") == expected:
            pe_offset(helper)
            valid.append(helper)
    require(len(valid) == 1, "Windows app must contain exactly one verified camera helper.")
    return valid[0]


def check_zip(path, kind, version):
    expected = [
        {MAC + name for name in ("Info.plist", "MacOS/gydt-results-compiler", "Resources/AppIcon.icns")},
        {APP_EXE, "GYDT-camera.exe"} if version == "1.0.4" else {APP_EXE},
        {PLUGIN + "/" + name for name in PLUGIN_FILES},
    ][kind]
    with zipfile.ZipFile(path) as z:
        members = [item for item in z.infolist() if not item.is_dir()]
        require(len(members) == len(expected) and {i.filename for i in members} == expected,
                f"Wrong or duplicate members in {path.name}.")
        allowed_dirs = {str(parent).replace("\\", "/") + "/" for name in expected for parent in Path(name).parents} - {"./"}
        require(all(not i.is_dir() or i.filename in allowed_dirs for i in z.infolist()), "Unexpected ZIP directory.")
        require(all(((i.external_attr >> 16) & 0o170000) != 0o120000 for i in z.infolist())
                and 0 < sum(i.file_size for i in members) <= MAX_BYTES and z.testzip() is None,
                f"Invalid ZIP members in {path.name}.")
        if kind == 0:
            macos(z, version)
        elif kind == 1:
            binary = z.read(APP_EXE)
            windows_version(binary, version)
            functional_app(binary, version)
            if version == "1.0.4":
                pe_offset(z.read("GYDT-camera.exe"))
            else:
                camera_payload(binary)
        else:
            wordpress(z, version)


def check(directory, tag):
    archives = names(tag)
    wanted = {*archives, MANIFEST}
    require({path.name for path in directory.iterdir()} == wanted, "Expected only three exact release ZIPs and SHA256SUMS.txt.")
    for name in wanted:
        path = directory / name
        require(path.is_file() and not path.is_symlink() and 0 < path.stat().st_size <= MAX_BYTES,
                f"Invalid release asset {name}.")
    manifest = directory / MANIFEST
    require(manifest.stat().st_size <= 4096, "Invalid checksum manifest size.")
    hashes = {}
    for line in manifest.read_text(encoding="ascii").splitlines():
        match = re.fullmatch(r"([a-f0-9]{64})  ([A-Za-z0-9._-]+)", line)
        require(match is not None, "Invalid checksum manifest row.")
        digest, name = match.groups()
        require(name in archives and name not in hashes, "Unexpected or duplicate checksum row.")
        hashes[name] = digest
    require(set(hashes) == set(archives), "Checksum manifest is incomplete.")
    for kind, name in enumerate(archives):
        path = directory / name
        require(sha(path) == hashes[name], f"SHA-256 mismatch for {name}.")
        check_zip(path, kind, tag[1:])


class GitHubRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        target = urllib.parse.urlsplit(newurl)
        require(target.scheme == "https" and target.netloc in {
            "github.com", "release-assets.githubusercontent.com", "objects.githubusercontent.com",
        }, "Release asset redirected away from GitHub.")
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def check_inventory(assets, tag):
    expected = {*names(tag), MANIFEST}
    actual = [asset.get("name") for asset in assets]
    require(len(actual) == len(expected) and set(actual) == expected, "Missing, duplicate or unexpected GitHub release assets.")


def check_draft(tag):
    names(tag)
    repo = ("--repo", "csd113/GYDT-Reporting")
    details = json.loads(subprocess.check_output(
        ["gh", "release", "view", tag, *repo, "--json", "tagName,isDraft,isPrerelease,assets"], text=True, timeout=30
    ))
    require(details.get("tagName") == tag and details.get("isDraft") is True
            and details.get("isPrerelease") is False, "The exact stable release must still be a draft.")
    check_inventory(details.get("assets", []), tag)
    with tempfile.TemporaryDirectory(prefix="gydt-draft-") as temporary:
        subprocess.run(["gh", "release", "download", tag, *repo, "--dir", temporary], check=True, timeout=120)
        check(Path(temporary), tag)


def check_published_event(event):
    release = event.get("release", {})
    require(event.get("action") == "published" and event.get("repository", {}).get("full_name") == "csd113/GYDT-Reporting"
            and release.get("draft") is False and release.get("prerelease") is False,
            "Not a stable published GYDT release event.")
    tag = release.get("tag_name", "")
    check_inventory(release.get("assets", []), tag)
    check_published(tag)


def check_published(tag):
    assets = (*names(tag), MANIFEST)
    client = urllib.request.build_opener(GitHubRedirect())
    with tempfile.TemporaryDirectory(prefix="gydt-release-") as temporary:
        directory = Path(temporary)
        for name in assets:
            with client.open(f"{ROOT}/releases/download/{tag}/{name}", timeout=20) as response, (directory / name).open("xb") as file:
                remaining = 4096 if name == MANIFEST else MAX_BYTES
                while chunk := response.read(128 * 1024):
                    remaining -= len(chunk)
                    require(remaining >= 0, "Public release asset exceeds its size limit.")
                    file.write(chunk)
        check(directory, tag)


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("tag")
    where = cli.add_mutually_exclusive_group(required=True)
    where.add_argument("--directory", type=Path)
    where.add_argument("--published", action="store_true")
    where.add_argument("--draft", action="store_true")
    args = cli.parse_args()
    try:
        if args.published:
            check_published(args.tag)
        elif args.draft:
            check_draft(args.tag)
        else:
            check(args.directory, args.tag)
    except (Hold, OSError, UnicodeError, RuntimeError, zipfile.BadZipFile, plistlib.InvalidFileException,
            subprocess.SubprocessError) as error:
        cli.exit(1, f"Release blocked. {error}\n")
    print(f"Verified all three functional {args.tag} archives and their SHA-256 manifest.")


if __name__ == "__main__":
    main()
