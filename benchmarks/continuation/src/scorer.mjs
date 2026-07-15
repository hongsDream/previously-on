const STATE_RECALL_DIMENSIONS = ["goal", "changedFiles", "testStatus", "nextStep"];

export function parseFinalJson(value) {
  if (value && typeof value === "object" && !Array.isArray(value)) {
    return structuredClone(value);
  }
  if (typeof value !== "string") {
    throw new TypeError("final response must be a JSON object or an exact JSON object string");
  }
  const source = value.replace(/^\uFEFF/, "").trim();
  let parsed;
  try {
    parsed = JSON.parse(source);
  } catch (error) {
    throw new Error(`final response is not exact JSON: ${error.message}`);
  }
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("final response JSON must be an object");
  }
  return parsed;
}

export function scoreFinalResponse(fixture, finalResponse, observations = {}) {
  if (!fixture || typeof fixture !== "object" || Array.isArray(fixture)) {
    throw new TypeError("fixture must be an object");
  }
  if (!observations || typeof observations !== "object" || Array.isArray(observations)) {
    throw new TypeError("scoring observations must be an object");
  }
  const responseDocument = parseFinalJson(finalResponse);
  const document = { ...responseDocument, ...structuredClone(observations) };
  const rubric = fixture.rubric ?? fixture.scoring ?? {};
  const criteria = normalizeCriteria(rubric.criteria ?? rubric.fields ?? rubric.correctness ?? []);
  if (criteria.length === 0) {
    throw new Error("fixture rubric must declare at least one correctness criterion");
  }

  const benchmarkRubric = criteria.every((criterion) => typeof criterion.kind === "string");
  if (benchmarkRubric) validateBenchmarkResponse(responseDocument, fixture);
  const criterionResults = criteria.map((criterion, index) =>
    benchmarkRubric
      ? scoreBenchmarkCriterion(criterion, fixture, document, index)
      : scoreCriterion(criterion, document, index),
  );
  const totalWeight = criterionResults.reduce((sum, result) => sum + result.weight, 0);
  const earnedWeight = criterionResults.reduce(
    (sum, result) => sum + (result.passed ? result.weight : 0),
    0,
  );
  const correctnessRatio = totalWeight === 0 ? 0 : earnedWeight / totalWeight;

  const invariantViolations = dedupe([
    ...(benchmarkRubric ? observedInvariantViolations(document) : explicitIssueIds(document, "invariantViolations")),
    ...normalizeRules(benchmarkRubric ? [] : (rubric.invariants ?? fixture.invariants)).flatMap((rule, index) => {
      const result = evaluateRule(rule, document);
      return result.passed ? [] : [rule.id ?? `invariant-${index + 1}`];
    }),
  ]);
  const staleClaims = dedupe([
    ...explicitIssueIds(document, "staleClaims"),
    ...(benchmarkRubric ? derivedStaleClaimIds(fixture, responseDocument) : []),
    ...normalizeRules(benchmarkRubric ? [] : (rubric.staleClaims ?? fixture.staleClaims)).flatMap((rule, index) =>
      evaluateTrigger(rule, document) ? [rule.id ?? `stale-claim-${index + 1}`] : [],
    ),
  ]);
  const seriousErrors = dedupe([
    ...explicitIssueIds(document, "seriousErrors"),
    ...explicitIssueIds(document, "authoritativeSeriousErrors"),
    ...criterionResults.filter((result) => result.seriousOnFailure && !result.passed).map((result) => result.id),
    ...normalizeRules(benchmarkRubric ? [] : (rubric.seriousErrors ?? fixture.seriousErrors)).flatMap((rule, index) =>
      evaluateTrigger(rule, document) ? [rule.id ?? `serious-error-${index + 1}`] : [],
    ),
  ]);
  const stateRecall = scoreStateRecall(
    rubric.stateRecall ?? fixture.stateRecall ?? fixture.expectedState,
    document,
  );
  const successThreshold = benchmarkRubric ? rubric.successThreshold : totalWeight;
  const thresholdPassed = earnedWeight >= successThreshold;
  const success =
    thresholdPassed &&
    invariantViolations.length === 0 &&
    staleClaims.length === 0 &&
    seriousErrors.length === 0;

  return {
    schemaVersion: 1,
    success,
    rubricPassed: thresholdPassed,
    correctness: {
      earnedWeight,
      totalWeight,
      successThreshold,
      ratio: correctnessRatio,
      percentage: correctnessRatio * 100,
      criteria: criterionResults,
    },
    invariantViolationCount: invariantViolations.length,
    invariantViolations,
    staleClaimCount: staleClaims.length,
    staleClaims,
    seriousErrorCount: seriousErrors.length,
    seriousErrors,
    stateRecall,
  };
}

