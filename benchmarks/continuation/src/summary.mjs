import {
  STATISTICAL_LIMITATIONS,
  additionalCheckpoints,
  comparePairedStrategies,
  detectDegradationBoundary,
  evaluateProductGate,
  pairObservations,
} from "./statistics.mjs";
import {
  closeSync,
  fsyncSync,
  mkdirSync,
  openSync,
  readFileSync,
  renameSync,
  writeFileSync,
} from "node:fs";
import { basename, dirname, join } from "node:path";
import { sha256, stableStringify } from "./io.mjs";

export function parseAppendOnlyResults(contents) {
  if (typeof contents !== "string") throw new TypeError("append-only results must be a string");
  const events = [];
  for (const [index, line] of contents.split(/\r?\n/).entries()) {
    if (!line.trim()) continue;
    try {
      const event = JSON.parse(line);
      if (!event || typeof event !== "object" || Array.isArray(event)) {
        throw new Error("event must be an object");
      }
      events.push(event);
    } catch (error) {
      throw new Error(`invalid append-only result at line ${index + 1}: ${error.message}`);
    }
  }
  return events;
}

export function collectCompletedArms(events, { phase = "measured", manifest = null } = {}) {
  if (!Array.isArray(events)) throw new TypeError("events must be an array");
  const completed = new Map();
  let campaignLockSha256 = null;
  for (const event of events) {
    if (!["arm_completed", "arm_model_error"].includes(event?.event)) continue;
    validateTerminalRecord(event, manifest);
    const observedLock = event.payload.binding.campaignLockSha256;
    if (campaignLockSha256 !== null && observedLock !== campaignLockSha256) {
      throw new Error("append-only results mix multiple campaign locks");
    }
    campaignLockSha256 = observedLock;
    const candidate = normalizeArm(event);
    if (!candidate) continue;
    if (candidate.phase !== undefined && candidate.phase !== phase) continue;
    const key = armKey(candidate);
    if (!key) throw new Error("completed arm omitted its model/scenario/strategy/compaction/repetition/fixture identity");
    const normalized = { ...candidate, checkpointKey: candidate.checkpointKey ?? key };
    if (completed.has(key)) {
      const previous = JSON.stringify(completed.get(key));
      const current = JSON.stringify(normalized);
      if (previous !== current) throw new Error(`completed arm ${key} was recorded with conflicting bytes`);
      continue;
    }
    completed.set(key, normalized);
  }
  return [...completed.values()].sort(compareArms);
}

export function buildContinuationSummary({ manifest = {}, events, bootstrap = {} }) {
  const arms = collectCompletedArms(events, { manifest });
  const expectedArms =
    manifest.expectedMeasuredArms ?? manifest.expectedArmCount ?? manifest.matrix?.expectedArms ??
    manifest.matrix?.expectedBaseMeasuredArms ?? null;
  const models = [...new Set(arms.map(modelName).filter(Boolean))].sort();
  const modelSummaries = {};
  for (const model of models) {
    modelSummaries[model] = summarizeModel(
      model,
      arms.filter((arm) => modelName(arm) === model),
      bootstrap,
      manifest,
    );
  }
  const modelRecommendations = Object.values(modelSummaries).map((item) => item.recommendation.action);
  const summary = {
    schemaVersion: 1,
    evidenceClass: "continuation_benchmark_derived_summary",
    fixtureSha256: manifest.fixtureSet?.expectedSha256 ?? manifest.fixtureSha256 ?? manifest.fixtureSHA ?? null,
    completedArms: arms.length,
    expectedArms,
    remainingArms: Number.isInteger(expectedArms) ? Math.max(0, expectedArms - arms.length) : null,
    models: modelSummaries,
    recommendation: {
      action:
        modelRecommendations.length > 0 && modelRecommendations.every((action) => action === "propose_auto_rollover")
          ? "model_specific_thresholds_only"
          : "no_auto_rollover",
      untestedModels: "no recommendation",
    },
    statisticalLimitations: STATISTICAL_LIMITATIONS,
  };
  return { json: summary, markdown: renderSummaryMarkdown(summary) };
}

