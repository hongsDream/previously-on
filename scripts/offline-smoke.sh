#!/usr/bin/env bash
set -euo pipefail
umask 077

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BINARY="${1:-$ROOT/target/debug/previously}"
[[ "$BINARY" = /* ]] || BINARY="$ROOT/$BINARY"
[[ -x "$BINARY" ]] || { echo "error: missing executable: $BINARY" >&2; exit 1; }
command -v sandbox-exec >/dev/null || { echo "error: sandbox-exec is required for the macOS offline gate" >&2; exit 1; }

STAGE="$(mktemp -d "${TMPDIR:-/tmp}/previously-on-offline.XXXXXX")"
DAEMON_PID=""
UI_PID=""
cleanup() {
  [[ -z "$UI_PID" ]] || kill "$UI_PID" 2>/dev/null || true
  [[ -z "$DAEMON_PID" ]] || kill "$DAEMON_PID" 2>/dev/null || true
  [[ -z "$UI_PID" ]] || wait "$UI_PID" 2>/dev/null || true
  [[ -z "$DAEMON_PID" ]] || wait "$DAEMON_PID" 2>/dev/null || true
  rm -rf "$STAGE"
}
trap cleanup EXIT
mkdir -p "$STAGE/home" "$STAGE/data" "$STAGE/repo" "$STAGE/bin"
printf '%s\n' '#!/usr/bin/env bash' 'printf "%s\\n" "codex-cli 0.0.0-offline"' > "$STAGE/bin/codex"
chmod 0755 "$STAGE/bin/codex"
git -C "$STAGE/repo" init -q
git -C "$STAGE/repo" config user.name PreviouslyOn
git -C "$STAGE/repo" config user.email previously-on@example.invalid
touch "$STAGE/repo/README.md"
git -C "$STAGE/repo" add README.md
git -C "$STAGE/repo" commit -qm init
REPO_CANONICAL="$(node -e 'const fs=require("node:fs"); process.stdout.write(fs.realpathSync(process.argv[1]))' "$STAGE/repo")"

PROFILE="$STAGE/no-outbound.sb"
printf '%s\n' \
  '(version 1)' \
  '(allow default)' \
  '(deny network-outbound (require-all (remote ip "*:*") (require-not (remote ip "localhost:*"))))' \
  > "$PROFILE"

offline() {
  sandbox-exec -f "$PROFILE" env \
    HOME="$STAGE/home" \
    PATH="$STAGE/bin:$PATH" \
    PREVIOUSLY_ON_DATA_DIR="$STAGE/data" \
    PREVIOUSLY_ON_MANAGED_ID="previously-on-v1" \
    "$@"
}

start_ui() {
  : >"$STAGE/ui.stdout"
  : >"$STAGE/ui.stderr"
  offline "$BINARY" ui --bind 127.0.0.1:0 --no-open >"$STAGE/ui.stdout" 2>"$STAGE/ui.stderr" &
  UI_PID=$!
  for _ in {1..100}; do
    grep -F 'PreviouslyOn UI: http://127.0.0.1:' "$STAGE/ui.stdout" >/dev/null 2>&1 && break
    kill -0 "$UI_PID" 2>/dev/null || {
      sed -n '1,120p' "$STAGE/ui.stderr" >&2
      echo "error: loopback UI exited before serving a request" >&2
      exit 1
    }
    sleep 0.05
  done
  UI_URL="$(sed -n 's/^PreviouslyOn UI: //p' "$STAGE/ui.stdout" | head -n 1)"
  [[ "$UI_URL" =~ ^http://127\.0\.0\.1:[0-9]+$ ]] || {
    echo "error: loopback UI did not report a valid ephemeral address" >&2
    exit 1
  }
  rm -f "$STAGE/ui.cookies"
  for _ in {1..100}; do
    if curl --fail --silent --max-time 2 \
      --cookie-jar "$STAGE/ui.cookies" --output "$STAGE/ui.html" "$UI_URL/"; then
      return
    fi
    kill -0 "$UI_PID" 2>/dev/null || {
      sed -n '1,120p' "$STAGE/ui.stderr" >&2
      echo "error: loopback UI exited before accepting a request" >&2
      exit 1
    }
    sleep 0.05
  done
  echo "error: loopback UI did not accept a request" >&2
  exit 1
}

assert_ui_state() {
  local expected="$1"
  curl --fail --silent --show-error --max-time 10 \
    --cookie "$STAGE/ui.cookies" --output "$STAGE/bootstrap-$expected.json" "$UI_URL/api/bootstrap"
  node -e '
    const fs = require("node:fs");
    const [path, expected] = process.argv.slice(1);
    const value = JSON.parse(fs.readFileSync(path, "utf8"));
    if (value.repository?.state !== expected) {
      throw new Error(`expected ${expected}, received ${JSON.stringify(value.repository)}`);
    }
    if (expected === "unregistered" && value.contractEvaluation !== null) {
      throw new Error(`${expected} bootstrap synthesized contractEvaluation`);
    }
    if (expected === "active" && (!Array.isArray(value.checkpoints) || value.checkpoints.length < 1)) {
      throw new Error("active bootstrap has no first checkpoint");
    }
  ' "$STAGE/bootstrap-$expected.json" "$expected"
}

stop_ui() {
  kill "$UI_PID"
  wait "$UI_PID" 2>/dev/null || true
  UI_PID=""
}

offline "$BINARY" --version
if offline curl --silent --max-time 2 https://example.com/ >/dev/null 2>&1; then
  echo "error: sandbox unexpectedly allowed non-loopback outbound HTTP" >&2
  exit 1
fi
start_ui
assert_ui_state unregistered

offline "$BINARY" setup codex --repo "$REPO_CANONICAL"
offline "$BINARY" status
offline "$BINARY" doctor

assert_ui_state registered-empty

offline "$BINARY" daemon >"$STAGE/daemon.stdout" 2>"$STAGE/daemon.stderr" &
DAEMON_PID=$!
for _ in {1..100}; do
  [[ -S "$STAGE/data/previously.sock" ]] && break
  kill -0 "$DAEMON_PID" 2>/dev/null || {
    sed -n '1,120p' "$STAGE/daemon.stderr" >&2
    echo "error: offline daemon exited before creating its socket" >&2
    exit 1
  }
  sleep 0.05
done
[[ -S "$STAGE/data/previously.sock" ]] || { echo "error: offline daemon socket was not created" >&2; exit 1; }

node -e '
  const fs = require("node:fs");
  fs.writeFileSync(process.argv[1], `${JSON.stringify({
    session_id: "offline-session",
    turn_id: "turn-1",
    cwd: process.argv[2],
    timestamp: "2026-07-13T00:00:00Z",
  })}\n`);
' "$STAGE/hook.json" "$REPO_CANONICAL"
offline "$BINARY" hook SessionStart <"$STAGE/hook.json" >"$STAGE/hook-response.json"
node -e '
  const fs = require("node:fs");
  const response = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
  if (Object.keys(response).length !== 0) throw new Error("unexpected SessionStart hook response");
' "$STAGE/hook-response.json"

node -e '
  const fs = require("node:fs");
  const base = {
    session_id: "offline-session",
    turn_id: "turn-1",
    cwd: process.argv[2],
    timestamp: "2026-07-13T00:00:01Z",
  };
  fs.writeFileSync(process.argv[1], `${JSON.stringify({...base, prompt: "Create the first synthetic checkpoint"})}\n`);
' "$STAGE/prompt.json" "$REPO_CANONICAL"
offline "$BINARY" hook UserPromptSubmit <"$STAGE/prompt.json" >"$STAGE/prompt-response.json"
node -e '
  const fs = require("node:fs");
  fs.writeFileSync(process.argv[1], `${JSON.stringify({
    session_id: "offline-session",
    turn_id: "turn-1",
    cwd: process.argv[2],
    timestamp: "2026-07-13T00:00:02Z",
    last_assistant_message: "Created the first synthetic checkpoint.",
  })}\n`);
' "$STAGE/stop.json" "$REPO_CANONICAL"
offline "$BINARY" hook Stop <"$STAGE/stop.json" >"$STAGE/stop-response.json"

assert_ui_state active
stop_ui

offline "$BINARY" export --format json >"$STAGE/export-before-purge.json"
node -e '
  const fs = require("node:fs");
  const value = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
  if ((value.canonical_events?.length ?? 0) < 4) {
    throw new Error(`daemon/hook delivery did not persist the synthetic first-checkpoint transition: ${JSON.stringify(value)}`);
  }
' "$STAGE/export-before-purge.json"

printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","clientInfo":{"name":"offline-smoke","version":"1"},"capabilities":{}}}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' \
  >"$STAGE/mcp-input.jsonl"
offline "$BINARY" mcp <"$STAGE/mcp-input.jsonl" >"$STAGE/mcp-output.jsonl"
node -e '
  const fs = require("node:fs");
  const rows = fs.readFileSync(process.argv[1], "utf8").trim().split(/\r?\n/).map(JSON.parse);
  if (rows.length !== 2 || rows[0].result?.serverInfo?.name !== "previously-on") {
    throw new Error("MCP initialize did not return the PreviouslyOn server identity");
  }
  const names = rows[1].result?.tools?.map(({name}) => name).sort();
  const expected = ["explain_fact", "get_task_timeline", "resume_task", "search_tasks", "suggest_resume"].sort();
  if (JSON.stringify(names) !== JSON.stringify(expected)) throw new Error("MCP tools/list contract changed");
' "$STAGE/mcp-output.jsonl"

offline "$BINARY" purge --repo "$REPO_CANONICAL"
wait "$DAEMON_PID" 2>/dev/null || true
DAEMON_PID=""
offline "$BINARY" export --format json >"$STAGE/export-after-purge.json"
node -e '
  const fs = require("node:fs");
  const value = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
  if (value.canonical_events?.length !== 0 || value.repositories?.length !== 0) {
    throw new Error("purge retained repository canonical data");
  }
' "$STAGE/export-after-purge.json"
offline "$BINARY" uninstall codex
offline "$BINARY" uninstall codex

[[ ! -e "$STAGE/data/setup-manifest.json" ]]
[[ ! -S "$STAGE/data/previously.sock" ]]
[[ ! -e "$STAGE/data/queue/events.jsonl" ]]
for managed_file in "$STAGE/home/.codex/hooks.json" "$STAGE/home/.codex/config.toml"; do
  [[ ! -e "$managed_file" ]] || ! grep -F 'previously-on-v1' "$managed_file" >/dev/null
done

echo "offline smoke passed: setup-before -> registered-empty -> first-checkpoint UI transition, daemon/hook, MCP, export, purge, and uninstall with non-loopback outbound denied"
