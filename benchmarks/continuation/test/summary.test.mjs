import assert from "node:assert/strict";
import test from "node:test";

import {
  buildContinuationSummary,
  collectCompletedArms,
  parseAppendOnlyResults,
  renderSummaryMarkdown,
} from "../src/summary.mjs";
import { sha256, stableStringify } from "../src/io.mjs";

const TEST_MANIFEST = {
  schemaVersion: 1,
  benchmarkId: "summary-test",
  prerequisite: { mergeSha: "a".repeat(40) },
  fixtureSet: { expectedSha256: "e".repeat(64) },
  matrix: {
    scenarioCount: 8,
    repetitions: 3,
    initialStrategies: ["same_task", "native_handoff"],
    compactionCheckpoints: [0, 1, 2, 3, 4],
    expectedBaseMeasuredArms: 240,
  },
};

function sealed(record) {
  return { ...record, recordSha256: sha256(stableStringify(record)) };
}

function arm({
  checkpoint,
  scenario,
  repetition,
  strategy,
  success,
  serious = 0,
  latency = 100,
  completion = latency,
  endToEnd = latency,
  stateRecall = { goal: true, changedFiles: true, testStatus: true, nextStep: true },
  nativeRecord = null,
}) {
  const fixtureSha256 = "f".repeat(64);
  const nativeArmKey = ["gpt-test", scenario, "native_handoff", checkpoint, repetition, fixtureSha256].join("/");
  const handoffSha256 = sha256(`handoff/${scenario}/${checkpoint}/${repetition}`);
  const checkpointSha256 = sha256(`handoff-checkpoint/${scenario}/${checkpoint}/${repetition}`);
  const handoff = strategy === "same_task" ? null : {
    sha256: handoffSha256,
    deliveredSha256: handoffSha256,
    checkpointSha256,
    byteLength: 128,
    reusedFromNativeArmKey: strategy === "context_pack" ? nativeArmKey : null,
    nativeResultRecordSha256: strategy === "context_pack" ? nativeRecord?.recordSha256 ?? null : null,
  };
  return sealed({
    schemaVersion: 1,
    event: "arm_completed",
    recordedAt: "2026-07-15T00:00:00.000Z",
    payload: {
      phase: "measured",
      arm: {
        model: "gpt-test",
        scenario,
        strategy,
        compaction: checkpoint,
        repetition,
        fixtureSha256,
        key: ["gpt-test", scenario, strategy, checkpoint, repetition, fixtureSha256].join("/"),
      },
      binding: {
        campaignLockSha256: "c".repeat(64),
        fixtureSha256,
        sourceSnapshotSha256: sha256(`source/${scenario}/${checkpoint}/${repetition}`),
        fixtureSetSha256: TEST_MANIFEST.fixtureSet.expectedSha256,
        manifestSha256: sha256(stableStringify(TEST_MANIFEST)),
        prerequisiteMergeSha: TEST_MANIFEST.prerequisite.mergeSha,
      },
      model: {
        requested: "gpt-test",
        actualSnapshotId: "gpt-test-2026-07-15",
        allPaidStagesIdentified: true,
      },
      metrics: {
        success,
        seriousErrorCount: serious,
        timing: { completionMs: completion, endToEndMs: endToEnd },
        stateRecall: {
          dimensions: Object.fromEntries(
            Object.entries(stateRecall).map(([dimension, recalled]) => [dimension, { recalled }]),
          ),
        },
      },
      handoff,
    },
  });
}