export const scoreContinuationResult = scoreFinalResponse;
export const scoreResponse = scoreFinalResponse;

function derivedStaleClaimIds(fixture, document) {
  const stateText = [document.goal, document.testStatus, document.nextStep]
    .filter((value) => typeof value === "string")
    .join("\n")
    .toLowerCase();
  return (fixture.staleFacts ?? []).flatMap((fact) => {
    if (typeof fact?.id !== "string" || typeof fact?.claim !== "string") return [];
    return stateText.includes(fact.claim.toLowerCase()) ? [fact.id] : [];
  });
}

function validateBenchmarkResponse(document, fixture) {
  const required = [
    "schemaVersion",
    "scenarioId",
    "goal",
    "changedFiles",
    "testStatus",
    "nextStep",
    "invariantViolations",
    "staleClaims",
    "seriousErrors",
    "completionMarker",
  ];
  const unknown = Object.keys(document).filter((key) => !required.includes(key));
  if (unknown.length > 0) {
    throw new Error(`benchmark final JSON contains unknown fields: ${unknown.sort().join(", ")}`);
  }
  for (const key of required) {
    if (!Object.hasOwn(document, key)) throw new Error(`benchmark final JSON omitted ${key}`);
  }
  for (const key of ["goal", "testStatus", "nextStep", "completionMarker"]) {
    if (typeof document[key] !== "string") throw new Error(`benchmark final JSON ${key} must be a string`);
  }
  if (document.schemaVersion !== 1) throw new Error("benchmark final JSON schemaVersion must be 1");
  if (document.scenarioId !== fixture.id) throw new Error("benchmark final JSON scenarioId does not match the fixture");
  if (document.completionMarker !== fixture.finalChallenge?.completionMarker) {
    throw new Error("benchmark final JSON completionMarker does not match the fixture");
  }
  if (!["passed", "failed", "not_run", "unavailable"].includes(document.testStatus)) {
    throw new Error("benchmark final JSON testStatus is invalid");
  }
  for (const key of ["changedFiles", "invariantViolations", "staleClaims", "seriousErrors"]) {
    if (!Array.isArray(document[key]) || !document[key].every((item) => typeof item === "string")) {
      throw new Error(`benchmark final JSON ${key} must be an array of strings`);
    }
  }
}

function scoreBenchmarkCriterion(criterion, fixture, document, index) {
  const values = criterion.values;
  if (!Array.isArray(values) || values.length === 0) {
    throw new Error(`benchmark rubric criterion ${criterion.id ?? index + 1} omitted values`);
  }
  let passed = false;
  let actual = null;
  switch (criterion.kind) {
    case "exact_changed_files": {
      actual = observedChangedFiles(document);
      passed = deepEqual([...actual].sort(), [...values].sort());
      break;
    }
    case "contains_all": {
      actual = targetText(document, criterion.target);
      passed = values.every((value) => actual.includes(value));
      break;
    }
    case "excludes_all": {
      actual = targetText(document, criterion.target);
      passed = values.every((value) => !actual.includes(value));
      break;
    }
    case "command_passed": {
      actual = observedCommands(document);
      passed = actual.some((command) => command.passed && deepEqual(command.argv, values));
      break;
    }
    case "no_invariant_violation": {
      actual = observedInvariantViolations(document);
      passed = values.every((id) => !actual.includes(id));
      break;
    }
    case "state_recall_equals": {
      actual = document.stateRecall ?? document.state_recall ?? {
        goal: document.goal,
        changedFiles: document.changedFiles,
        testStatus: document.testStatus,
        nextStep: document.nextStep,
      };
      passed = values.every((dimension) =>
        Object.hasOwn(fixture.expectedState ?? {}, dimension) &&
        equalStateDimension(dimension, actual[dimension], fixture.expectedState[dimension]),
      );
      break;
    }
    default:
      throw new Error(`unsupported benchmark rubric kind ${criterion.kind}`);
  }
  const weight = criterion.points;
  if (!Number.isFinite(weight) || weight <= 0) {
    throw new Error(`benchmark rubric criterion ${criterion.id ?? index + 1} has invalid points`);
  }
  return {
    id: criterion.id ?? `criterion-${index + 1}`,
    required: false,
    seriousOnFailure: criterion.seriousOnFailure === true,
    kind: criterion.kind,
    target: criterion.target,
    weight,
    passed,
    actual,
    expectation: values,
  };
}

function equalStateDimension(dimension, actual, expected) {
  if (dimension === "changedFiles" && Array.isArray(actual) && Array.isArray(expected)) {
    return deepEqual([...actual].sort(), [...expected].sort());
  }
  return deepEqual(actual, expected);
}

