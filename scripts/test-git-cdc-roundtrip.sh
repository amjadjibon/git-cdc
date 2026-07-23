#!/usr/bin/env bash
# Smoke test: is the built git-cdc binary actually alive and doing its one
# job? track -> add -> commit (clean to a manifest) -> checkout (smudge
# back to the original bytes), in a throwaway repo. Not a regression suite —
# see `cargo test --workspace` / crates/cli/tests for that.
#
# Usage: scripts/test-git-cdc-roundtrip.sh [path-to-git-cdc-binary]
# Defaults to `git-cdc` on $PATH.
set -euo pipefail

BIN="${1:-git-cdc}"

case "$BIN" in
*/*)
	# A path (relative or absolute) — resolve to absolute before we cd
	# into the scratch repo below.
	[ -x "$BIN" ] || {
		echo "FAIL: git-cdc binary not found or not executable: $BIN" >&2
		exit 1
	}
	BIN="$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")"
	;;
*)
	command -v "$BIN" >/dev/null 2>&1 || {
		echo "FAIL: git-cdc binary not found on \$PATH: $BIN" >&2
		exit 1
	}
	BIN="$(command -v "$BIN")"
	;;
esac

repo="$(mktemp -d)"
trap 'rm -rf "$repo"' EXIT

cd "$repo"
export GIT_CONFIG_GLOBAL=/dev/null
export GIT_CONFIG_SYSTEM=/dev/null

git init -q
git config user.email smoke@test.local
git config user.name smoke

"$BIN" install
git config filter.cdc.clean "$BIN clean"
git config filter.cdc.smudge "$BIN smudge"
git config filter.cdc.process "$BIN filter-process"
"$BIN" track '*.bin'

# A couple MiB so it actually chunks, not just passes through as one blob.
head -c 2097152 /dev/urandom >asset.bin
cp asset.bin asset.bin.orig

git add .gitattributes asset.bin
git commit -q -m "smoke: add asset"

blob="$(git show HEAD:asset.bin)"
if [[ "$blob" != version\ git-cdc/spec/v1* ]]; then
	echo "FAIL: committed blob is not a git-cdc manifest" >&2
	exit 1
fi

rm asset.bin
git checkout -q -- asset.bin

if ! cmp -s asset.bin asset.bin.orig; then
	echo "FAIL: checked-out asset.bin does not match the original bytes" >&2
	exit 1
fi

echo "OK: git-cdc track/add/commit/checkout round-trip is byte-identical"
