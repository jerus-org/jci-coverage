#!/usr/bin/env bash
# Run scripts/check-otterwise-drift.sh and, when OtterWise's uploader has
# changed, open one GitHub issue about it, then fail.
#
# Usage: scripts/report-otterwise-drift.sh [script-file-or-url]
# (the argument is passed to the check). Meant for the scheduled CircleCI
# pipeline of .circleci/otterwise_drift.yml.
#
# Needs: bash, curl, jq. Environment:
#   GITHUB_TOKEN                      token that may create issues (from the
#                                     bot-check context); without it the run
#                                     fails and says so, with no issue opened
#   CIRCLE_PROJECT_USERNAME/_REPONAME the repository to open the issue in
#   CIRCLE_BUILD_URL                  linked from the issue (optional)
#   GITHUB_API_URL                    API base, default https://api.github.com
#                                     (a test points it at a local stub)
#
# Exit status: 0 no drift; 1 drift, or the check could not run, or the issue
# could not be opened. An issue is opened only for a change the check itself
# reports (exit 1: the fields differ; exit 2: it found no fields at all), not
# for a failed download, and not again while one for the same change is open.
set -uo pipefail

here="$(dirname "$0")"

output="$("$here/check-otterwise-drift.sh" "$@" 2>&1)"
status=$?
printf '%s\n' "$output"

[ "$status" -eq 0 ] && exit 0

case "$status" in
    1) title="OtterWise uploader: the form fields differ from the manifest" ;;
    2) title="OtterWise uploader: no form fields found" ;;
    *)
        echo "The check could not run (exit $status); not opening an issue." >&2
        exit 1
        ;;
esac

if [ -z "${GITHUB_TOKEN:-}" ]; then
    echo "GITHUB_TOKEN is not set; not opening an issue." >&2
    exit 1
fi
repo="${CIRCLE_PROJECT_USERNAME:-}/${CIRCLE_PROJECT_REPONAME:-}"
if [ "$repo" = "/" ]; then
    echo "CIRCLE_PROJECT_USERNAME/CIRCLE_PROJECT_REPONAME are not set; not opening an issue." >&2
    exit 1
fi
api="${GITHUB_API_URL:-https://api.github.com}/repos/$repo/issues"

gh_curl() {
    curl --fail --silent --show-error --location \
        --header "Authorization: Bearer $GITHUB_TOKEN" \
        --header "Accept: application/vnd.github+json" \
        "$@"
}

# One open issue per change is enough: while it is open the run still fails, and
# nothing is added to it. (The issues list also returns pull requests, which
# have a `pull_request` key.)
existing="$(gh_curl "$api?state=open&per_page=100" \
    | jq -r --arg t "$title" \
        '[.[] | select(.pull_request == null and .title == $t)][0].number // empty')" \
    || { echo "Could not list the open issues; not opening one." >&2; exit 1; }
if [ -n "$existing" ]; then
    echo "Issue #$existing already reports this; not opening another."
    exit 1
fi

build="${CIRCLE_BUILD_URL:-}"
body="$(cat <<EOF
The scheduled check found that OtterWise's uploader (https://otterwise.app/bash) no longer matches \`crates/jci-coverage/tests/otterwise_reference_fields.txt\`.

\`\`\`
$output
\`\`\`

To resolve it, review what changed in the uploader, update the fields \`jci-coverage\` sends and the \`KNOWN_UNSENT\` list in \`crates/jci-coverage/src/upload/otterwise.rs\`, then update the manifest and its "Last reviewed" date. \`scripts/check-otterwise-drift.sh\` shows the difference.

${build:+Found by $build}
EOF
)"

payload="$(jq -n --arg title "$title" --arg body "$body" '{title: $title, body: $body}')"
created="$(gh_curl --request POST --data "$payload" "$api" | jq -r '.html_url // empty')" \
    || { echo "Could not open the issue." >&2; exit 1; }
echo "Opened ${created:-an issue}."
exit 1