export function renderSummaryMarkdown(summary) {
  const lines = [
    "# Continuation benchmark recommendation",
    "",
    `- Completed measured arms: ${summary.completedArms}${summary.expectedArms === null ? "" : ` / ${summary.expectedArms}`}`,
    `- Overall recommendation: \`${summary.recommendation.action}\``,
    "- Recommendations are model-specific; untested model snapshots receive no threshold.",
    "",
  ];
  for (const [model, result] of Object.entries(summary.models)) {
    lines.push(`## ${model}`, "");
    lines.push(`- Result: \`${result.recommendation.action}\``);
    lines.push(`- Completed arms: ${result.completedArms}`);
    if (result.baseMatrix.expectedArms !== null) {
      lines.push(`- Base matrix: ${result.baseMatrix.completedArms} / ${result.baseMatrix.expectedArms}`);
    }
    lines.push(
      result.degradationBoundary.detected
        ? `- Degradation boundary: checkpoint ${result.degradationBoundary.boundaryCheckpoint}, confirmed at ${result.degradationBoundary.confirmedAtCheckpoint}`
        : `- Degradation boundary: not established (${result.degradationBoundary.reason})`,
    );
    lines.push(`- Product arm gate: ${result.productGate?.passed ? "passed" : "not passed or unavailable"}`);
    if (result.recommendation.threshold !== null) {
      lines.push(`- Proposed automatic rollover checkpoint: ${result.recommendation.threshold}`);
    }
    for (const reason of result.recommendation.reasons) lines.push(`- Reason: ${reason}`);
    if (result.strategyComparison.length > 0) {
      lines.push("", "### Paired same-task vs native-handoff comparison", "");
      lines.push(
        "All differences use `native_handoff - same_task`; positive latency means native handoff is slower.",
        "",
        "| Checkpoint | Pairs | Accuracy pp | Serious error pp | Completion % | E2E % | Goal pp | Files pp | Tests pp | Next step pp |",
        "| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
      );
      for (const comparison of result.strategyComparison) {
        lines.push([
          comparison.checkpoint,
          `${comparison.coverage.observedPairCount}/${comparison.coverage.expectedPairCount ?? "?"}`,
          formatInterval(comparison.accuracyDifferencePp),
          formatInterval(comparison.seriousErrorDifferencePp),
          formatInterval(comparison.completionLatencyPercentChange),
          formatInterval(comparison.endToEndLatencyPercentChange),
          formatInterval(comparison.stateRecallDifferencePp.goal),
          formatInterval(comparison.stateRecallDifferencePp.changedFiles),
          formatInterval(comparison.stateRecallDifferencePp.testStatus),
          formatInterval(comparison.stateRecallDifferencePp.nextStep),
        ].join(" | ").replace(/^/u, "| ").replace(/$/u, " |"));
      }
    }
    lines.push("");
  }
  lines.push("## Statistical limitations", "");
  for (const limitation of summary.statisticalLimitations) lines.push(`- ${limitation}`);
  lines.push("");
  return `${lines.join("\n").trimEnd()}\n`;
}

export function summarizeResultFiles({ resultsPath, manifest, jsonPath, markdownPath, bootstrap }) {
  const events = parseAppendOnlyResults(readFileSync(resultsPath, "utf8"));
  const output = buildContinuationSummary({ manifest, events, bootstrap });
  writeAtomic(jsonPath, `${JSON.stringify(output.json, null, 2)}\n`);
  writeAtomic(markdownPath, output.markdown);
  return output;
}

