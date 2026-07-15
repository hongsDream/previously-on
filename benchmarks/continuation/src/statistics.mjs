export const STATISTICAL_LIMITATIONS = Object.freeze([
  "Each model/scenario/strategy/checkpoint condition has only n=3 repeats; confidence intervals are low-power and may be wide.",
  "Scenario-repeat arms are paired for comparisons, but the eight scenarios are not assumed to be interchangeable population samples.",
  "Bootstrap intervals describe this fixed benchmark matrix and must not be generalized to untested model snapshots or versions.",
]);

export const STATE_RECALL_DIMENSIONS = Object.freeze(["goal", "changedFiles", "testStatus", "nextStep"]);

export function seededBootstrap(values, statistic, options = {}) {
  if (!Array.isArray(values) || values.length === 0) return unavailable("no observations");
  if (typeof statistic !== "function") throw new TypeError("statistic must be a function");
  const iterations = options.iterations ?? 10_000;
  if (!Number.isInteger(iterations) || iterations < 100) {
    throw new Error("bootstrap iterations must be an integer >= 100");
  }
  const confidence = options.confidence ?? 0.95;
  if (!(confidence > 0 && confidence < 1)) throw new Error("confidence must be between 0 and 1");
  const seed = String(options.seed ?? "previously-on-continuation-v1");
  const random = mulberry32(hashSeed(seed));
  const estimate = statistic(values);
  if (!Number.isFinite(estimate)) return unavailable("statistic is unavailable for these observations");
  const estimates = new Array(iterations);
  for (let iteration = 0; iteration < iterations; iteration += 1) {
    const sample = new Array(values.length);
    for (let index = 0; index < values.length; index += 1) {
      sample[index] = values[Math.floor(random() * values.length)];
    }
    estimates[iteration] = statistic(sample);
  }
  const finite = estimates.filter(Number.isFinite).sort((left, right) => left - right);
  if (finite.length === 0) return unavailable("bootstrap resamples produced no finite statistics");
  const alpha = 1 - confidence;
  return {
    available: true,
    estimate,
    lower: quantile(finite, alpha / 2),
    upper: quantile(finite, 1 - alpha / 2),
    confidence,
    method: "seeded_paired_percentile_bootstrap",
    iterations,
    seed,
    n: values.length,
  };
}

export function pairedAccuracyDifference(pairs, options = {}) {
  const normalized = normalizePairs(pairs);
  return clusteredPairedBootstrap(
    normalized,
    (sample) => mean(sample.map((pair) => binary(pair.comparison, options.comparisonSelector))) * 100 -
      mean(sample.map((pair) => binary(pair.baseline, options.baselineSelector))) * 100,
    { ...options, seed: options.seed ?? "accuracy-difference-v1" },
  );
}

export function pairedSeriousErrorDifference(pairs, options = {}) {
  const normalized = normalizePairs(pairs);
  return clusteredPairedBootstrap(
    normalized,
    (sample) => mean(sample.map((pair) => serious(pair.comparison, options.comparisonSelector))) * 100 -
      mean(sample.map((pair) => serious(pair.baseline, options.baselineSelector))) * 100,
    { ...options, seed: options.seed ?? "serious-error-difference-v1" },
  );
}

export function pairedMedianLatencyPercentChange(pairs, options = {}) {
  const allPairs = normalizePairs(pairs);
  const normalized = allPairs.filter((pair) => {
    const baseline = latency(pair.baseline, options.baselineSelector);
    const comparison = latency(pair.comparison, options.comparisonSelector);
    return Number.isFinite(baseline) && baseline > 0 && Number.isFinite(comparison) && comparison >= 0;
  });
  if (normalized.length !== allPairs.length) {
    return unavailable(`latency coverage is incomplete: ${normalized.length}/${allPairs.length} paired observations`);
  }
  return clusteredPairedBootstrap(
    normalized,
    (sample) => {
      const baseline = median(sample.map((pair) => latency(pair.baseline, options.baselineSelector)));
      const comparison = median(sample.map((pair) => latency(pair.comparison, options.comparisonSelector)));
      return baseline > 0 ? ((comparison - baseline) / baseline) * 100 : Number.NaN;
    },
    { ...options, seed: options.seed ?? "latency-percent-change-v1" },
  );
}

