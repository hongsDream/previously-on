#!/usr/bin/env bash
set -euo pipefail

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
OUTPUT="$ROOT/outputs/mapped-compatibility-results.json"
LATEST_BIN=""
PREVIOUS_BIN=""
APP_CURRENT="not-run"
APP_PREVIOUS="not-run"

usage() {
  printf '%s\n' "usage: scripts/run-compatibility.sh [--latest-bin PATH --previous-bin PATH] [--codex-app-current VERSION --codex-app-previous VERSION] [--output PATH]"
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --latest-bin) LATEST_BIN="$2"; shift 2 ;;
    --previous-bin) PREVIOUS_BIN="$2"; shift 2 ;;
    --codex-app-current) APP_CURRENT="$2"; shift 2 ;;
    --codex-app-previous) APP_PREVIOUS="$2"; shift 2 ;;
    --output) OUTPUT="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
done

TEMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/previously-compat.XXXXXX")"
trap 'rm -rf "$TEMP_ROOT"' EXIT

if [ -z "$LATEST_BIN" ] || [ -z "$PREVIOUS_BIN" ]; then
  VERSIONS_JSON="$(npm view @openai/codex versions --json)"
  RESOLVED="$(VERSIONS_JSON="$VERSIONS_JSON" node -e '
    const versions = JSON.parse(process.env.VERSIONS_JSON);
    const stable = versions.filter((version) => /^\d+\.\d+\.\d+$/.test(version));
    stable.sort((a, b) => {
      const aa = a.split(".").map(Number);
      const bb = b.split(".").map(Number);
      return aa[0] - bb[0] || aa[1] - bb[1] || aa[2] - bb[2];
    });
    if (stable.length < 2) throw new Error("npm returned fewer than two stable Codex versions");
    process.stdout.write(`${stable.at(-1)}\n${stable.at(-2)}\n`);
  ')"
  LATEST_VERSION="$(printf '%s\n' "$RESOLVED" | sed -n '1p')"
  PREVIOUS_VERSION="$(printf '%s\n' "$RESOLVED" | sed -n '2p')"
  npm install --no-audit --no-fund --prefix "$TEMP_ROOT/latest" "@openai/codex@$LATEST_VERSION"
  npm install --no-audit --no-fund --prefix "$TEMP_ROOT/previous" "@openai/codex@$PREVIOUS_VERSION"
  LATEST_BIN="$TEMP_ROOT/latest/node_modules/.bin/codex"
  PREVIOUS_BIN="$TEMP_ROOT/previous/node_modules/.bin/codex"
else
  LATEST_VERSION="$($LATEST_BIN --version | awk '{print $NF}')"
  PREVIOUS_VERSION="$($PREVIOUS_BIN --version | awk '{print $NF}')"
fi

if [ "$LATEST_VERSION" = "$PREVIOUS_VERSION" ]; then
  printf '%s\n' "latest and previous Codex versions must differ" >&2
  exit 2
fi

mkdir -p "$TEMP_ROOT/codex-home-latest" "$TEMP_ROOT/codex-home-previous"
CODEX_HOME="$TEMP_ROOT/codex-home-latest" \
  node "$ROOT/scripts/compatibility/probe-codex.mjs" "$LATEST_BIN" > "$TEMP_ROOT/latest-probe.json"
CODEX_HOME="$TEMP_ROOT/codex-home-previous" \
  node "$ROOT/scripts/compatibility/probe-codex.mjs" "$PREVIOUS_BIN" > "$TEMP_ROOT/previous-probe.json"

TARGETS="$(node -e '
  const matrix = require(process.argv[1]);
  process.stdout.write([...new Set(matrix.scenarios.map((scenario) => scenario.testTarget))].sort().join("\n"));
' "$ROOT/fixtures/compatibility/scenarios.json")"

run_fixture_matrix() {
  version="$1"
  PREVIOUSLY_CODEX_UNDER_TEST="$version" cargo test --locked --test compatibility_matrix
  for target in $TARGETS; do
    PREVIOUSLY_CODEX_UNDER_TEST="$version" cargo test --locked --test "$target"
  done
}

