#!/bin/sh
set -eu

[ "$#" -eq 1 ] || { printf 'Usage: %s path/to/production-installer.zip\n' "$0" >&2; exit 2; }
test_dir=$(cd -P "$(dirname "$0")" && pwd)
stage=$(mktemp -d "${TMPDIR:-/tmp}/gydt-plugin-check.XXXXXXXX")
trap 'rm -rf "$stage"' EXIT
trap 'exit 1' HUP INT TERM
UNZIP= UNZIPOPT= unzip -q "$1" -d "$stage"
plugin="$stage/gydt-sample-results-importer"
[ "$(find "$plugin" -type f | wc -l | tr -d ' ')" -eq 12 ]
[ "$(sed -n '/^ \* Version: /p' "$plugin/gydt-sample-results-importer.php")" = ' * Version: 1.0.5' ]
for f in "$plugin/"*.php; do php -l "$f"; done
node --check "$plugin/queue.js"
python3 -B "$test_dir/make-production-fixtures.py" "$stage" "$plugin/substances-catalog.xml"
php "$test_dir/check-production-package.php" "$plugin" "$stage"
php "$test_dir/check-acf-contract.php" "$plugin"
php "$test_dir/check-native-spectra.php" "$plugin"
php "$test_dir/check-import-history.php" "$plugin"
for case in timeout ambient external-after-lock clear state-unknown rollback-failed dirty-release; do
    php "$test_dir/check-submission-lock.php" "$case" "$plugin"
done