export function pairedStateRecallDifference(pairs, dimension, options = {}) {
  if (!STATE_RECALL_DIMENSIONS.includes(dimension)) {
    throw new Error(`unsupported state-recall dimension ${dimension}`);
  }
  const allPairs = normalizePairs(pairs);
  const normalized = allPairs.filter((pair) =>
    recallValue(pair.baseline, dimension) !== null && recallValue(pair.comparison, dimension) !== null,
  );
  if (normalized.length !== allPairs.length) {
    return unavailable(
      `state-recall coverage for ${dimension} is incomplete: ${normalized.length}/${allPairs.length} paired observations`,
    );
  }
  return clusteredPairedBootstrap(
    normalized,
    (sample) => mean(sample.map((pair) => recallValue(pair.comparison, dimension))) * 100 -
      mean(sample.map((pair) => recallValue(pair.baseline, dimension))) * 100,
    { ...options, seed: options.seed ?? `state-recall-${dimension}-difference-v1` },
  );
}

export function comparePairedStrategies(pairs, options = {}) {
  const normalized = normalizePairs(pairs);
  const comparisonOptions = { ...options, confidence: 0.95 };
  const coverage = pairCoverage(normalized, comparisonOptions.expectedPairCount);
  if (!coverage.complete) {
    const missing = unavailable(coverage.reason);
    return {
      coverage,
      accuracyDifferencePp: missing,
      seriousErrorDifferencePp: missing,
      completionLatencyPercentChange: missing,
      endToEndLatencyPercentChange: missing,
      stateRecallDifferencePp: Object.fromEntries(
        STATE_RECALL_DIMENSIONS.map((dimension) => [dimension, missing]),
      ),
    };
  }
  return {
    coverage,
    accuracyDifferencePp: pairedAccuracyDifference(normalized, {
      ...comparisonOptions,
      seed: comparisonOptions.accuracySeed ?? `strategy-accuracy-${comparisonOptions.seed ?? "v1"}`,
    }),
    seriousErrorDifferencePp: pairedSeriousErrorDifference(normalized, {
      ...comparisonOptions,
      seed: comparisonOptions.seriousSeed ?? `strategy-serious-${comparisonOptions.seed ?? "v1"}`,
    }),
    completionLatencyPercentChange: pairedMedianLatencyPercentChange(normalized, {
      ...comparisonOptions,
      seed: comparisonOptions.completionSeed ?? `strategy-completion-${comparisonOptions.seed ?? "v1"}`,
      baselineSelector: (value) => metric(value, "completionLatencyMs"),
      comparisonSelector: (value) => metric(value, "completionLatencyMs"),
    }),
    endToEndLatencyPercentChange: pairedMedianLatencyPercentChange(normalized, {
      ...comparisonOptions,
      seed: comparisonOptions.endToEndSeed ?? `strategy-end-to-end-${comparisonOptions.seed ?? "v1"}`,
      baselineSelector: (value) => metric(value, "endToEndLatencyMs"),
      comparisonSelector: (value) => metric(value, "endToEndLatencyMs"),
    }),
    stateRecallDifferencePp: Object.fromEntries(
      STATE_RECALL_DIMENSIONS.map((dimension) => [
        dimension,
        pairedStateRecallDifference(normalized, dimension, {
          ...comparisonOptions,
          seed: `strategy-state-recall-${dimension}-${comparisonOptions.seed ?? "v1"}`,
        }),
      ]),
    ),
  };
}