function summarizeModel(model, arms, bootstrap, manifest) {
  const baseMatrix = baseMatrixCompletion(arms, manifest);
  const expectedPairsPerCheckpoint = Number.isInteger(manifest.matrix?.scenarioCount) &&
    Number.isInteger(manifest.matrix?.repetitions)
    ? manifest.matrix.scenarioCount * manifest.matrix.repetitions
    : null;
  const sameTask = arms.filter((arm) => strategyName(arm) === "same_task");
  const nativeHandoff = arms.filter((arm) => strategyName(arm) === "native_handoff");
  const checkpoints = [...new Set(sameTask.map(compactionCheckpoint))]
    .filter(Number.isFinite)
    .sort((left, right) => left - right)
    .map((checkpoint) => ({ checkpoint, observations: sameTask.filter((arm) => compactionCheckpoint(arm) === checkpoint) }));
  const degradationBoundary = detectDegradationBoundary(checkpoints, {
    ...bootstrap,
    expectedPairCount: expectedPairsPerCheckpoint,
  });
  const strategyComparison = [...new Set([...sameTask, ...nativeHandoff].map(compactionCheckpoint))]
    .filter(Number.isFinite)
    .sort((left, right) => left - right)
    .map((checkpoint) => {
      const baseline = sameTask.filter((arm) => compactionCheckpoint(arm) === checkpoint);
      const comparison = nativeHandoff.filter((arm) => compactionCheckpoint(arm) === checkpoint);
      const pairs = pairObservations(baseline, comparison);
      return {
        checkpoint,
        baselineStrategy: "same_task",
        comparisonStrategy: "native_handoff",
        pairedArmCount: pairs.length,
        ...comparePairedStrategies(pairs, {
          ...bootstrap,
          expectedPairCount: expectedPairsPerCheckpoint,
          seed: `same-vs-native-${model}-${checkpoint}-${bootstrap.seed ?? "v1"}`,
        }),
      };
    });
  const nextCheckpoints = baseMatrix.complete === true
    ? additionalCheckpoints(
      degradationBoundary,
      checkpoints.map((entry) => entry.checkpoint),
    )
    : [];
  let productGate = null;
  let gateCheckpoints = [];
  if (degradationBoundary.detected) {
    gateCheckpoints = [
      degradationBoundary.boundaryCheckpoint,
      degradationBoundary.confirmedAtCheckpoint,
    ];
    const availableProductCheckpoints = new Set(
      arms.filter((arm) => strategyName(arm) === "context_pack").map(compactionCheckpoint),
    );
    if (gateCheckpoints.every((checkpoint) => availableProductCheckpoints.has(checkpoint))) {
      const native = arms.filter(
        (arm) => strategyName(arm) === "native_handoff" && gateCheckpoints.includes(compactionCheckpoint(arm)),
      );
      const product = arms.filter(
        (arm) => strategyName(arm) === "context_pack" && gateCheckpoints.includes(compactionCheckpoint(arm)),
      );
      const pairs = pairObservations(native, product, (arm) => [
        arm.scenarioId ?? arm.scenario,
        compactionCheckpoint(arm),
        arm.repetition ?? arm.repeat,
        arm.fixtureSha ?? arm.fixtureSHA ?? arm.fixtureSha256,
      ].join("/"));
      const handoffBinding = validateProductHandoffPairs(pairs);
      const acceptedPairs = handoffBinding.valid ? pairs : [];
      productGate = evaluateProductGate(acceptedPairs, {
        ...bootstrap,
        expectedPairCount: expectedPairsPerCheckpoint === null ? null : expectedPairsPerCheckpoint * gateCheckpoints.length,
        baselineLatencySelector: selectedLatency,
        comparisonLatencySelector: selectedLatency,
      });
      productGate.handoffBinding = handoffBinding;
      productGate.reasons = [...handoffBinding.reasons, ...productGate.reasons];
      productGate.checkpoints = gateCheckpoints;
      productGate.pairedArmCount = acceptedPairs.length;
      productGate.candidatePairedArmCount = pairs.length;
    }
  }
  const snapshotIds = [...new Set(arms.map(snapshotId).filter(Boolean))].sort();
  const allPaidStagesIdentified = arms.length > 0 && arms.every((arm) =>
    arm.modelIdentity?.allPaidStagesIdentified === true && typeof snapshotId(arm) === "string",
  );
  const snapshotStable = allPaidStagesIdentified && snapshotIds.length === 1;
  const seriousErrorCoverageComplete = arms.every((arm) => Number.isFinite(Number(
    arm.metrics?.seriousErrorCount ?? arm.score?.seriousErrorCount ?? arm.scoring?.seriousErrorCount ?? arm.seriousErrorCount,
  )));
  const reasons = [];
  if (baseMatrix.complete === false) reasons.push("the required base measured matrix is incomplete for this model");
  if (!degradationBoundary.detected) reasons.push("no confirmed two-checkpoint degradation boundary");
  if (degradationBoundary.detected && !productGate) reasons.push("verified Context Pack product-arm results are unavailable at the boundary");
  if (productGate && !productGate.passed) reasons.push(...productGate.reasons);
  if (nextCheckpoints.length > 0) reasons.push(`additional checkpoints are required: ${nextCheckpoints.join(", ")}`);
  if (!snapshotStable) reasons.push("observed model snapshot identity is unavailable or changed during the campaign");
  if (!seriousErrorCoverageComplete) reasons.push("serious-error observability is incomplete");
  const recommend =
    baseMatrix.complete === true &&
    degradationBoundary.detected &&
    productGate?.passed === true &&
    nextCheckpoints.length === 0 &&
    snapshotStable &&
    seriousErrorCoverageComplete;
  return {
    model,
    observedModelSnapshotIds: snapshotIds,
    snapshotStable,
    allPaidStagesIdentified,
    seriousErrorCoverageComplete,
    completedArms: arms.length,
    baseMatrix,
    strategyComparison,
    degradationBoundary,
    nextCheckpoints,
    productGate,
    recommendation: {
      action: recommend ? "propose_auto_rollover" : "no_auto_rollover",
      threshold: recommend ? degradationBoundary.boundaryCheckpoint : null,
      modelSpecific: true,
      validatedSnapshotsOnly: snapshotIds,
      reasons,
    },
  };
}

