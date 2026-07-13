#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BINARY="${1:-$ROOT/target/debug/previously}"
[[ "$BINARY" = /* ]] || BINARY="$ROOT/$BINARY"
[[ -x "$BINARY" ]] || { echo "error: missing executable: $BINARY" >&2; exit 1; }
command -v sandbox-exec >/dev/null || { echo "error: sandbox-exec is required for the macOS offline gate" >&2; exit 1; }

STAGE="$(mktemp -d "${TMPDIR:-/tmp}/previously-on-offline.XXXXXX")"
trap 'rm -rf "$STAGE"' EXIT
mkdir -p "$STAGE/home" "$STAGE/data" "$STAGE/repo"
git -C "$STAGE/repo" init -q
git -C "$STAGE/repo" config user.name PreviouslyOn
git -C "$STAGE/repo" config user.email previously-on@example.invalid
touch "$STAGE/repo/README.md"
git -C "$STAGE/repo" add README.md
git -C "$STAGE/repo" commit -qm init

PROFILE="$STAGE/no-outbound.sb"
printf '%s\n' '(version 1)' '(allow default)' '(deny network-outbound)' > "$PROFILE"

offline() {
  sandbox-exec -f "$PROFILE" env \
    HOME="$STAGE/home" \
    PREVIOUSLY_ON_DATA_DIR="$STAGE/data" \
    PREVIOUSLY_ON_MANAGED_ID="previously-on-v1" \
    "$@"
}

offline "$BINARY" --version
offline "$BINARY" setup codex --repo "$STAGE/repo"
offline "$BINARY" status
offline "$BINARY" doctor
offline "$BINARY" export --format json
offline "$BINARY" purge --repo "$STAGE/repo"
offline "$BINARY" uninstall codex

echo "offline smoke passed with outbound network denied"