export function detectDegradationBoundary(checkpoints, options = {}) {
  if (!Array.isArray(checkpoints)) throw new TypeError("checkpoints must be an array");
  const baselineCheckpoint = options.baselineCheckpoint ?? 0;
  const sorted = [...checkpoints].sort((left, right) => left.checkpoint - right.checkpoint);
  const baseline = sorted.find((entry) => entry.checkpoint === baselineCheckpoint);
  if (!baseline) {
    return {
      detected: false,
      boundaryCheckpoint: null,
      confirmedAtCheckpoint: null,
      checks: [],
      reason: `baseline checkpoint ${baselineCheckpoint} is unavailable`,
      statisticalLimitations: STATISTICAL_LIMITATIONS,
    };
  }
  const checks = sorted
    .filter((entry) => entry.checkpoint > baselineCheckpoint)
    .map((entry) => checkpointComparison(baseline, entry, options));
  let boundary = null;
  for (let index = 1; index < checks.length; index += 1) {
    const commonMetrics = checks[index - 1].supportedBreaches.filter((metric) =>
      checks[index].supportedBreaches.includes(metric),
    );
    if (commonMetrics.length > 0) {
      boundary = { first: checks[index - 1], second: checks[index], metrics: commonMetrics };
      break;
    }
  }
  return {
    detected: Boolean(boundary),
    boundaryCheckpoint: boundary?.first.checkpoint ?? null,
    confirmedAtCheckpoint: boundary?.second.checkpoint ?? null,
    confirmingMetrics: boundary?.metrics ?? [],
    checks,
    reason: boundary
      ? "the same degradation metric crossed its point threshold and adverse-side 95% CI at two consecutive checkpoints"
      : "no metric crossed its point threshold and adverse-side 95% CI at two consecutive checkpoints; isolated or uncertain spikes were not adopted",
    thresholds: { successDropPp: -10, seriousErrorIncreasePp: 10, medianLatencyIncreasePercent: 25 },
    statisticalLimitations: STATISTICAL_LIMITATIONS,
  };
}

export function evaluateProductGate(pairs, options = {}) {
  const coverage = pairCoverage(pairs, options.expectedPairCount);
  const accuracy = pairedAccuracyDifference(pairs, {
    ...options,
    seed: options.accuracySeed ?? options.seed ?? "product-gate-accuracy-v1",
  });
  const seriousErrors = pairedSeriousErrorDifference(pairs, {
    ...options,
    seed: options.seriousSeed ?? options.seed ?? "product-gate-serious-v1",
  });
  const latencyChange = pairedMedianLatencyPercentChange(pairs, {
    ...options,
    seed: options.latencySeed ?? options.seed ?? "product-gate-latency-v1",
    baselineSelector: options.baselineLatencySelector,
    comparisonSelector: options.comparisonLatencySelector,
  });
  const accuracyNonInferior = accuracy.available && accuracy.lower > -5;
  const noSeriousErrorIncrease =
    seriousErrors.available && seriousErrors.estimate <= 0 && seriousErrors.upper <= 0;
  const meaningfulAccuracyGain = accuracy.available && accuracy.estimate >= 5;
  const meaningfulLatencyGain = latencyChange.available && latencyChange.estimate <= -15;
  const passed =
    coverage.complete && accuracyNonInferior && noSeriousErrorIncrease && (meaningfulAccuracyGain || meaningfulLatencyGain);
  const reasons = [];
  if (!coverage.complete) reasons.push(coverage.reason);
  if (!accuracyNonInferior) reasons.push("lower accuracy CI is not greater than -5pp");
  if (!noSeriousErrorIncrease) reasons.push("serious-error point estimate or upper 95% CI exceeded zero, or was unavailable");
  if (!meaningfulAccuracyGain && !meaningfulLatencyGain) {
    reasons.push("neither accuracy improved by at least 5pp nor median latency improved by at least 15%");
  }
  return {
    passed,
    recommendation: passed ? "product_arm_gate_passed" : "no_auto_rollover",
    accuracyDifferencePp: accuracy,
    seriousErrorDifferencePp: seriousErrors,
    medianLatencyPercentChange: latencyChange,
    criteria: {
      accuracyNonInferior,
      noSeriousErrorIncrease,
      meaningfulAccuracyGain,
      meaningfulLatencyGain,
    },
    reasons,
    coverage,
    statisticalLimitations: STATISTICAL_LIMITATIONS,
  };
}