function formatInterval(result) {
  if (result?.available !== true) return "unavailable";
  return `${formatNumber(result.estimate)} [${formatNumber(result.lower)}, ${formatNumber(result.upper)}]`;
}

function formatNumber(value) {
  if (!Number.isFinite(value)) return "unavailable";
  return Number(value.toFixed(2)).toString();
}

function baseMatrixCompletion(arms, manifest) {
  const matrix = manifest.matrix;
  if (
    !matrix ||
    !Number.isInteger(matrix.scenarioCount) ||
    !Number.isInteger(matrix.repetitions) ||
    !Array.isArray(matrix.initialStrategies) ||
    !Array.isArray(matrix.compactionCheckpoints)
  ) {
    return { complete: null, completedArms: null, expectedArms: null };
  }
  const strategies = new Set(matrix.initialStrategies.map((strategy) =>
    strategy.toLowerCase().replaceAll("-", "_"),
  ));
  const checkpoints = new Set(matrix.compactionCheckpoints.map(Number));
  const completedArms = arms.filter((arm) =>
    strategies.has(strategyName(arm)) && checkpoints.has(compactionCheckpoint(arm)),
  );
  const expectedArms =
    matrix.scenarioCount * matrix.repetitions * strategies.size * checkpoints.size;
  const expectedPairsPerCondition = matrix.scenarioCount * matrix.repetitions;
  const conditions = [...strategies].flatMap((strategy) => [...checkpoints].map((checkpoint) => ({
    strategy,
    checkpoint,
    observedArms: completedArms.filter((arm) =>
      strategyName(arm) === strategy && compactionCheckpoint(arm) === checkpoint,
    ).length,
  })));
  const complete = completedArms.length === expectedArms &&
    conditions.every((condition) => condition.observedArms === expectedPairsPerCondition);
  return { complete, completedArms: completedArms.length, expectedArms, expectedPairsPerCondition, conditions };
}

