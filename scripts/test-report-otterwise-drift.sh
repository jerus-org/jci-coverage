#!/usr/bin/env bash
# Tests for scripts/report-otterwise-drift.sh against a local stub of the
# GitHub issues API. Needs: bash, curl, jq, python3.
#
# Usage: scripts/test-report-otterwise-drift.sh
set -uo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'kill "${server_pid:-0}" 2>/dev/null; rm -rf "$work"' EXIT

manifest="$here/../crates/jci-coverage/tests/otterwise_reference_fields.txt"

# The stub answers GET .../issues with the JSON in $work/issues.json and records
# every POST body, one JSON object per line, in $work/posts.jsonl. A POST gets
# the status in $work/post_status (default 201).
cat >"$work/stub.py" <<'PY'
import http.server, json, os, sys

work = sys.argv[1]

class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def reply(self, status, body):
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        with open(os.path.join(work, "gets.log"), "a") as f:
            f.write(self.path + " auth=" + str(self.headers.get("Authorization")) + "\n")
        with open(os.path.join(work, "issues.json")) as f:
            self.reply(200, json.load(f))

    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length).decode()
        with open(os.path.join(work, "posts.jsonl"), "a") as f:
            f.write(body.replace("\n", " ") + "\n")
        status_file = os.path.join(work, "post_status")
        status = int(open(status_file).read()) if os.path.exists(status_file) else 201
        self.reply(status, {"html_url": "https://github.com/o/r/issues/1"})

server = http.server.HTTPServer(("127.0.0.1", 0), Handler)
with open(os.path.join(work, "port"), "w") as f:
    f.write(str(server.server_port))
server.serve_forever()
PY
python3 "$work/stub.py" "$work" &
server_pid=$!
for _ in $(seq 50); do [ -s "$work/port" ] && break; sleep 0.1; done
port="$(cat "$work/port")"

failures=0
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }
pass() { echo "ok:   $*"; }

# An uploader script posting exactly the fields of the manifest.
matching="$work/matching.sh"
grep -vE '^\s*(#|$)' "$manifest" | sed -E 's/^(.*)$/curl -F \1=x/' >"$matching"

reset() {
    rm -f "$work/posts.jsonl" "$work/gets.log" "$work/post_status"
    echo '[]' >"$work/issues.json"
}

# run_case NAME EXPECTED_STATUS [script args] -- runs the reporter and checks the exit status.
run() {
    GITHUB_TOKEN="${TOKEN-t0ken}" CIRCLE_PROJECT_USERNAME=o CIRCLE_PROJECT_REPONAME=r \
        CIRCLE_BUILD_URL=https://circleci.example/build/1 \
        GITHUB_API_URL="http://127.0.0.1:$port" \
        "$here/report-otterwise-drift.sh" "$@" >"$work/out.txt" 2>&1
}
posts() { [ -f "$work/posts.jsonl" ] && wc -l <"$work/posts.jsonl" | tr -d ' ' || echo 0; }

# 1. No drift: success, the API is not touched.
reset
run "$matching"; status=$?
[ "$status" -eq 0 ] && [ ! -f "$work/gets.log" ] && [ "$(posts)" = 0 ] \
    && pass "no drift: exit 0, no API call" || fail "no drift (status $status)"

# 2. Drift: one issue is opened, naming the difference, and the run fails.
reset
{ cat "$matching"; echo 'curl -F brand_new_field=x'; } >"$work/drifted.sh"
run "$work/drifted.sh"; status=$?
if [ "$status" -eq 1 ] && [ "$(posts)" = 1 ] \
    && jq -e '.title == "OtterWise uploader: the form fields differ from the manifest"' "$work/posts.jsonl" >/dev/null \
    && jq -e '.body | contains("brand_new_field") and contains("https://circleci.example/build/1")' "$work/posts.jsonl" >/dev/null \
    && grep -q 'auth=Bearer t0ken' "$work/gets.log"; then
    pass "drift: exit 1, one issue naming the field and the build"
else
    fail "drift (status $status, posts $(posts))"; cat "$work/out.txt"
fi

# 3. An open issue with that title: nothing more is opened, the run still fails.
reset
echo '[{"number": 7, "title": "OtterWise uploader: the form fields differ from the manifest"}]' >"$work/issues.json"
run "$work/drifted.sh"; status=$?
[ "$status" -eq 1 ] && [ "$(posts)" = 0 ] && grep -q '#7' "$work/out.txt" \
    && pass "open issue: exit 1, no second issue" || fail "open issue (status $status, posts $(posts))"

# 4. A pull request with that title is not an issue about it.
reset
echo '[{"number": 8, "pull_request": {}, "title": "OtterWise uploader: the form fields differ from the manifest"}]' >"$work/issues.json"
run "$work/drifted.sh"; status=$?
[ "$status" -eq 1 ] && [ "$(posts)" = 1 ] \
    && pass "a pull request with the title does not count" || fail "pull request title (status $status, posts $(posts))"

# 5. An issue with another title does not count.
reset
echo '[{"number": 9, "title": "something else"}]' >"$work/issues.json"
run "$work/drifted.sh"; status=$?
[ "$status" -eq 1 ] && [ "$(posts)" = 1 ] \
    && pass "an unrelated open issue does not count" || fail "unrelated issue (status $status, posts $(posts))"

# 6. No form fields at all: its own issue.
reset
echo 'echo no uploads here' >"$work/empty.sh"
run "$work/empty.sh"; status=$?
[ "$status" -eq 1 ] && [ "$(posts)" = 1 ] \
    && jq -e '.title == "OtterWise uploader: no form fields found"' "$work/posts.jsonl" >/dev/null \
    && pass "no fields: exit 1, its own issue" || fail "no fields (status $status, posts $(posts))"

# 7. The script cannot be fetched: fails, but no issue about a change.
reset
run "http://127.0.0.1:1/nothing"; status=$?
[ "$status" -ne 0 ] && [ "$(posts)" = 0 ] && grep -q 'could not run' "$work/out.txt" \
    && pass "fetch failure: fails, no issue" || fail "fetch failure (status $status, posts $(posts))"

# 8. No token: fails and says so, with no API call.
reset
TOKEN='' run "$work/drifted.sh"; status=$?
[ "$status" -eq 1 ] && [ ! -f "$work/gets.log" ] && grep -q 'GITHUB_TOKEN is not set' "$work/out.txt" \
    && pass "no token: exit 1, says why, no API call" || fail "no token (status $status)"

# 9. The API refuses the new issue: fails and says so.
reset
echo 403 >"$work/post_status"
run "$work/drifted.sh"; status=$?
[ "$status" -eq 1 ] && grep -q 'Could not open the issue' "$work/out.txt" \
    && pass "refused issue: exit 1, says so" || fail "refused issue (status $status)";

echo
if [ "$failures" -eq 0 ]; then echo "all passed"; else echo "$failures failed"; exit 1; fi
