# `0.1.0-alpha.3` verified source-preview checklist

This checklist accepts a reviewed source and CI/PR artifact preview. It does **not** authorize a
tag, GitHub Release, crates.io publication, or live model/compatibility run. Every completed item
must be bound to the reviewed commit. Any public-release item in the final section remains blocked.

## Source-preview gates

- [ ] Version is `0.1.0-alpha.3` in Cargo, the UI package, locked metadata, the release builder,
      issue template, and the versioned App Server probe.
- [ ] Fixture and schema validation pass without authenticated Codex or model execution.
- [ ] `cargo fmt --check`, locked all-target/all-feature Clippy with warnings denied, and all locked
      Rust tests pass.
- [ ] UI typecheck, lint, tests, production build, and `npm audit --audit-level=high` pass.
- [ ] Task edit/grouping replay, move/merge/split/undo/idempotency, rejection, mixed provenance,
      graph determinism/provenance/redaction, auth/CSRF, and accessible/mobile UI tests pass.
- [ ] Fake App Servers verify permission ready/blocked/unsupported states, exact named-permission
      request fields, no sandbox/permissions conflict, strict/malformed output, timeout/restart,
      idempotency, prompt injection/redaction, unavailable metrics, lineage pagination,
      cross-repository skips, and missing-parent degradation.
- [ ] Latest and previous stable Codex CLI probes generate official App Server schemas and confirm
      feature-level core import, continuation, experimental refresh, initialize, and read-only list
      contracts without a model call or task creation. Version strings remain provenance only.
- [ ] Secret scan and a focused security review find no unresolved critical/high issue in the
      grouping, graph, AI refresh, lineage, setup, or loopback API changes.
- [ ] `./scripts/build-release.sh` completes packaging, extracted-source offline installation,
      offline smoke, license/SBOM generation, and byte-for-byte archive reproducibility.
- [ ] `SHA256SUMS` verifies the source archive, macOS arm64 archive, binary, CycloneDX SBOM,
      NOTICE, and third-party license inventory.
- [ ] GitHub CI `quality`, `reliability-adversarial`, and `package-release` succeed for the exact PR
      head. The package job retains `previously-on-v0.1.0-alpha.3-source.tar.gz`,
      `previously-on-v0.1.0-alpha.3-macos-arm64.tar.gz`, and checksums for 14 days as PR artifacts.

## Truthful capability and documentation checks

- [ ] AI refresh is documented as beta, explicit opt-in, explicit user-triggered, input-only, and
      candidate-only. AI output is not Evidence and unavailable metrics are not invented.
- [ ] Local agent lineage is documented as same-device read-only observation, not cloud/team
      access, orchestration, or write-back.
- [ ] Consent-gated continuation opens only the documented `codex://threads/<thread-id>` deep link,
      and the review UI preserves Copy ID as a fallback when the operating-system opener fails.
- [ ] The continuation boundary is documented as a provisional policy: seven observed compactions
      OR 80% observed context usage; independently, 72 hours of inactivity plus a relevant code
      change. It is not called benchmark validated.
- [ ] The continuation campaign remains untouched at 6/864 complete, 858 remaining, and
      `no_auto_rollover`. No paid measured arm or calibration call runs.
- [ ] The pre-existing 60 authenticated compatibility workflows are not rerun. Local mapped/schema
      regressions remain explicitly distinct from live compatibility evidence.
- [ ] README, architecture, privacy, compatibility, roadmap, changelog, NOTICE/license inventory,
      and SBOM metadata describe the exact source-preview behavior and limitations.

## Explicitly blocked public-release actions

- [ ] **BLOCKED:** create or push `v0.1.0-alpha.3`.
- [ ] **BLOCKED:** create or publish a GitHub Release.
- [ ] **BLOCKED:** publish `previously-on 0.1.0-alpha.3` to crates.io.
- [ ] **BLOCKED:** claim transparent capture support from mapped regressions or schema probes.
- [ ] **BLOCKED:** run a real AI calibration/model call without a compatible App Server, verified
      permission profile, and fresh user approval at execution time.
- [ ] **BLOCKED:** treat the incomplete continuation campaign or provisional seven/80 policy as a
      model-specific measured recommendation.

Before any later public release, repeat name/trademark review, confirm protected publication
environments and branch rules, supply a SHA/version-bound eligible live compatibility artifact,
review all 60 retained workflow rows and serious-stale evaluation without rerunning validated
work, decide signing/notarization, inspect provenance/SBOM/checksums, and obtain explicit human
approval for each irreversible publication step.