export function additionalCheckpoints(boundary, evaluatedCheckpoints) {
  const evaluated = new Set((evaluatedCheckpoints ?? []).map(Number));
  if (boundary?.detected) {
    const first = Number(boundary.boundaryCheckpoint);
    return [first - 1, first + 1]
      .filter((checkpoint) => checkpoint >= 0 && checkpoint % 2 === 1 && !evaluated.has(checkpoint));
  }
  const maximum = evaluated.size === 0 ? Number.NEGATIVE_INFINITY : Math.max(...evaluated);
  return maximum >= 16 ? [18, 20].filter((checkpoint) => !evaluated.has(checkpoint)) : [];
}

export function pairObservations(baseline, comparison, key = defaultPairKey) {
  const comparisonByKey = new Map(comparison.map((item, index) => [key(item, index), item]));
  return baseline.flatMap((item, index) => {
    const pairKey = key(item, index);
    return comparisonByKey.has(pairKey)
      ? [{ key: pairKey, baseline: item, comparison: comparisonByKey.get(pairKey) }]
      : [];
  });
}

export function median(values) {
  const finite = values.filter(Number.isFinite).sort((left, right) => left - right);
  if (finite.length === 0) return Number.NaN;
  const middle = Math.floor(finite.length / 2);
  return finite.length % 2 === 0 ? (finite[middle - 1] + finite[middle]) / 2 : finite[middle];
}

function clusteredPairedBootstrap(pairs, statistic, options) {
  if (pairs.length === 0) return unavailable("no paired observations");
  const clusters = new Map();
  for (const [index, pair] of pairs.entries()) {
    const cluster = options.clusterKey
      ? options.clusterKey(pair, index)
      : pair.baseline?.scenarioId ?? pair.baseline?.scenario ??
        pair.comparison?.scenarioId ?? pair.comparison?.scenario ?? `pair-${index}`;
    const key = String(cluster);
    if (!clusters.has(key)) clusters.set(key, []);
    clusters.get(key).push(pair);
  }
  const result = seededBootstrap(
    [...clusters.values()],
    (sampledClusters) => statistic(sampledClusters.flat()),
    options,
  );
  if (!result.available) return result;
  return {
    ...result,
    method: "seeded_scenario_cluster_paired_percentile_bootstrap",
    n: pairs.length,
    clusterCount: clusters.size,
    repeatsPerCondition: 3,
  };
}

