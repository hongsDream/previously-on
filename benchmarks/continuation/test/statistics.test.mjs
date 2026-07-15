import assert from "node:assert/strict";
import test from "node:test";

import {
  STATISTICAL_LIMITATIONS,
  additionalCheckpoints,
  comparePairedStrategies,
  detectDegradationBoundary,
  evaluateProductGate,
  pairedAccuracyDifference,
  pairedMedianLatencyPercentChange,
  pairedStateRecallDifference,
  seededBootstrap,
} from "../src/statistics.mjs";

test("seeded percentile bootstrap is byte-deterministic for the same seed", () => {
  const first = seededBootstrap([1, 2, 3], (values) => values.reduce((sum, value) => sum + value, 0) / values.length, {
    iterations: 1_000,
    seed: "fixed",
  });
  const second = seededBootstrap([1, 2, 3], (values) => values.reduce((sum, value) => sum + value, 0) / values.length, {
    iterations: 1_000,
    seed: "fixed",
  });
  assert.deepEqual(first, second);
  assert.equal(first.n, 3);
  assert.ok(first.lower <= first.estimate);
  assert.ok(first.upper >= first.estimate);
});

test("paired accuracy and median latency preserve comparison direction", () => {
  const pairs = [
    { baseline: { success: false, metrics: { endToEndLatencyMs: 100 } }, comparison: { success: true, metrics: { endToEndLatencyMs: 80 } } },
    { baseline: { success: true, metrics: { endToEndLatencyMs: 120 } }, comparison: { success: true, metrics: { endToEndLatencyMs: 90 } } },
    { baseline: { success: false, metrics: { endToEndLatencyMs: 110 } }, comparison: { success: true, metrics: { endToEndLatencyMs: 85 } } },
  ];
  assert.ok(pairedAccuracyDifference(pairs, { iterations: 1_000 }).estimate > 0);
  assert.ok(pairedMedianLatencyPercentChange(pairs, { iterations: 1_000 }).estimate <= -15);
});

test("paired strategy comparison reports both latency measures and all four state-recall dimensions", () => {
  const recalled = (values) => ({
    stateRecall: {
      dimensions: Object.fromEntries(Object.entries(values).map(([dimension, value]) => [dimension, { recalled: value }])),
    },
  });
  const pairs = Array.from({ length: 6 }, (_, index) => ({
    key: `s${Math.floor(index / 3)}/${index % 3}/fixture`,
    baseline: {
      scenarioId: `s${Math.floor(index / 3)}`,
      success: false,
      seriousErrorCount: 1,
      metrics: {
        completionLatencyMs: 100,
        endToEndLatencyMs: 120,
        ...recalled({ goal: false, changedFiles: true, testStatus: false, nextStep: true }),
      },
    },
    comparison: {
      scenarioId: `s${Math.floor(index / 3)}`,
      success: true,
      seriousErrorCount: 0,
      metrics: {
        completionLatencyMs: 80,
        endToEndLatencyMs: 90,
        ...recalled({ goal: true, changedFiles: true, testStatus: true, nextStep: false }),
      },
    },
  }));
  const comparison = comparePairedStrategies(pairs, {
    iterations: 500,
    expectedPairCount: 6,
    seed: "strategy",
    confidence: 0.8,
  });
  assert.equal(comparison.coverage.complete, true);
  assert.equal(comparison.accuracyDifferencePp.estimate, 100);
  assert.equal(comparison.seriousErrorDifferencePp.estimate, -100);
  assert.equal(comparison.completionLatencyPercentChange.estimate, -20);
  assert.equal(comparison.endToEndLatencyPercentChange.estimate, -25);
  assert.equal(comparison.stateRecallDifferencePp.goal.estimate, 100);
  assert.equal(comparison.stateRecallDifferencePp.changedFiles.estimate, 0);
  assert.equal(comparison.stateRecallDifferencePp.testStatus.estimate, 100);
  assert.equal(comparison.stateRecallDifferencePp.nextStep.estimate, -100);
  assert.equal(comparison.stateRecallDifferencePp.goal.confidence, 0.95);
  assert.throws(() => pairedStateRecallDifference(pairs, "unknown", { iterations: 500 }), /unsupported/);

  const missing = structuredClone(pairs);
  delete missing[0].comparison.metrics.stateRecall.dimensions.nextStep;
  const incomplete = comparePairedStrategies(missing, { iterations: 500, expectedPairCount: 6 });
  assert.equal(incomplete.stateRecallDifferencePp.nextStep.available, false);
  assert.equal(incomplete.stateRecallDifferencePp.goal.available, true);

  const partial = comparePairedStrategies(pairs.slice(0, 5), { iterations: 500, expectedPairCount: 6 });
  assert.equal(partial.coverage.complete, false);
  assert.equal(partial.accuracyDifferencePp.available, false);
});