function normalizeArm(event) {
  let candidate;
  if (event.payload?.arm && typeof event.payload.arm === "object") {
    const { arm, ...payload } = event.payload;
    candidate = {
      ...payload,
      ...arm,
      model: arm.model ?? payload.model?.requested ?? payload.model,
      modelIdentity: payload.model,
      metrics: { ...(payload.metrics ?? {}), ...(arm.metrics ?? {}) },
      terminalRecordSha256: event.recordSha256,
    };
  } else {
    candidate = event.arm ?? event.result ?? event.payload?.result ?? event.payload ?? event;
  }
  return candidate && typeof candidate === "object" && !Array.isArray(candidate) ? candidate : null;
}

export function validateProductHandoffPairs(pairs) {
  const reasons = [];
  for (const pair of pairs) {
    const native = pair.baseline;
    const product = pair.comparison;
    const nativeKey = armKey(native);
    const nativeHandoff = native.handoff;
    const productHandoff = product.handoff;
    const sourceSnapshotSha256 = native.binding?.sourceSnapshotSha256;
    if (
      !/^[0-9a-f]{64}$/u.test(sourceSnapshotSha256 ?? '') ||
      product.binding?.sourceSnapshotSha256 !== sourceSnapshotSha256 ||
      !/^[0-9a-f]{64}$/u.test(nativeHandoff?.sha256 ?? '') ||
      nativeHandoff?.deliveredSha256 !== nativeHandoff.sha256 ||
      productHandoff?.sha256 !== nativeHandoff.sha256 ||
      productHandoff?.deliveredSha256 !== nativeHandoff.sha256 ||
      !/^[0-9a-f]{64}$/u.test(nativeHandoff?.checkpointSha256 ?? '') ||
      productHandoff?.checkpointSha256 !== nativeHandoff.checkpointSha256 ||
      productHandoff?.reusedFromNativeArmKey !== nativeKey ||
      productHandoff?.nativeResultRecordSha256 !== native.terminalRecordSha256 ||
      !Number.isInteger(nativeHandoff?.byteLength) ||
      nativeHandoff.byteLength <= 0 ||
      productHandoff?.byteLength !== nativeHandoff.byteLength
    ) {
      reasons.push(`product handoff provenance mismatch for ${pair.key}`);
    }
  }
  const uniqueReasons = [...new Set(reasons)];
  return {
    valid: pairs.length > 0 && uniqueReasons.length === 0,
    observedPairCount: pairs.length,
    reasons: pairs.length === 0 ? ["no native/product pairs were available for handoff provenance validation"] : uniqueReasons,
  };
}

function validateTerminalRecord(event, manifest) {
  if (event.schemaVersion !== 1 || typeof event.recordedAt !== "string" || !event.payload?.arm) {
    throw new Error("terminal arm record omitted its version, timestamp, or payload");
  }
  const { recordSha256, ...recordBody } = event;
  if (!/^[0-9a-f]{64}$/.test(recordSha256 ?? '') || recordSha256 !== sha256(stableStringify(recordBody))) {
    throw new Error(`terminal arm ${event.payload?.arm?.key ?? '<unknown>'} has an invalid record hash`);
  }
  const binding = event.payload.binding;
  const arm = event.payload.arm;
  for (const field of ["campaignLockSha256", "fixtureSha256", "fixtureSetSha256", "manifestSha256", "prerequisiteMergeSha"]) {
    const length = field === "prerequisiteMergeSha" ? 40 : 64;
    if (!new RegExp(`^[0-9a-f]{${length}}$`, "u").test(binding?.[field] ?? '')) {
      throw new Error(`terminal arm binding omitted or invalidated ${field}`);
    }
  }
  if (binding.fixtureSha256 !== arm.fixtureSha256) throw new Error("terminal arm fixture binding mismatch");
  if (manifest?.prerequisite?.mergeSha && binding.prerequisiteMergeSha !== manifest.prerequisite.mergeSha) {
    throw new Error("terminal arm prerequisite binding mismatch");
  }
  if (manifest?.fixtureSet?.expectedSha256 && binding.fixtureSetSha256 !== manifest.fixtureSet.expectedSha256) {
    throw new Error("terminal arm fixture-set binding mismatch");
  }
  if (manifest?.schemaVersion === 1 && manifest?.benchmarkId) {
    const expectedManifest = sha256(stableStringify(manifest));
    if (binding.manifestSha256 !== expectedManifest) throw new Error("terminal arm manifest binding mismatch");
  }
}