function checkpointComparison(baseline, comparison, options) {
  const pairs = pairObservations(
    baseline.observations ?? [],
    comparison.observations ?? [],
    options.pairKey ?? defaultPairKey,
  );
  const coverage = pairCoverage(pairs, options.expectedPairCount);
  if (!coverage.complete) {
    const missing = unavailable(coverage.reason);
    return {
      checkpoint: comparison.checkpoint,
      pairedArmCount: pairs.length,
      coverage,
      degraded: false,
      pointBreaches: [],
      supportedBreaches: [],
      successDifferencePp: missing,
      seriousErrorDifferencePp: missing,
      completionLatencyPercentChange: missing,
      endToEndLatencyPercentChange: missing,
    };
  }
  const success = pairedAccuracyDifference(pairs, {
    ...options,
    seed: `boundary-success-${comparison.checkpoint}-${options.seed ?? "v1"}`,
  });
  const seriousErrors = pairedSeriousErrorDifference(pairs, {
    ...options,
    seed: `boundary-serious-${comparison.checkpoint}-${options.seed ?? "v1"}`,
  });
  const completionLatency = pairedMedianLatencyPercentChange(pairs, {
    ...options,
    seed: `boundary-completion-${comparison.checkpoint}-${options.seed ?? "v1"}`,
    baselineSelector: (value) => metric(value, "completionLatencyMs"),
    comparisonSelector: (value) => metric(value, "completionLatencyMs"),
  });
  const endToEndLatency = pairedMedianLatencyPercentChange(pairs, {
    ...options,
    seed: `boundary-end-to-end-${comparison.checkpoint}-${options.seed ?? "v1"}`,
    baselineSelector: (value) => metric(value, "endToEndLatencyMs"),
    comparisonSelector: (value) => metric(value, "endToEndLatencyMs"),
  });
  const pointBreaches = [];
  const supportedBreaches = [];
  if (success.available && success.estimate <= -10) {
    pointBreaches.push("success_drop");
    if (success.upper <= -10) supportedBreaches.push("success_drop");
  }
  if (seriousErrors.available && seriousErrors.estimate >= 10) {
    pointBreaches.push("serious_error_increase");
    if (seriousErrors.lower >= 10) supportedBreaches.push("serious_error_increase");
  }
  if (completionLatency.available && completionLatency.estimate >= 25) {
    pointBreaches.push("completion_latency_increase");
    if (completionLatency.lower >= 25) supportedBreaches.push("completion_latency_increase");
  }
  if (endToEndLatency.available && endToEndLatency.estimate >= 25) {
    pointBreaches.push("end_to_end_latency_increase");
    if (endToEndLatency.lower >= 25) supportedBreaches.push("end_to_end_latency_increase");
  }
  return {
    checkpoint: comparison.checkpoint,
    pairedArmCount: pairs.length,
    coverage,
    degraded: supportedBreaches.length > 0,
    pointBreaches,
    supportedBreaches,
    successDifferencePp: success,
    seriousErrorDifferencePp: seriousErrors,
    completionLatencyPercentChange: completionLatency,
    endToEndLatencyPercentChange: endToEndLatency,
  };
}

function pairCoverage(pairs, expectedPairCount) {
  if (!Number.isInteger(expectedPairCount)) {
    return { complete: true, expectedPairCount: null, observedPairCount: pairs.length, uniquePairCount: pairs.length, reason: null };
  }
  const identities = pairs.map((pair, index) => pair.key ?? defaultPairKey(pair.baseline, index));
  const uniquePairCount = new Set(identities).size;
  const complete = pairs.length === expectedPairCount && uniquePairCount === expectedPairCount;
  return {
    complete,
    expectedPairCount,
    observedPairCount: pairs.length,
    uniquePairCount,
    reason: complete
      ? null
      : `condition coverage is incomplete: expected ${expectedPairCount} unique pairs, observed ${pairs.length} rows and ${uniquePairCount} unique pairs`,
  };
}

function normalizePairs(pairs) {
  if (!Array.isArray(pairs)) throw new TypeError("pairs must be an array");
  return pairs.map((pair, index) => {
    if (Array.isArray(pair) && pair.length === 2) return { baseline: pair[0], comparison: pair[1] };
    if (pair && Object.hasOwn(pair, "baseline") && Object.hasOwn(pair, "comparison")) return pair;
    throw new Error(`pair ${index} must contain baseline and comparison values`);
  });
}

function binary(value, selector) {
  const selected = selector ? selector(value) : success(value);
  if (typeof selected === "boolean") return selected ? 1 : 0;
  if (selected === 0 || selected === 1) return selected;
  throw new Error("accuracy observations must be boolean or 0/1");
}

function serious(value, selector) {
  const selected = selector ? selector(value) : seriousError(value);
  if (typeof selected === "boolean") return selected ? 1 : 0;
  if (selected === 0 || selected === 1) return selected;
  if (Number.isFinite(selected)) return selected > 0 ? 1 : 0;
  throw new Error("serious-error observations must be boolean or numeric");
}