test("degradation requires two consecutive threshold breaches and reports CIs", () => {
  const observation = (id, success, latency) => ({
    scenarioId: id,
    repetition: 1,
    fixtureSha: "a".repeat(64),
    success,
    seriousErrorCount: success ? 0 : 1,
    metrics: { completionLatencyMs: latency, endToEndLatencyMs: latency },
  });
  const baseline = ["a", "b", "c", "d"].map((id) => observation(id, true, 100));
  const spike = baseline.map((item) => ({ ...item, success: false, seriousErrorCount: 1, metrics: { completionLatencyMs: 150, endToEndLatencyMs: 150 } }));
  const recovered = baseline.map((item) => structuredClone(item));
  const isolated = detectDegradationBoundary([
    { checkpoint: 0, observations: baseline },
    { checkpoint: 2, observations: spike },
    { checkpoint: 4, observations: recovered },
  ], { iterations: 500 });
  assert.equal(isolated.detected, false);

  const confirmed = detectDegradationBoundary([
    { checkpoint: 0, observations: baseline },
    { checkpoint: 2, observations: spike },
    { checkpoint: 4, observations: spike },
  ], { iterations: 500 });
  assert.equal(confirmed.detected, true);
  assert.equal(confirmed.boundaryCheckpoint, 2);
  assert.equal(confirmed.confirmedAtCheckpoint, 4);
  assert.equal(confirmed.checks[0].successDifferencePp.available, true);
  assert.equal(confirmed.checks[0].successDifferencePp.method, "seeded_scenario_cluster_paired_percentile_bootstrap");
  assert.ok(confirmed.statisticalLimitations.some((item) => item.includes("n=3")));

  const successOnly = baseline.map((item) => ({
    ...structuredClone(item),
    success: false,
    seriousErrorCount: 0,
  }));
  const latencyOnly = baseline.map((item) => ({
    ...structuredClone(item),
    metrics: { completionLatencyMs: 150, endToEndLatencyMs: 150 },
  }));
  const alternatingMetrics = detectDegradationBoundary([
    { checkpoint: 0, observations: baseline },
    { checkpoint: 2, observations: successOnly },
    { checkpoint: 4, observations: latencyOnly },
  ], { iterations: 500 });
  assert.equal(alternatingMetrics.detected, false);
  assert.deepEqual(alternatingMetrics.checks[0].supportedBreaches, ["success_drop"]);
  assert.deepEqual(alternatingMetrics.checks[1].supportedBreaches, [
    "completion_latency_increase",
    "end_to_end_latency_increase",
  ]);

  const completionOnly = baseline.map((item) => ({
    ...structuredClone(item),
    metrics: { completionLatencyMs: 150, endToEndLatencyMs: 100 },
  }));
  const endToEndOnly = baseline.map((item) => ({
    ...structuredClone(item),
    metrics: { completionLatencyMs: 100, endToEndLatencyMs: 150 },
  }));
  const alternatingLatencyMetrics = detectDegradationBoundary([
    { checkpoint: 0, observations: baseline },
    { checkpoint: 2, observations: completionOnly },
    { checkpoint: 4, observations: endToEndOnly },
  ], { iterations: 500 });
  assert.equal(alternatingLatencyMetrics.detected, false);
  assert.deepEqual(alternatingLatencyMetrics.checks[0].supportedBreaches, ["completion_latency_increase"]);
  assert.deepEqual(alternatingLatencyMetrics.checks[1].supportedBreaches, ["end_to_end_latency_increase"]);
});

