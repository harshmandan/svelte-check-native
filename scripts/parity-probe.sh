#!/usr/bin/env bash
# Run upstream `svelte-check --tsgo` and svelte-check-native on one probe
# workspace and print both diagnostic sets plus their difference.
#
#   scripts/parity-probe.sh init <dir>     create <dir> as a probe workspace
#   scripts/parity-probe.sh run  <dir>     check it with both engines
#
# A probe workspace borrows the reference install from bench/cnblocks
# (svelte 5.55, @sveltejs/kit 2, svelte-check 4.4.6) through per-entry
# symlinks, so each probe keeps its own node_modules/.cache. `init` writes
# a strict tsconfig only when <dir> has none; edit it per probe.
#
# Output lines are `file line:col code severity`, 0-based as machine-verbose
# prints them. SCN_BIN overrides the native binary (default:
# target/release/svelte-check-native); extra arguments after <dir> go to
# both engines.
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
ref="$repo/bench/cnblocks/node_modules"
cmd=${1:?usage: parity-probe.sh init|run <dir>}
dir=${2:?usage: parity-probe.sh init|run <dir>}
shift 2

case $cmd in
init)
	mkdir -p "$dir/src" "$dir/node_modules"
	for entry in "$ref"/* "$ref"/.bin "$ref"/.pnpm; do
		[ -e "$entry" ] || continue
		ln -sfn "$entry" "$dir/node_modules/$(basename "$entry")"
	done
	[ -f "$dir/tsconfig.json" ] || cat >"$dir/tsconfig.json" <<'EOF'
{
  "compilerOptions": {
    "strict": true,
    "module": "esnext",
    "moduleResolution": "bundler",
    "target": "esnext",
    "skipLibCheck": true,
    "allowJs": true,
    "checkJs": true
  },
  "include": ["src/**/*"]
}
EOF
	;;
run)
	dir=$(cd "$dir" && pwd)
	bin=${SCN_BIN:-$repo/target/release/svelte-check-native}
	export TSGO_BIN=${TSGO_BIN:-$repo/node_modules/@typescript/native-preview-darwin-arm64/lib/tsgo}
	tuples() {
		grep -o '"type":"[A-Z]*","filename":"[^"]*","start":{[^}]*}.*"code":[^,}]*' |
			sed -E 's/"type":"([A-Z]).*filename":"([^"]*)","start":\{"line":([0-9]+),"character":([0-9]+)\}.*"code":(.*)/\2 \3:\4 \5 \1/' |
			sort -V || true
	}
	for d in "$dir/.svelte-check" "$dir/.svelte-kit/.svelte-check" "$dir/node_modules/.cache"; do
		if [ -e "$d" ]; then trash "$d"; fi
	done
	up=$(mktemp)
	ours=$(mktemp)
	CT_WAIT=1 CT_QUIET=1 "$repo/scripts/ct" exec "$dir/node_modules/.bin/svelte-check" --tsgo \
		--workspace "$dir" --output machine-verbose "$@" 2>&1 | tuples >"$up" || true
	CT_WAIT=1 CT_QUIET=1 "$repo/scripts/ct" exec "$bin" \
		--workspace "$dir" --output machine-verbose "$@" 2>&1 | tuples >"$ours" || true
	echo "== upstream"
	cat "$up"
	echo "== ours"
	cat "$ours"
	echo "== diff (< upstream only, > ours only)"
	if diff "$up" "$ours" >/dev/null; then echo "MATCH"; else diff "$up" "$ours" | grep '^[<>]' || true; fi
	trash "$up" "$ours"
	;;
*)
	echo "unknown command: $cmd" >&2
	exit 2
	;;
esac
