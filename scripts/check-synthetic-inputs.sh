#!/bin/sh
set -eu

cd "$(dirname "$0")/.."
expected='data/substances-catalog.xml
templates/results-email.eml
templates/results-sms.txt'
if [ "$(awk 'NF == 2 {print $2}' scripts/synthetic-inputs.sha256)" != "$expected" ] ||
   ! shasum -a 256 -c scripts/synthetic-inputs.sha256 >/dev/null; then
    printf '%s\n' 'Only the reviewed synthetic catalogue and draft templates can be packaged here.' >&2
    exit 1
fi
