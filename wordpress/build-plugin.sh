#!/bin/sh
set -eu

if [ "$#" -ne 1 ]; then
    printf 'Usage: %s path/to/plugin.zip\n' "$0" >&2
    exit 2
fi

source_dir=$(cd "$(dirname "$0")" && pwd)
"$source_dir/../scripts/check-synthetic-inputs.sh"
case "$1" in
    /*) output=$1 ;;
    *) output="$PWD/$1" ;;
esac
if [ -e "$output" ] || [ -L "$output" ]; then
    printf 'The output path already exists. Choose a new filename.\n' >&2
    exit 2
fi

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
plugin=gydt-sample-results-importer
mkdir "$stage/$plugin"
set --
for name in LICENSE.txt THIRD-PARTY-NOTICES.txt acf-contract.php contract.php \
    gydt-sample-results-importer.php import-history.php import.php pilot.php \
    queue.css queue.js submission-lock.php zip.php; do
    cp "$source_dir/$plugin/$name" "$stage/$plugin/$name"
    set -- "$@" "$plugin/$name"
done
cp "$source_dir/../data/substances-catalog.xml" "$stage/$plugin/substances-catalog.xml"
(cd "$stage" && ZIPOPT= zip -q -X "$stage/plugin.zip" "$@" "$plugin/substances-catalog.xml")
(set -C; cat "$stage/plugin.zip" > "$output")
