#!/usr/bin/env bash
# Smoke test: is a running git-cdc-server alive and answering auth'd
# requests? One read-only GET against /healthz. Safe to run against a
# live/production server.
#
# Usage: scripts/test-git-cdc-server-health.sh <base_url> <token>
# Example: scripts/test-git-cdc-server-health.sh http://127.0.0.1:8077 my-secret
set -euo pipefail

base_url="${1:?usage: $0 <base_url> <token>}"
token="${2:?usage: $0 <base_url> <token>}"

body_file="$(mktemp)"
trap 'rm -f "$body_file"' EXIT

if ! status="$(curl -sS -o "$body_file" -w '%{http_code}' --max-time 5 \
	-H "Authorization: Bearer $token" \
	"$base_url/healthz")"; then
	echo "FAIL: could not reach $base_url/healthz" >&2
	exit 1
fi
body="$(cat "$body_file")"

if [ "$status" != "200" ] || [ "$body" != "ok" ]; then
	echo "FAIL: $base_url/healthz returned status $status, body '$body'" >&2
	exit 1
fi

echo "OK: $base_url/healthz is up"
