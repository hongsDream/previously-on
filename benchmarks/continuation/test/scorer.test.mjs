import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { evaluateRule, parseFinalJson, scoreFinalResponse } from "../src/scorer.mjs";

const fixture = {
  rubric: {
    criteria: [
      { id: "answer", path: "answer", equals: 42, weight: 2 },
      { id: "file", path: "changedFiles", containsAll: ["src/lib.rs"] },
      { id: "test", path: "tests.status", oneOf: ["passed", "not_run"] },
    ],
    invariants: [{ id: "no-commit", path: "committed", equals: false }],
    staleClaims: [{ id: "old-main", path: "claims", forbidden: "old-main-sha" }],
    seriousErrors: [{ id: "wrong-file", when: { path: "changedFiles", includes: "secrets.env" } }],
    stateRecall: {
      goal: { equals: "return the answer" },
      changedFiles: { containsAll: ["src/lib.rs"] },
      testStatus: "passed",
      nextStep: "open-pr",
    },
  },
};

test("strictly parses one final JSON object without accepting fenced prose", () => {
  assert.deepEqual(parseFinalJson('{"ok":true}'), { ok: true });
  assert.throws(() => parseFinalJson('```json\n{"ok":true}\n```'), /not exact JSON/);
  assert.throws(() => parseFinalJson("[]"), /must be an object/);
});

test("scores correctness, invariant/stale/serious errors, and four recall dimensions", () => {
  const final = {
    answer: 42,
    changedFiles: ["src/lib.rs"],
    tests: { status: "passed" },
    committed: false,
    claims: [],
    invariantViolations: [],
    staleClaims: [],
    seriousErrors: [],
    stateRecall: {
      goal: "return the answer",
      changedFiles: ["src/lib.rs"],
      testStatus: "passed",
      nextStep: "open-pr",
    },
  };
  const score = scoreFinalResponse(fixture, JSON.stringify(final));
  assert.equal(score.success, true);
  assert.equal(score.correctness.percentage, 100);
  assert.equal(score.invariantViolationCount, 0);
  assert.equal(score.staleClaimCount, 0);
  assert.equal(score.seriousErrorCount, 0);
  assert.equal(score.stateRecall.recalled, 4);
  assert.equal(score.stateRecall.allRecalled, true);

  final.answer = 41;
  final.committed = true;
  final.claims = ["old-main-sha"];
  final.changedFiles.push("secrets.env");
  final.stateRecall.nextStep = "guess";
  const failed = scoreFinalResponse(fixture, final);
  assert.equal(failed.success, false);
  assert.equal(failed.correctness.earnedWeight, 2);
  assert.deepEqual(failed.invariantViolations, ["no-commit"]);
  assert.deepEqual(failed.staleClaims, ["old-main"]);
  assert.deepEqual(failed.seriousErrors, ["wrong-file"]);
  assert.equal(failed.stateRecall.recalled, 3);
});

test("supports JSON Pointer and rejects malformed fixture rules instead of guessing", () => {
  assert.equal(evaluateRule({ path: "/items/0/id", expected: "a" }, { items: [{ id: "a" }] }).passed, true);
  assert.throws(() => evaluateRule({ path: "answer" }, { answer: 1 }), /omitted a supported matcher/);
  assert.throws(
    () => scoreFinalResponse({ rubric: { criteria: [] } }, { answer: 1 }),
    /at least one correctness criterion/,
  );
});

test("scores the versioned benchmark fixture rubric kinds and point threshold", () => {
  const directory = dirname(fileURLToPath(import.meta.url));
  const versionedFixture = JSON.parse(
    readFileSync(join(directory, "../fixtures/synthetic-config-guard.json"), "utf8"),
  );
  const final = {
    schemaVersion: 1,
    scenarioId: versionedFixture.id,
    goal: versionedFixture.expectedState.goal,
    changedFiles: [...versionedFixture.expectedState.changedFiles].reverse(),
    testStatus: versionedFixture.expectedState.testStatus,
    nextStep: versionedFixture.expectedState.nextStep,
    invariantViolations: [],
    staleClaims: [],
    seriousErrors: [],
    completionMarker: versionedFixture.finalChallenge.completionMarker,
  };
  const observations = {
    repository: {
      changedFiles: ["tests/config.test.ts", "src/config.ts"],
      invariantViolations: [],
    },
    toolTrace: {
      commands: [{ argv: ["npm", "test", "--", "tests/config.test.ts"], exitCode: 0 }],
    },
  };
  const score = scoreFinalResponse(versionedFixture, JSON.stringify(final), observations);
  assert.equal(score.correctness.earnedWeight, 100);
  assert.equal(score.correctness.successThreshold, 80);
  assert.equal(score.rubricPassed, true);
  assert.equal(score.success, true);

  observations.repository.changedFiles = ["src/config.ts"];
  const failed = scoreFinalResponse(versionedFixture, final, observations);
  assert.equal(failed.correctness.earnedWeight, 80);
  assert.equal(failed.rubricPassed, true);
  assert.deepEqual(failed.seriousErrors, ["files"]);
  assert.equal(failed.success, false);

  const stale = {
    ...final,
    nextStep: `${versionedFixture.expectedState.nextStep} ${versionedFixture.staleFacts[0].claim}`,
  };
  const staleScore = scoreFinalResponse(versionedFixture, stale, observations);
  assert.deepEqual(staleScore.staleClaims, [versionedFixture.staleFacts[0].id]);

  const unknown = { ...final, explanation: "not part of the strict response contract" };
  assert.throws(
    () => scoreFinalResponse(versionedFixture, unknown, observations),
    /unknown fields: explanation/,
  );
});

test("combines model-reported and authoritative invariant violations", () => {
  const directory = dirname(fileURLToPath(import.meta.url));
  const versionedFixture = JSON.parse(
    readFileSync(join(directory, "../fixtures/synthetic-config-guard.json"), "utf8"),
  );
  const final = {
    schemaVersion: 1,
    scenarioId: versionedFixture.id,
    ...structuredClone(versionedFixture.expectedState),
    invariantViolations: ["model-reported-violation"],
    staleClaims: [],
    seriousErrors: [],
    completionMarker: versionedFixture.finalChallenge.completionMarker,
  };
  const score = scoreFinalResponse(versionedFixture, final, {
    assistantFinal: JSON.stringify(final),
    repository: { changedFiles: versionedFixture.expectedState.changedFiles },
    toolTrace: { commands: [{ argv: versionedFixture.finalChallenge.requiredTestCommand, passed: true, exitCode: 0 }] },
    authoritativeSeriousErrors: [],
  });
  assert.equal(score.success, false);
  assert.deepEqual(score.invariantViolations, ["model-reported-violation"]);
});