test("parses JSONL and deduplicates byte-identical completed arms", () => {
  const event = arm({ checkpoint: 0, scenario: "s1", repetition: 1, strategy: "same_task", success: true });
  const events = parseAppendOnlyResults(`${JSON.stringify(event)}\n${JSON.stringify(event)}\n`);
  assert.equal(collectCompletedArms(events).length, 1);
  assert.throws(() => parseAppendOnlyResults('{"ok":true}\nnot-json\n'), /line 2/);

  const siblingPayload = {
    type: "arm_completed",
    payload: {
      model: "gpt-sibling",
      metrics: { completion_latency_ms: 123 },
      arm: {
        scenario: "s1",
        strategy: "same_task",
        compaction: 0,
        repetition: 1,
        fixtureSha256: "a".repeat(64),
        status: "completed",
      },
    },
  };
  assert.equal(collectCompletedArms([siblingPayload]).length, 0);

  const runnerPayload = arm({ checkpoint: 0, scenario: "s1", repetition: 1, strategy: "same_task", success: true, latency: 75 });
  const [runnerArm] = collectCompletedArms([runnerPayload]);
  assert.equal(runnerArm.model, "gpt-test");
  assert.equal(runnerArm.modelIdentity.actualSnapshotId, "gpt-test-2026-07-15");
  assert.equal(runnerArm.metrics.timing.endToEndMs, 75);

  const calibration = structuredClone(runnerPayload);
  calibration.payload.phase = "calibration";
  const { recordSha256: _oldHash, ...calibrationBody } = calibration;
  calibration.recordSha256 = sha256(stableStringify(calibrationBody));
  assert.equal(collectCompletedArms([calibration]).length, 0);

  const corrupt = structuredClone(runnerPayload);
  corrupt.recordSha256 = "0".repeat(64);
  assert.throws(() => collectCompletedArms([corrupt]), /invalid record hash/);

  const otherCampaign = structuredClone(runnerPayload);
  otherCampaign.payload.arm.repetition = 2;
  otherCampaign.payload.arm.key = otherCampaign.payload.arm.key.replace("/1/", "/2/");
  otherCampaign.payload.binding.campaignLockSha256 = "d".repeat(64);
  const { recordSha256: _otherOld, ...otherBody } = otherCampaign;
  otherCampaign.recordSha256 = sha256(stableStringify(otherBody));
  assert.throws(() => collectCompletedArms([runnerPayload, otherCampaign]), /multiple campaign locks/);
});

test("emits per-checkpoint paired same-task vs native-handoff metrics and state recall", () => {
  const events = [];
  for (const scenario of Array.from({ length: 8 }, (_, index) => `s${index + 1}`)) {
    for (const repetition of [1, 2, 3]) {
      events.push(arm({
        checkpoint: 0,
        scenario,
        repetition,
        strategy: "same_task",
        success: false,
        serious: 1,
        completion: 100,
        endToEnd: 120,
        stateRecall: { goal: false, changedFiles: true, testStatus: false, nextStep: true },
      }));
      events.push(arm({
        checkpoint: 0,
        scenario,
        repetition,
        strategy: "native_handoff",
        success: true,
        serious: 0,
        completion: 80,
        endToEnd: 90,
        stateRecall: { goal: true, changedFiles: true, testStatus: true, nextStep: false },
      }));
    }
  }
  const output = buildContinuationSummary({ manifest: TEST_MANIFEST, events, bootstrap: { iterations: 500 } });
  const [comparison] = output.json.models["gpt-test"].strategyComparison;
  assert.equal(comparison.checkpoint, 0);
  assert.equal(comparison.baselineStrategy, "same_task");
  assert.equal(comparison.comparisonStrategy, "native_handoff");
  assert.equal(comparison.coverage.complete, true);
  assert.equal(comparison.pairedArmCount, 24);
  assert.equal(comparison.accuracyDifferencePp.estimate, 100);
  assert.equal(comparison.seriousErrorDifferencePp.estimate, -100);
  assert.equal(comparison.completionLatencyPercentChange.estimate, -20);
  assert.equal(comparison.endToEndLatencyPercentChange.estimate, -25);
  assert.equal(comparison.stateRecallDifferencePp.goal.estimate, 100);
  assert.equal(comparison.stateRecallDifferencePp.changedFiles.estimate, 0);
  assert.equal(comparison.stateRecallDifferencePp.testStatus.estimate, 100);
  assert.equal(comparison.stateRecallDifferencePp.nextStep.estimate, -100);
  assert.equal(comparison.accuracyDifferencePp.confidence, 0.95);
  assert.match(output.markdown, /native_handoff - same_task/);
  assert.match(output.markdown, /Goal pp/);
  assert.match(output.markdown, /100 \[100, 100\]/);
});

