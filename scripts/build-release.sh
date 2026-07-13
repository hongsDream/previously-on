#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PACKAGE_NAME="$(awk -F ' *= *' '/^name = / { gsub(/"/, "", $2); print $2; exit }' "$ROOT/Cargo.toml")"
VERSION="$(awk -F ' *= *' '/^version = / { gsub(/"/, "", $2); print $2; exit }' "$ROOT/Cargo.toml")"
EXPECTED_VERSION="${EXPECTED_VERSION:-0.1.0-alpha.1}"
OUTPUT_DIR="${OUTPUT_DIR:-$ROOT/outputs}"
SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-0}"

[[ "$PACKAGE_NAME" == "previously-on" ]] || { echo "error: unexpected package: $PACKAGE_NAME" >&2; exit 1; }
[[ "$VERSION" == "$EXPECTED_VERSION" ]] || { echo "error: expected $EXPECTED_VERSION, found $VERSION" >&2; exit 1; }
[[ "$(uname -s)" == "Darwin" ]] || { echo "error: release archives are built on macOS" >&2; exit 1; }
[[ "${MACOS_ARCH:-$(uname -m)}" =~ ^(arm64|aarch64)$ ]] || {
  echo "error: v$VERSION publishes an Apple Silicon binary only" >&2
  exit 1
}

if [[ -n "${GITHUB_REF_TYPE:-}" && "${GITHUB_REF_TYPE}" == "tag" ]]; then
  [[ "${GITHUB_REF_NAME:-}" == "v$VERSION" ]] || {
    echo "error: tag ${GITHUB_REF_NAME:-<missing>} does not match v$VERSION" >&2
    exit 1
  }
fi

if git -C "$ROOT" rev-parse --is-inside-work-tree >/dev/null 2>&1 \
  && [[ "${RELEASE_ALLOW_DIRTY:-0}" != "1" ]] \
  && [[ -n "$(git -C "$ROOT" status --porcelain --untracked-files=normal)" ]]; then
  echo "error: release builds require a clean checkout (set RELEASE_ALLOW_DIRTY=1 only for local rehearsal)" >&2
  exit 1
fi

mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR="$(cd "$OUTPUT_DIR" && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
[[ "$TARGET_DIR" = /* ]] || TARGET_DIR="$ROOT/$TARGET_DIR"
STAGE="$(mktemp -d "${TMPDIR:-/tmp}/previously-on-release.XXXXXX")"
trap 'rm -rf "$STAGE"' EXIT

echo "==> quality gates"
(
  cd "$ROOT"
  npm --prefix ui ci
  npm --prefix ui run typecheck
  npm --prefix ui run lint
  npm --prefix ui test -- --run
  npm --prefix ui run build
  npm --prefix ui audit --audit-level=high
  if git rev-parse --is-inside-work-tree >/dev/null 2>&1 && [[ "${RELEASE_ALLOW_DIRTY:-0}" != "1" ]]; then
    git diff --exit-code -- ui/dist
  fi
  cargo fmt --check
  cargo clippy --locked --all-targets --all-features -- -D warnings
  PREVIOUSLY_ON_ADVERSARIAL_TESTS=1 cargo test --locked --all
)

echo "==> locked metadata, license inventory, and SBOM"
node "$ROOT/scripts/generate-release-metadata.mjs" \
  --sbom "$STAGE/previously-on-v$VERSION.cdx.json" \
  --licenses "$STAGE/THIRD_PARTY_LICENSES.md"
cmp "$ROOT/THIRD_PARTY_LICENSES.md" "$STAGE/THIRD_PARTY_LICENSES.md" || {
  echo "error: THIRD_PARTY_LICENSES.md is stale; regenerate it before release" >&2
  exit 1
}

echo "==> crates.io package and extracted-source offline install"
(
  cd "$ROOT"
  cargo package --locked
)
CRATE_ARCHIVE="$TARGET_DIR/package/$PACKAGE_NAME-$VERSION.crate"
[[ -f "$CRATE_ARCHIVE" ]] || { echo "error: cargo package did not produce $CRATE_ARCHIVE" >&2; exit 1; }

SOURCE_STAGE="$STAGE/source"
SOURCE_BUNDLE_NAME="$PACKAGE_NAME-$VERSION"
mkdir -p "$SOURCE_STAGE"
COPYFILE_DISABLE=1 tar -xzf "$CRATE_ARCHIVE" -C "$SOURCE_STAGE"
SOURCE_BUNDLE_DIR="$SOURCE_STAGE/$SOURCE_BUNDLE_NAME"
[[ -f "$SOURCE_BUNDLE_DIR/Cargo.toml" && -f "$SOURCE_BUNDLE_DIR/ui/dist/index.html" ]] || {
  echo "error: packaged source is incomplete" >&2
  exit 1
}

INSTALL_ROOT="$STAGE/install"
CARGO_TARGET_DIR="$STAGE/offline-target" cargo install \
  --path "$SOURCE_BUNDLE_DIR" \
  --root "$INSTALL_ROOT" \
  --locked \
  --offline
BINARY_PATH="$INSTALL_ROOT/bin/previously"
[[ -x "$BINARY_PATH" ]] || { echo "error: extracted source did not install previously" >&2; exit 1; }
"$BINARY_PATH" --version | grep -F "previously $VERSION" >/dev/null || {
  echo "error: installed binary version does not match $VERSION" >&2
  exit 1
}
"$ROOT/scripts/offline-smoke.sh" "$BINARY_PATH"

STAMP="$(date -u -r "$SOURCE_DATE_EPOCH" '+%Y%m%d%H%M.%S' 2>/dev/null)" || {
  echo "error: invalid SOURCE_DATE_EPOCH: $SOURCE_DATE_EPOCH" >&2
  exit 1
}
BINARY_BASENAME="previously-v$VERSION-macos-arm64"
MACOS_BUNDLE_NAME="$PACKAGE_NAME-v$VERSION-macos-arm64"
MACOS_ARCHIVE_BASENAME="$MACOS_BUNDLE_NAME.tar.gz"
SOURCE_ARCHIVE_BASENAME="$PACKAGE_NAME-v$VERSION-source.tar.gz"
SBOM_BASENAME="$PACKAGE_NAME-v$VERSION.cdx.json"

install -m 0755 "$BINARY_PATH" "$OUTPUT_DIR/$BINARY_BASENAME"
install -m 0644 "$STAGE/previously-on-v$VERSION.cdx.json" "$OUTPUT_DIR/$SBOM_BASENAME"
install -m 0644 "$ROOT/NOTICE" "$OUTPUT_DIR/NOTICE"
install -m 0644 "$ROOT/THIRD_PARTY_LICENSES.md" "$OUTPUT_DIR/THIRD_PARTY_LICENSES.md"

MACOS_BUNDLE_DIR="$STAGE/$MACOS_BUNDLE_NAME"
mkdir -p "$MACOS_BUNDLE_DIR"
install -m 0755 "$BINARY_PATH" "$MACOS_BUNDLE_DIR/previously"
for file in LICENSE NOTICE README.md CHANGELOG.md THIRD_PARTY_LICENSES.md; do
  install -m 0644 "$ROOT/$file" "$MACOS_BUNDLE_DIR/$file"
done
install -m 0644 "$STAGE/previously-on-v$VERSION.cdx.json" "$MACOS_BUNDLE_DIR/$SBOM_BASENAME"
find "$MACOS_BUNDLE_DIR" -exec env TZ=UTC touch -t "$STAMP" {} +

make_archive() {
  local base_dir="$1"
  local bundle="$2"
  local output="$3"
  (
    cd "$base_dir"
    find "$bundle" -type f -print \
      | LC_ALL=C sort \
      | COPYFILE_DISABLE=1 tar --format ustar --uid 0 --gid 0 --uname root --gname root -cf - -T - \
      | gzip -n > "$output"
  )
}

echo "==> reproducible archives"
make_archive "$STAGE" "$MACOS_BUNDLE_NAME" "$STAGE/macos-1.tar.gz"
make_archive "$STAGE" "$MACOS_BUNDLE_NAME" "$STAGE/macos-2.tar.gz"
cmp "$STAGE/macos-1.tar.gz" "$STAGE/macos-2.tar.gz"
install -m 0644 "$STAGE/macos-1.tar.gz" "$OUTPUT_DIR/$MACOS_ARCHIVE_BASENAME"

find "$SOURCE_BUNDLE_DIR" -exec env TZ=UTC touch -t "$STAMP" {} +
make_archive "$SOURCE_STAGE" "$SOURCE_BUNDLE_NAME" "$STAGE/source-1.tar.gz"
make_archive "$SOURCE_STAGE" "$SOURCE_BUNDLE_NAME" "$STAGE/source-2.tar.gz"
cmp "$STAGE/source-1.tar.gz" "$STAGE/source-2.tar.gz"
install -m 0644 "$STAGE/source-1.tar.gz" "$OUTPUT_DIR/$SOURCE_ARCHIVE_BASENAME"

tar -tzf "$OUTPUT_DIR/$MACOS_ARCHIVE_BASENAME" | grep -F "$MACOS_BUNDLE_NAME/previously" >/dev/null
tar -tzf "$OUTPUT_DIR/$SOURCE_ARCHIVE_BASENAME" | grep -F "$SOURCE_BUNDLE_NAME/Cargo.toml" >/dev/null

ARTIFACTS=(
  "$BINARY_BASENAME"
  "$MACOS_ARCHIVE_BASENAME"
  "$SOURCE_ARCHIVE_BASENAME"
  "$SBOM_BASENAME"
  "NOTICE"
  "THIRD_PARTY_LICENSES.md"
)
(
  cd "$OUTPUT_DIR"
  shasum -a 256 "${ARTIFACTS[@]}" > SHA256SUMS
  shasum -a 256 -c SHA256SUMS
)

echo "Release artifacts written to $OUTPUT_DIR:"
printf '  %s\n' "${ARTIFACTS[@]}" SHA256SUMS