test("point threshold without an adverse-side confidence interval is not supported degradation", () => {
  const baseline = Array.from({ length: 8 }, (_, index) => ({
    scenarioId: `s${index}`,
    repetition: 1,
    fixtureSha: "b".repeat(64),
    success: true,
    seriousErrorCount: 0,
    metrics: { completionLatencyMs: 100, endToEndLatencyMs: 100 },
  }));
  const uncertain = baseline.map((item, index) => ({ ...item, success: index !== 0 }));
  const result = detectDegradationBoundary([
    { checkpoint: 0, observations: baseline },
    { checkpoint: 2, observations: uncertain },
    { checkpoint: 4, observations: uncertain },
  ], { iterations: 2_000, seed: "uncertain" });
  assert.ok(result.checks[0].pointBreaches.includes("success_drop"));
  assert.ok(!result.checks[0].supportedBreaches.includes("success_drop"));
  assert.equal(result.detected, false);
});

test("product gate enforces non-inferiority, no serious increase, and material improvement", () => {
  const passing = Array.from({ length: 6 }, (_, index) => ({
    baseline: { success: false, seriousErrorCount: 0, metrics: { endToEndLatencyMs: 100 + index } },
    comparison: { success: true, seriousErrorCount: 0, metrics: { endToEndLatencyMs: 75 + index } },
  }));
  const gate = evaluateProductGate(passing, { iterations: 1_000 });
  assert.equal(gate.passed, true);
  assert.equal(gate.recommendation, "product_arm_gate_passed");

  const failing = passing.map((pair) => ({
    baseline: { ...pair.baseline, success: true },
    comparison: { ...pair.comparison, success: false, seriousErrorCount: 1 },
  }));
  const rejected = evaluateProductGate(failing, { iterations: 1_000 });
  assert.equal(rejected.passed, false);
  assert.equal(rejected.recommendation, "no_auto_rollover");
  assert.deepEqual(rejected.statisticalLimitations, STATISTICAL_LIMITATIONS);
});

test("adaptive schedule extends through 20 or refines an even-grid boundary with odd checkpoints", () => {
  assert.deepEqual(additionalCheckpoints({ detected: false }, [0, 2, 4, 6, 8, 10, 12, 14, 16]), [18, 20]);
  assert.deepEqual(additionalCheckpoints({ detected: false }, [0, 2, 4, 6, 8, 10, 12, 14]), []);
  assert.deepEqual(
    additionalCheckpoints({ detected: true, boundaryCheckpoint: 6, confirmedAtCheckpoint: 8 }, [0, 2, 4, 6, 8]),
    [5, 7],
  );
});

test("reads scored and timing values from the runner terminal payload shape", () => {
  const pairs = Array.from({ length: 6 }, (_, index) => ({
    baseline: {
      scenarioId: `s${Math.floor(index / 3)}`,
      metrics: {
        success: false,
        seriousErrorCount: 0,
        timing: { completionMs: 120 + index, endToEndMs: 140 + index },
      },
    },
    comparison: {
      scenarioId: `s${Math.floor(index / 3)}`,
      metrics: {
        success: true,
        seriousErrorCount: 0,
        timing: { completionMs: 80 + index, endToEndMs: 90 + index },
      },
    },
  }));
  assert.ok(pairedAccuracyDifference(pairs, { iterations: 500 }).estimate > 0);
  assert.ok(pairedMedianLatencyPercentChange(pairs, { iterations: 500 }).estimate <= -15);
  assert.equal(evaluateProductGate(pairs, { iterations: 500 }).passed, true);
});

test("condition and latency coverage fail closed instead of using partial pairs", () => {
  const partial = [{
    key: "s1/1/f",
    baseline: { scenarioId: "s1", success: false, seriousErrorCount: 0, metrics: { endToEndLatencyMs: 100 } },
    comparison: { scenarioId: "s1", success: true, seriousErrorCount: 0, metrics: { endToEndLatencyMs: 75 } },
  }];
  const gate = evaluateProductGate(partial, { iterations: 500, expectedPairCount: 48 });
  assert.equal(gate.passed, false);
  assert.equal(gate.coverage.complete, false);

  const missingLatency = structuredClone(partial);
  delete missingLatency[0].comparison.metrics.endToEndLatencyMs;
  const latency = pairedMedianLatencyPercentChange(missingLatency, { iterations: 500 });
  assert.equal(latency.available, false);
  assert.match(latency.reason, /latency coverage is incomplete/);
});