test("emits model-specific threshold only after boundary and product gate pass", () => {
  const events = [];
  for (const scenario of Array.from({ length: 8 }, (_, index) => `s${index + 1}`)) {
    for (const repetition of [1, 2, 3]) {
      const nativeByCheckpoint = new Map();
      for (const checkpoint of [0, 1, 2, 3, 4]) {
        const degraded = checkpoint > 0;
        events.push(arm({
          checkpoint,
          scenario,
          repetition,
          strategy: "same_task",
          success: !degraded,
          serious: degraded ? 1 : 0,
          latency: degraded ? 140 + checkpoint : 100,
        }));
        const native = arm({ checkpoint, scenario, repetition, strategy: "native_handoff", success: false, latency: 100 });
        events.push(native);
        nativeByCheckpoint.set(checkpoint, native);
      }
      events.push(arm({
        checkpoint: 1,
        scenario,
        repetition,
        strategy: "context_pack",
        success: true,
        latency: 75,
        nativeRecord: nativeByCheckpoint.get(1),
      }));
      events.push(arm({
        checkpoint: 2,
        scenario,
        repetition,
        strategy: "context_pack",
        success: true,
        latency: 75,
        nativeRecord: nativeByCheckpoint.get(2),
      }));
    }
  }
  const output = buildContinuationSummary({
    manifest: TEST_MANIFEST,
    events,
    bootstrap: { iterations: 500 },
  });
  const model = output.json.models["gpt-test"];
  assert.equal(model.degradationBoundary.detected, true);
  assert.equal(model.productGate.passed, true);
  assert.equal(model.recommendation.action, "propose_auto_rollover");
  assert.equal(model.recommendation.threshold, 1);
  assert.equal(output.json.recommendation.action, "model_specific_thresholds_only");
  assert.match(output.markdown, /n=3 repeats/);
  assert.match(output.markdown, /gpt-test/);

  const mismatchedHandoffEvents = structuredClone(events);
  const mismatchedProduct = mismatchedHandoffEvents.find((event) =>
    event.payload.arm.strategy === "context_pack"
  );
  mismatchedProduct.payload.handoff.sha256 = "0".repeat(64);
  mismatchedProduct.payload.handoff.deliveredSha256 = "0".repeat(64);
  const { recordSha256: _handoffHash, ...mismatchedBody } = mismatchedProduct;
  mismatchedProduct.recordSha256 = sha256(stableStringify(mismatchedBody));
  const mismatchedHandoff = buildContinuationSummary({
    manifest: TEST_MANIFEST,
    events: mismatchedHandoffEvents,
    bootstrap: { iterations: 200 },
  });
  assert.equal(mismatchedHandoff.json.models["gpt-test"].productGate.passed, false);
  assert.equal(mismatchedHandoff.json.models["gpt-test"].productGate.handoffBinding.valid, false);
  assert.equal(mismatchedHandoff.json.models["gpt-test"].recommendation.action, "no_auto_rollover");
  assert.match(
    mismatchedHandoff.json.models["gpt-test"].productGate.reasons.join(" "),
    /product handoff provenance mismatch/,
  );

  const incompleteIdentityEvents = structuredClone(events);
  incompleteIdentityEvents[0].payload.model.allPaidStagesIdentified = false;
  const { recordSha256: _old, ...changedBody } = incompleteIdentityEvents[0];
  incompleteIdentityEvents[0].recordSha256 = sha256(stableStringify(changedBody));
  const incompleteIdentity = buildContinuationSummary({
    manifest: TEST_MANIFEST,
    events: incompleteIdentityEvents,
    bootstrap: { iterations: 200 },
  });
  assert.equal(incompleteIdentity.json.models["gpt-test"].snapshotStable, false);
  assert.equal(incompleteIdentity.json.models["gpt-test"].recommendation.action, "no_auto_rollover");
});

test("states no_auto_rollover when a boundary or product arm is unavailable", () => {
  const events = [
    arm({ checkpoint: 0, scenario: "s1", repetition: 1, strategy: "same_task", success: true }),
    arm({ checkpoint: 2, scenario: "s1", repetition: 1, strategy: "same_task", success: false }),
  ];
  const output = buildContinuationSummary({ manifest: TEST_MANIFEST, events, bootstrap: { iterations: 200 } });
  assert.equal(output.json.models["gpt-test"].recommendation.action, "no_auto_rollover");
  assert.equal(output.json.recommendation.action, "no_auto_rollover");
  assert.match(renderSummaryMarkdown(output.json), /no_auto_rollover/);
});