cd "$ROOT"
run_fixture_matrix "$LATEST_VERSION"
run_fixture_matrix "$PREVIOUS_VERSION"

GIT_TREE_STATE="clean"
if [ -n "$(git -C "$ROOT" status --porcelain)" ]; then
  GIT_TREE_STATE="dirty"
fi

mkdir -p "$(dirname -- "$OUTPUT")"
LATEST_VERSION="$LATEST_VERSION" \
PREVIOUS_VERSION="$PREVIOUS_VERSION" \
GIT_COMMIT="$(git -C "$ROOT" rev-parse HEAD)" \
GIT_TREE_STATE="$GIT_TREE_STATE" \
APP_CURRENT="$APP_CURRENT" \
APP_PREVIOUS="$APP_PREVIOUS" \
MATRIX_PATH="$ROOT/fixtures/compatibility/scenarios.json" \
LATEST_PROBE="$TEMP_ROOT/latest-probe.json" \
PREVIOUS_PROBE="$TEMP_ROOT/previous-probe.json" \
OUTPUT="$OUTPUT" \
node -e '
  const fs = require("node:fs");
  const crypto = require("node:crypto");
  const matrix = JSON.parse(fs.readFileSync(process.env.MATRIX_PATH, "utf8"));
  const versions = [process.env.LATEST_VERSION, process.env.PREVIOUS_VERSION];
  const result = {
    schemaVersion: 1,
    product: "PreviouslyOn",
    productVersion: "0.1.0-alpha.1",
    gitCommit: process.env.GIT_COMMIT,
    gitTreeState: process.env.GIT_TREE_STATE,
    scenarioMatrixSha256: crypto.createHash("sha256").update(fs.readFileSync(process.env.MATRIX_PATH)).digest("hex"),
    generatedAt: new Date().toISOString(),
    evidenceClass: "local_mapped_regression_plus_live_app_server_schema_probe",
    supportMode: "explicit_run_and_import",
    codexCli: { latest: versions[0], previous: versions[1] },
    codexApp: { current: process.env.APP_CURRENT, previous: process.env.APP_PREVIOUS },
    localMappedRegression: {
      scenarioCount: matrix.scenarios.length,
      categoryCounts: matrix.scenarios.reduce((counts, scenario) => {
        counts[scenario.category] = (counts[scenario.category] ?? 0) + 1;
        return counts;
      }, {}),
      runs: versions.map((version) => ({
        codexVersionSlot: version,
        passed: matrix.scenarios.map((scenario) => scenario.id),
        failed: [],
      })),
      limitation: "These are repo-local mapped regression suites, not 30 end-to-end Codex workflows.",
    },
    liveAppServerSchemaProbes: {
      [versions[0]]: JSON.parse(fs.readFileSync(process.env.LATEST_PROBE, "utf8")),
      [versions[1]]: JSON.parse(fs.readFileSync(process.env.PREVIOUS_PROBE, "utf8")),
    },
    liveCodexWorkflowMatrix: {
      status: "not_run",
      requiredScenarioCount: 30,
      requiredPerCliVersion: 30,
      requiredReconstruction: ["user_prompt", "assistant_final", "file_change_tool", "test_command"],
      evidence: null,
    },
    transparentCaptureReleaseGate: {
      eligible: false,
      blocksTransparentCaptureClaim: true,
      reason: "App Server schema probes and mapped local regressions do not prove 30 live Codex workflows or stable cross-surface source-ID linkage.",
    },
  };
  fs.writeFileSync(process.env.OUTPUT, `${JSON.stringify(result, null, 2)}\n`, { mode: 0o600 });
' 

printf 'mapped regressions and App Server schema probes passed: %s (%s and %s); transparent capture remains unverified\n' "$OUTPUT" "$LATEST_VERSION" "$PREVIOUS_VERSION"
