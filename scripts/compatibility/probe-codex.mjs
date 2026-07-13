#!/usr/bin/env node
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";

const binary = process.argv[2];
if (!binary) {
  throw new Error("usage: probe-codex.mjs <codex-binary>");
}

const child = spawn(binary, ["app-server", "--stdio"], {
  stdio: ["pipe", "pipe", "pipe"],
});
const lines = createInterface({ input: child.stdout });
let nextId = 1;
const pending = new Map();
let stderr = "";
child.stderr.on("data", (chunk) => {
  stderr += chunk.toString("utf8");
});
lines.on("line", (line) => {
  let message;
  try {
    message = JSON.parse(line);
  } catch {
    return;
  }
  const waiter = pending.get(message.id);
  if (waiter) {
    pending.delete(message.id);
    waiter(message);
  }
});

function request(method, params) {
  const id = nextId++;
  child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      pending.delete(id);
      reject(new Error(`${method} timed out`));
    }, 10_000);
    pending.set(id, (message) => {
      clearTimeout(timeout);
      if (message.error) {
        const error = new Error(`${method}: ${message.error.message ?? "JSON-RPC error"}`);
        error.rpc = message.error;
        reject(error);
      } else {
        resolve(message.result);
      }
    });
  });
}

function notify(method, params) {
  child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", method, params })}\n`);
}

try {
  const initialize = await request("initialize", {
    clientInfo: {
      name: "previously-on-compatibility",
      title: "PreviouslyOn Compatibility Probe",
      version: "0.1.0-alpha.1",
    },
    capabilities: { experimentalApi: false, requestAttestation: false },
  });
  notify("initialized", {});
  const page = await request("thread/list", {
    limit: 100,
    sortKey: "created_at",
    sortDirection: "desc",
    useStateDbOnly: false,
  });
  if (!page || !Array.isArray(page.data)) {
    throw new Error("thread/list result omitted data array");
  }
  const summariesWithStableIds = page.data.filter(
    (thread) =>
      typeof thread?.id === "string" &&
      thread.id.length > 0 &&
      typeof thread?.sessionId === "string" &&
      thread.sessionId.length > 0,
  ).length;
  process.stdout.write(
    `${JSON.stringify({
      ok: true,
      binary,
      userAgent: initialize?.userAgent ?? null,
      summariesObserved: page.data.length,
      summariesWithStableIds,
      stableSourceIdCoverage:
        page.data.length === 0
          ? "not_observed"
          : summariesWithStableIds === page.data.length
            ? "complete"
            : "degraded",
      methods: ["initialize", "initialized", "thread/list"],
    })}\n`,
  );
} catch (error) {
  process.stderr.write(`${error.stack ?? error}\n${stderr}`);
  process.exitCode = 1;
} finally {
  child.stdin.end();
  setTimeout(() => child.kill("SIGTERM"), 250).unref();
}
