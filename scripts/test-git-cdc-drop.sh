#!/usr/bin/env bash
# Smoke test: does `git cdc drop` actually reclaim local disk, and does
# `git cdc pull` bring the content back? Both halves of local storage (the
# hydrated worktree file and its chunks) must go, `git status` must stay
# clean, and pull must restore the original bytes. Uses a throwaway
# fs-backed chunk store, so no server is needed. Not a regression suite —
# see `cargo test --workspace` / crates/cli/tests for that.
#
# Usage: scripts/test-git-cdc-drop.sh [path-to-git-cdc-binary]
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

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

repo="$scratch/repo"
remote="$scratch/remote"
mkdir -p "$repo" "$remote"

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
git config cdc.store.scheme fs
git config cdc.store.option "root=$remote"
git config cdc.store.prefix chunks/
"$BIN" track '*.bin'

# A couple MiB so it actually chunks into several objects.
head -c 4194304 /dev/urandom >asset.bin
# Kept outside the repo: an untracked file in it would show up in the
# `git status` check below.
cp asset.bin "$scratch/asset.bin.orig"

git add .gitattributes asset.bin
git commit -q -m "smoke: add asset"

# drop --chunks only deletes chunks the remote confirms it has, so push first.
"$BIN" push

chunks_before="$(find .git/cdc/objects -type f | wc -l | tr -d ' ')"
if [ "$chunks_before" -lt 2 ]; then
	echo "FAIL: expected several local chunks before drop, found $chunks_before" >&2
	exit 1
fi

"$BIN" drop --chunks asset.bin

if [[ "$(head -c 24 asset.bin)" != version\ git-cdc/spec/v1 ]]; then
	echo "FAIL: dropped asset.bin is not manifest text" >&2
	exit 1
fi

chunks_after="$(find .git/cdc/objects -type f | wc -l | tr -d ' ')"
if [ "$chunks_after" != "0" ]; then
	echo "FAIL: drop --chunks left $chunks_after chunks in the local store" >&2
	exit 1
fi

# The whole point of writing the manifest back verbatim: git must not see a
# dropped file as modified.
dirty="$(git status --porcelain)"
if [ -n "$dirty" ]; then
	echo "FAIL: worktree is dirty after drop: $dirty" >&2
	exit 1
fi

"$BIN" pull

if ! cmp -s asset.bin "$scratch/asset.bin.orig"; then
	echo "FAIL: pull did not restore the original bytes after drop" >&2
	exit 1
fi

echo "OK: git cdc drop --chunks reclaimed $chunks_before chunks + the worktree copy, pull restored it"
