#!/bin/bash
set -exo pipefail
gen-changelog generate \
    --display-summaries \
    --name "CHANGELOG.md" \
    --package "jci-coverage" \
    --repository-dir "../.." \
    --next-version "${NEW_VERSION:-${1}}"

# Refresh the third-party license notices so every release ships current
# attribution. Runs from the crate directory, where about.toml / about.hbs
# live and where THIRD-PARTY-LICENSES.md is packaged.
#
# --locked: this runs inside cargo-release's commit window, after the version
# bump — an unlocked `cargo metadata` call here could silently rewrite
# Cargo.lock so the released commit's lockfile drifts from what CI audited
# and tested pre-release. Matches the `just licenses` invocation.
cargo about generate --locked about.hbs --output-file THIRD-PARTY-LICENSES.md
