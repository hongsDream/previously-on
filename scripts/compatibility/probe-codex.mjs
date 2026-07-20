#!/usr/bin/env node
import { spawn, spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createInterface } from "node:readline";

const binary = process.argv[2];
if (!binary) {
  throw new Error("usage: probe-codex.mjs <codex-binary>");
}

function probeSchema() {
  const directory = mkdtempSync(join(tmpdir(), "previously-codex-schema-"));
  try {
    const generated = spawnSync(
      binary,
      ["app-server", "generate-json-schema", "--experimental", "--out", directory],
      { encoding: "utf8", timeout: 10_000 },
    );
    if (generated.error) throw generated.error;
    if (generated.status !== 0) {
      throw new Error(
        `generate-json-schema exited ${generated.status}: ${generated.stderr.trim()}`,
      );
    }

    const schemas = [
      ["thread/list", "ThreadList"],
      ["thread/read", "ThreadRead"],
      ["thread/start", "ThreadStart"],
      ["thread/resume", "ThreadResume"],
      ["thread/name/set", "ThreadSetName"],
      ["turn/start", "TurnStart"],
      ["permissionProfile/list", "PermissionProfileList"],
    ];
    const v2 = join(directory, "v2");
    const readSchema = (name) =>
      JSON.parse(readFileSync(join(v2, `${name}.json`), "utf8"));
    const supportedMethods = schemas
      .filter(([, schema]) => {
        try {
          readSchema(`${schema}Params`);
          readSchema(`${schema}Response`);
          return true;
        } catch {
          return false;
        }
      })
      .map(([method]) => method);
    const threadStart = readSchema("ThreadStartParams");
    const turnStart = readSchema("TurnStartParams");
    const exactRefreshFields =
      Object.hasOwn(threadStart.properties ?? {}, "permissions") &&
      Object.hasOwn(threadStart.properties ?? {}, "approvalPolicy") &&
      Object.hasOwn(turnStart.properties ?? {}, "outputSchema");
    const capability = (required, exactFields = true) => {
      const count = required.filter((method) => supportedMethods.includes(method)).length;
      if (count === required.length && exactFields) return "complete";
      if (count > 0) return "degraded";
      return "unsupported";
    };
    const result = {
      methods: ["initialize", "initialized", ...supportedMethods].sort(),
      capabilities: {
        coreImport: capability(["thread/list", "thread/read"]),
        continuation: capability([
          "thread/start",
          "thread/resume",
          "thread/name/set",
          "turn/start",
        ]),
        experimentalRefresh: capability(
          ["permissionProfile/list", "thread/start", "turn/start"],
          exactRefreshFields,
        ),
      },
      exactRefreshFields,
    };
    const unavailable = Object.entries(result.capabilities)
      .filter(([, status]) => status !== "complete")
      .map(([name, status]) => `${name}=${status}`);
    if (unavailable.length > 0) {
      throw new Error(`required App Server schema capabilities unavailable: ${unavailable.join(", ")}`);
    }
    return result;
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

const schema = probeSchema();

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
      version: "0.1.0-alpha.3",
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
      methods: schema.methods,
      capabilities: schema.capabilities,
      exactRefreshFields: schema.exactRefreshFields,
    })}\n`,
  );
} catch (error) {
  process.stderr.write(`${error.stack ?? error}\n${stderr}`);
  process.exitCode = 1;
} finally {
  child.stdin.end();
  setTimeout(() => child.kill("SIGTERM"), 250).unref();
}
