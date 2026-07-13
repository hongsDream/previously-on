# `v0.1.0-alpha.1` release checklist

The release owner records links or hashes for every item before creating the tag. A failed item
stops publication; the crate name must not be published speculatively because crates.io versions
cannot be overwritten.

## Local and compatibility gates

- [ ] All known audit findings are closed or explicitly release-blocking.
- [ ] The mapped-regression result is labelled ineligible and is not used as live evidence.
- [ ] A separately produced live artifact passes all 30 authenticated reconstruction workflows on
      each npm-selected CLI version and satisfies `scripts/validate-live-compatibility.mjs`; the
      separate mapped artifact passes all 30 category-specific fault assertions for both slots.
- [ ] The live producer consumed the separate mapped artifact from the same clean commit and CLI
      versions; every scenario entry matches its exact Rust target, filter, and expectation.
- [ ] The mapped driver recorded one actual row outcome per scenario and version; every filter
      resolved to exactly one Rust test and no zero-test or misspelled-filter pass was accepted.
- [ ] The live artifact was produced by `scripts/compatibility/live-harness.mjs` from the reviewed
      clean commit: 30 `exec` + `exec resume` workflows per CLI version, 60 passes total.
- [ ] If `--resume` was used, every skipped checkpoint was revalidated against the same commit,
      binaries, fixtures, mapped artifact, App evidence, retained file, and evidence digest; no
      checkpoint crossed an evidence-binding change.
- [ ] Every live workflow records ground-truth filesystem/Git/JSONL hashes and reconstructs prompt,
      assistant final, paired file-change tool, test command, and stable linked source IDs.
- [ ] The retained evidence bundle contains structured verdicts, stable identifiers, hashes, and
      counts only; raw Codex JSONL, prompts, tool output, source code, credentials, and absolute
      repository paths were not persisted.
- [ ] Every scenario `evidencePath` and the retained mapped artifact are present in the tarball,
      independently hashed, and accepted by the validator after safe extraction.
- [ ] The approved model and cost window are recorded; `CODEX_HOME` and auth are used only in the
      manual producer environment and never added to normal CI or release environment variables.
- [ ] Current Codex App stable and the previous obtainable build are recorded accurately in
      retained sanitized evidence files. A degraded signature result remains release-blocking.
- [ ] Serious stale applications are measured by a retained evaluator artifact bound to this
      commit and version; an unmeasured or hardcoded zero is not accepted.
- [ ] If stale evaluation was attached after the live run, `--finalize-stale-evaluation`
      revalidated all 60 retained scenario digests and unchanged commit, binary, mapped, runner,
      and App bindings without rerunning a Codex workflow.
- [ ] The outbound-denied smoke exercised setup, daemon/hook persistence, MCP initialize and
      tools/list, a loopback UI request, export, purge, repeated uninstall, and cleanup.
- [ ] `docs/compatibility.md` contains run dates, artifact hashes, and supported/degraded status.
- [ ] `quality`, `reliability-adversarial`, and `package-release` pass on `main`.
- [ ] The release archive was reproduced and `SHA256SUMS` verified after extraction.

## Name and public repository gates

- [ ] Repeat GitHub and crates.io searches for `PreviouslyOn` and `previously-on` immediately
      before publication; attach result URLs and timestamps.
- [ ] Repeat the relevant trademark search and review for likelihood of confusion. Do not choose
      an automatic replacement name if a same-field conflict is found.
- [ ] GitHub private vulnerability reporting is enabled and its form opens successfully.
- [ ] A `main` branch ruleset requires the three CI jobs and prevents tag deletion.
- [ ] Actions permissions are restricted to read by default; release workflow exceptions are
      reviewed.
- [ ] The protected `crates-io` environment requires a human reviewer and contains only
      `CARGO_REGISTRY_TOKEN`.
- [ ] The protected `release-compatibility` environment requires a human reviewer and defines the
      reviewed `LIVE_COMPATIBILITY_BUNDLE_URL` and `LIVE_COMPATIBILITY_BUNDLE_SHA256` values.
- [ ] GitHub's hosted-runner contract still lists standard public `macos-14` as arm64; the build
      script's runtime `uname -m` check also passes.

## Immutable publication sequence

- [ ] Create and push `v0.1.0-alpha.1` from the reviewed clean `main` commit.
- [ ] Approve the `release-compatibility` gate and confirm it accepts the pinned live artifact.
- [ ] Confirm `transparentCaptureReleaseGate.eligible` is true only after all 60 scenario entries
      and their retained evidence hashes have been reviewed.
- [ ] Confirm the tag workflow creates a draft release and provenance attestations.
- [ ] Inspect archive names, SBOM, NOTICE, third-party inventory, and checksums.
- [ ] Explicitly approve the protected crates.io job.
- [ ] Confirm `cargo install previously-on --version 0.1.0-alpha.1 --locked` succeeds.
- [ ] Confirm the workflow publishes the GitHub release only after exact-version install passes.