function latency(value, selector) {
  if (selector) return selector(value);
  const endToEnd = metric(value, "endToEndLatencyMs");
  return Number.isFinite(endToEnd) ? endToEnd : metric(value, "completionLatencyMs");
}

function success(value) {
  return value?.metrics?.success ?? value?.score?.success ?? value?.scoring?.success ??
    value?.success ?? value?.rubricPassed ?? value;
}

function seriousError(value) {
  return value?.metrics?.seriousErrorCount ?? value?.score?.seriousErrorCount ??
    value?.scoring?.seriousErrorCount ??
    value?.seriousErrorCount ?? value?.seriousError ?? value;
}

function recallValue(value, dimension) {
  const stateRecall = value?.metrics?.stateRecall ?? value?.score?.stateRecall ??
    value?.scoring?.stateRecall ?? value?.stateRecall;
  const selected = stateRecall?.dimensions?.[dimension]?.recalled ?? stateRecall?.[dimension]?.recalled ??
    stateRecall?.[dimension];
  if (typeof selected === "boolean") return selected ? 1 : 0;
  if (selected === 0 || selected === 1) return selected;
  return null;
}

function metric(value, name) {
  const snakeName = name.replaceAll(/[A-Z]/g, (letter) => `_${letter.toLowerCase()}`);
  const timingName = name === "completionLatencyMs"
    ? "completionMs"
    : name === "endToEndLatencyMs" ? "endToEndMs" : name;
  const timingSnakeName = timingName.replaceAll(/[A-Z]/g, (letter) => `_${letter.toLowerCase()}`);
  const selected = value?.metrics?.timing?.[timingName] ?? value?.metrics?.timing?.[timingSnakeName] ??
    value?.metrics?.[name] ?? value?.metrics?.[snakeName] ??
    value?.latency?.[name] ?? value?.latency?.[snakeName] ?? value?.[name] ?? value?.[snakeName];
  return selected === null || selected === undefined ? Number.NaN : Number(selected);
}

function defaultPairKey(value, index) {
  const scenario = value?.scenarioId ?? value?.scenario;
  const repetition = value?.repetition ?? value?.repeat;
  const fixtureSha = value?.fixtureSha ?? value?.fixtureSHA ?? value?.fixtureSha256;
  if (scenario !== undefined && repetition !== undefined && fixtureSha !== undefined) {
    return `${scenario}/${repetition}/${fixtureSha}`;
  }
  const key = value?.checkpointKey ?? value?.key;
  if (typeof key === "string") {
    const parts = key.split("/");
    if (parts.length >= 6) return `${parts.at(-5)}/${parts.at(-2)}/${parts.at(-1)}`;
    return key;
  }
  return `observation-${index}`;
}

function mean(values) {
  return values.reduce((sum, value) => sum + value, 0) / values.length;
}

function quantile(sorted, probability) {
  if (sorted.length === 1) return sorted[0];
  const position = (sorted.length - 1) * probability;
  const lower = Math.floor(position);
  const upper = Math.ceil(position);
  const fraction = position - lower;
  return sorted[lower] + (sorted[upper] - sorted[lower]) * fraction;
}

function hashSeed(seed) {
  let hash = 2166136261;
  for (const character of seed) {
    hash ^= character.codePointAt(0);
    hash = Math.imul(hash, 16777619);
  }
  return hash >>> 0;
}

function mulberry32(seed) {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let value = state;
    value = Math.imul(value ^ (value >>> 15), value | 1);
    value ^= value + Math.imul(value ^ (value >>> 7), value | 61);
    return ((value ^ (value >>> 14)) >>> 0) / 4294967296;
  };
}

function unavailable(reason) {
  return {
    available: false,
    estimate: null,
    lower: null,
    upper: null,
    confidence: 0.95,
    method: "seeded_paired_percentile_bootstrap",
    iterations: 0,
    seed: null,
    n: 0,
    reason,
  };
}
