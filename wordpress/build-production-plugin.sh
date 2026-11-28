#!/bin/sh
set -eu

die() { printf '%s\n' "$1" >&2; exit 2; }
[ "$#" -eq 2 ] || die "Usage: $0 /path/to/released-v1.0.2.zip /path/to/new-v1.0.5.zip"

source_dir=$(cd -P "$(dirname "$0")" && pwd)
case "$1" in /*) baseline=$1 ;; *) baseline="$PWD/$1" ;; esac
case "$2" in /*) output=$2 ;; *) output="$PWD/$2" ;; esac
case "$output" in *.zip) ;; *) die "The output must end in .zip." ;; esac
[ -f "$baseline" ] || die "The released production installer is required."
[ ! -e "$output" ] && [ ! -L "$output" ] || die "Choose a new output filename."
expected=5d9c1c457af3cdc0e9a1aa2567b37e94d066f839e700311a399b02fa33bce550
actual=$(shasum -a 256 < "$baseline")
[ "${actual%% *}" = "$expected" ] || die "The baseline is not the exact released production importer v1.0.2."

out_dir=$(cd -P "$(dirname "$output")" && pwd)
stage=$(mktemp -d "$out_dir/.gydt-plugin.XXXXXXXX")
trap 'rm -rf "$stage"' EXIT
trap 'exit 1' HUP INT TERM
archive="$stage/candidate.zip"
plugin=gydt-sample-results-importer
UNZIP= UNZIPOPT= unzip -q "$baseline" -d "$stage"
(
    cd "$stage/$plugin"
    for patch in 01-private-spectra 02-shared-uploads 03-canonical-jpg 04-version 05-updater 06-rollover-boundary 07-native-spectra; do
        git apply --unidiff-zero "$source_dir/production-patches/$patch.patch"
    done
    cp "$source_dir/production-patches/updater.php" updater.php
    for f in acf-contract.php contract.php "$plugin.php" import-history.php import.php \
        queue.css queue.js spectrum.php submission-lock.php substances-catalog.xml updater.php zip.php; do
        [ -f "$f" ] && [ ! -L "$f" ] || die "Missing production plugin source file."
        TZ=UTC touch -t 200001010000 "$f"
    done
    [ "$(sed -n '/^ \* Version: /p' "$plugin.php")" = ' * Version: 1.0.5' ] || die "Wrong plugin version."
)
(
    cd "$stage"
    TZ=UTC ZIPOPT= zip -q -X "$archive" \
        "$plugin/acf-contract.php" "$plugin/contract.php" "$plugin/$plugin.php" \
        "$plugin/import-history.php" "$plugin/import.php" "$plugin/queue.css" "$plugin/queue.js" \
        "$plugin/spectrum.php" "$plugin/submission-lock.php" "$plugin/substances-catalog.xml" "$plugin/updater.php" "$plugin/zip.php"
)
UNZIP= UNZIPOPT= unzip -tq "$archive" >/dev/null
ln "$archive" "$output"
printf '%s\n' "$output"