function armKey(arm) {
  if (typeof arm.checkpointKey === "string" && arm.checkpointKey) return arm.checkpointKey;
  const fields = [
    modelName(arm),
    arm.scenarioId ?? arm.scenario,
    strategyName(arm),
    compactionCheckpoint(arm),
    arm.repetition ?? arm.repeat,
    arm.fixtureSha ?? arm.fixtureSHA ?? arm.fixtureSha256,
  ];
  return fields.every((field) => field !== undefined && field !== null && field !== "")
    ? fields.join("/")
    : null;
}

function modelName(arm) {
  return arm.model ?? arm.identity?.model ?? arm.key?.model ?? null;
}

function snapshotId(arm) {
  const candidate = arm.modelSnapshotId ?? arm.modelSnapshot ?? arm.actualModelSnapshot ??
    arm.identity?.modelSnapshotId ?? arm.identity?.actualModelId ??
    arm.modelIdentity?.actualSnapshotId ?? arm.modelIdentity?.actualSnapshot ?? arm.modelIdentity?.snapshot ??
    arm.modelIdentity?.resolvedModel ?? arm.modelIdentity?.catalogModel ?? null;
  return typeof candidate === "string" && candidate ? candidate : null;
}

function strategyName(arm) {
  const strategy = arm.strategy ?? arm.key?.strategy;
  if (typeof strategy !== "string") return null;
  const normalized = strategy.toLowerCase().replaceAll("-", "_");
  return normalized === "verified_context_pack_contracts" ? "context_pack" : normalized;
}

function compactionCheckpoint(arm) {
  const value = arm.compaction ?? arm.compactionCheckpoint ?? arm.checkpoint ?? arm.key?.compaction;
  return value === undefined || value === null ? Number.NaN : Number(value);
}

function selectedLatency(arm) {
  const metrics = arm.metrics ?? {};
  const value = metrics.timing?.endToEndMs ?? metrics.timing?.end_to_end_ms ??
    metrics.endToEndLatencyMs ?? metrics.end_to_end_latency_ms ?? arm.endToEndLatencyMs ??
    arm.end_to_end_latency_ms ?? metrics.completionLatencyMs ?? metrics.completion_latency_ms ??
    metrics.timing?.completionMs ?? metrics.timing?.completion_ms ?? arm.completionLatencyMs ??
    arm.completion_latency_ms;
  return value === undefined || value === null || value === "unavailable" ? Number.NaN : Number(value);
}

function compareArms(left, right) {
  return String(armKey(left)).localeCompare(String(armKey(right)));
}

function writeAtomic(path, contents) {
  const directory = dirname(path);
  mkdirSync(directory, { recursive: true, mode: 0o700 });
  const temporary = join(directory, `.${basename(path)}.${process.pid}.tmp`);
  const handle = openSync(temporary, "w", 0o600);
  try {
    writeFileSync(handle, contents);
    fsyncSync(handle);
  } finally {
    closeSync(handle);
  }
  renameSync(temporary, path);
}
