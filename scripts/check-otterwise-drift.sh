#!/usr/bin/env bash
# Fail if OtterWise's bash uploader posts a different set of form fields than
# crates/jci-coverage/tests/otterwise_reference_fields.txt records.
#
# Usage: scripts/check-otterwise-drift.sh [script-file-or-url]
# Default source is https://otterwise.app/bash. On drift, review the uploader's
# change, update jci-coverage's fields (and the `KNOWN_UNSENT` list in
# `upload/otterwise.rs`), then update the manifest and its review date.
set -euo pipefail

source_arg="${1:-https://otterwise.app/bash}"
manifest="$(dirname "$0")/../crates/jci-coverage/tests/otterwise_reference_fields.txt"

if [ -f "$source_arg" ]; then
    script="$(cat "$source_arg")"
else
    script="$(curl --fail --silent --show-error --location --retry 3 "$source_arg")"
fi

# `-F name=` / `-F name=@file` arguments of the uploader's curl calls.
live="$(grep -oE -- '-F [a-z_]+=' <<<"$script" | sed -E 's/^-F //; s/=$//' | sort -u)"
[ -n "$live" ] || { echo "no form fields found in $source_arg" >&2; exit 2; }

recorded="$(grep -vE '^\s*(#|$)' "$manifest" | sort -u)"

if diff <(echo "$recorded") <(echo "$live") >/dev/null; then
    echo "OtterWise uploader fields match the manifest ($(wc -l <<<"$live") fields)."
else
    echo "OtterWise uploader fields differ from the manifest (< recorded, > live):" >&2
    diff <(echo "$recorded") <(echo "$live") >&2 || true
    exit 1
fi