export function evaluateRule(rule, document) {
  if (!rule || typeof rule !== "object" || Array.isArray(rule)) {
    throw new TypeError("rubric rule must be an object");
  }
  const path = rule.path ?? rule.jsonPath;
  if (typeof path !== "string" || path.length === 0) {
    throw new Error(`rubric rule ${rule.id ?? "<unnamed>"} omitted path`);
  }
  const actual = valueAtPath(document, path);
  const found = actual.found;
  let passed;
  let expectation;

  if (Object.hasOwn(rule, "present")) {
    passed = rule.present ? found : !found;
    expectation = rule.present ? "present" : "absent";
  } else if (Object.hasOwn(rule, "absent")) {
    passed = rule.absent ? !found : found;
    expectation = rule.absent ? "absent" : "present";
  } else if (Object.hasOwn(rule, "equals") || Object.hasOwn(rule, "expected")) {
    const expected = Object.hasOwn(rule, "equals") ? rule.equals : rule.expected;
    passed = found && deepEqual(actual.value, expected);
    expectation = { equals: expected };
  } else if (Object.hasOwn(rule, "notEquals")) {
    passed = !found || !deepEqual(actual.value, rule.notEquals);
    expectation = { notEquals: rule.notEquals };
  } else if (Object.hasOwn(rule, "includes")) {
    passed = found && includes(actual.value, rule.includes);
    expectation = { includes: rule.includes };
  } else if (Object.hasOwn(rule, "containsAll")) {
    const expected = Array.isArray(rule.containsAll) ? rule.containsAll : [rule.containsAll];
    passed = found && expected.every((item) => includes(actual.value, item));
    expectation = { containsAll: expected };
  } else if (Object.hasOwn(rule, "oneOf")) {
    if (!Array.isArray(rule.oneOf) || rule.oneOf.length === 0) {
      throw new Error(`rubric rule ${rule.id ?? path} oneOf must be a non-empty array`);
    }
    passed = found && rule.oneOf.some((item) => deepEqual(actual.value, item));
    expectation = { oneOf: rule.oneOf };
  } else if (Object.hasOwn(rule, "matches")) {
    if (typeof actual.value !== "string") {
      passed = false;
    } else {
      const expression = new RegExp(rule.matches, rule.flags ?? "u");
      passed = expression.test(actual.value);
    }
    expectation = { matches: rule.matches, flags: rule.flags ?? "u" };
  } else {
    throw new Error(`rubric rule ${rule.id ?? path} omitted a supported matcher`);
  }

  return {
    passed,
    path,
    found,
    actual: found ? actual.value : null,
    expectation,
  };
}

function scoreCriterion(criterion, document, index) {
  const result = evaluateRule(criterion, document);
  const weight = criterion.weight ?? 1;
  if (!Number.isFinite(weight) || weight <= 0) {
    throw new Error(`rubric criterion ${criterion.id ?? index + 1} has invalid weight`);
  }
  return {
    id: criterion.id ?? `criterion-${index + 1}`,
    required: criterion.required !== false,
    seriousOnFailure: criterion.seriousOnFailure === true,
    weight,
    ...result,
  };
}

function scoreStateRecall(specification, document) {
  const spec = specification && typeof specification === "object" ? specification : {};
  const stateDocument = document.stateRecall ?? document.state_recall ?? {
    goal: document.goal,
    changedFiles: document.changedFiles,
    testStatus: document.testStatus,
    nextStep: document.nextStep,
  };
  const dimensions = {};
  for (const dimension of STATE_RECALL_DIMENSIONS) {
    const configured = spec[dimension];
    if (configured === undefined) {
      dimensions[dimension] = { available: false, recalled: null };
      continue;
    }
    let rule =
      configured && typeof configured === "object" && !Array.isArray(configured)
        ? { path: `stateRecall.${dimension}`, ...configured }
        : { path: `stateRecall.${dimension}`, equals: configured };
    const normalizedState = { ...stateDocument };
    if (dimension === "changedFiles" && Array.isArray(configured) && Array.isArray(stateDocument[dimension])) {
      normalizedState[dimension] = [...stateDocument[dimension]].sort();
      rule = { path: `stateRecall.${dimension}`, equals: [...configured].sort() };
    }
    const result = evaluateRule(rule, { ...document, stateRecall: normalizedState });
    dimensions[dimension] = { available: true, recalled: result.passed, ...result };
  }
  const available = Object.values(dimensions).filter((dimension) => dimension.available);
  const recalled = available.filter((dimension) => dimension.recalled).length;
  return {
    dimensions,
    recalled,
    total: available.length,
    ratio: available.length === 0 ? null : recalled / available.length,
    allRecalled: available.length === 0 ? null : recalled === available.length,
  };
}

function observedChangedFiles(document) {
  const value = document.repository?.changedFiles ?? document.repository?.changed_files ??
    document.changedFiles ?? document.changed_files ?? [];
  if (!Array.isArray(value)) throw new Error("scoring input changedFiles must be an array");
  return value.map((item, index) => {
    if (typeof item === "string") return item;
    if (item && typeof item.path === "string") return item.path;
    throw new Error(`scoring input changedFiles[${index}] must be a path string or object`);
  });
}

function observedInvariantViolations(document) {
  const explicit = dedupe([
    ...explicitIssueIds(document, "invariantViolations"),
    ...explicitIssueIds(document.repository ?? {}, "invariantViolations"),
  ]);
  const states = document.repository?.invariants ?? document.invariants;
  if (!states || Array.isArray(states) || typeof states !== "object") return explicit;
  return dedupe([
    ...explicit,
    ...Object.entries(states).filter(([, value]) => value === false || value === "violated").map(([id]) => id),
  ]);
}

function targetText(document, target) {
  if (target === "assistant_final") {
    const value = document.assistantFinal ?? document.assistant_final ?? document.final;
    if (value === undefined) return JSON.stringify(document);
    if (typeof value !== "string") throw new Error("scoring input assistantFinal must be a string");
    return value;
  }
  const value = document[target] ?? "";
  return typeof value === "string" ? value : JSON.stringify(value);
}

function observedCommands(document) {
  const value = document.toolTrace?.commands ?? document.tool_trace?.commands ??
    document.testCommands ?? document.test_commands ?? document.commands ?? [];
  if (!Array.isArray(value)) throw new Error("scoring input commands must be an array");
  return value.map((command, index) => {
    if (Array.isArray(command)) return { argv: command, passed: true };
    if (!command || typeof command !== "object") {
      throw new Error(`scoring input command ${index} must be argv or an object`);
    }
    const argv = command.argv ?? command.command;
    if (!Array.isArray(argv) || !argv.every((item) => typeof item === "string")) {
      throw new Error(`scoring input command ${index} omitted string argv`);
    }
    const passed = command.passed === true || command.status === "passed" || command.exitCode === 0 || command.exit_code === 0;
    return { argv, passed };
  });
}

function explicitIssueIds(document, key) {
  const value = document[key] ?? document.errors?.[key];
  if (value === undefined) return [];
  if (!Array.isArray(value)) throw new Error(`final response ${key} must be an array when present`);
  return value.map((item, index) => {
    if (typeof item === "string" && item.trim()) return item.trim();
    if (item && typeof item === "object" && typeof item.id === "string" && item.id.trim()) {
      return item.id.trim();
    }
    throw new Error(`final response ${key}[${index}] must be a string or an object with id`);
  });
}

function evaluateTrigger(rule, document) {
  const condition = rule.when && typeof rule.when === "object"
    ? { path: rule.path ?? rule.when.path, ...rule.when }
    : rule;
  if (Object.hasOwn(condition, "forbidden")) {
    return evaluateRule({ path: condition.path, includes: condition.forbidden }, document).passed;
  }
  return evaluateRule(condition, document).passed;
}

function normalizeCriteria(value) {
  if (Array.isArray(value)) return value;
  if (!value || typeof value !== "object") return [];
  return Object.entries(value).map(([path, expected]) => ({ id: path, path, expected }));
}

function normalizeRules(value) {
  if (value === undefined || value === null) return [];
  if (!Array.isArray(value)) throw new Error("rubric issue rules must be arrays");
  return value;
}

function valueAtPath(document, path) {
  const parts = path.startsWith("/")
    ? path
        .slice(1)
        .split("/")
        .map((part) => part.replaceAll("~1", "/").replaceAll("~0", "~"))
    : path.replaceAll(/\[(\d+)\]/g, ".$1").split(".").filter(Boolean);
  let current = document;
  for (const part of parts) {
    if (!current || typeof current !== "object" || !Object.hasOwn(current, part)) {
      return { found: false, value: undefined };
    }
    current = current[part];
  }
  return { found: true, value: current };
}

function includes(actual, expected) {
  if (typeof actual === "string") return actual.includes(String(expected));
  if (Array.isArray(actual)) return actual.some((item) => deepEqual(item, expected));
  return false;
}

function deepEqual(left, right) {
  if (Object.is(left, right)) return true;
  if (Array.isArray(left) && Array.isArray(right)) {
    return left.length === right.length && left.every((item, index) => deepEqual(item, right[index]));
  }
  if (left && right && typeof left === "object" && typeof right === "object") {
    const leftKeys = Object.keys(left).sort();
    const rightKeys = Object.keys(right).sort();
    return deepEqual(leftKeys, rightKeys) && leftKeys.every((key) => deepEqual(left[key], right[key]));
  }
  return false;
}

function dedupe(values) {
  return [...new Set(values)].sort();
}
