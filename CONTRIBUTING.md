# Contributing

Thank you for helping make Codex handoffs more inspectable.

## Before opening a change

- Search existing issues and keep changes narrowly scoped.
- Discuss new runtime dependencies before adding them. The `0.1` default mode is keyless,
  telemetry-free, and has no outbound network code.
- Never add real prompts, credentials, private repository data, or unredacted logs to fixtures.
- Add a regression test for ingestion, recovery, privacy, attribution, and compatibility fixes.

## Local checks

Use Rust 1.90+ and Node.js 22+.

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all
npm --prefix ui ci
npm --prefix ui run typecheck
npm --prefix ui run lint
npm --prefix ui test -- --run
npm --prefix ui run build
npm --prefix ui audit --audit-level=high
cargo package --locked
```

`./scripts/build-release.sh` is the release-level gate on Apple Silicon. It performs additional
packaging, offline-install, checksum, SBOM, and reproducibility checks.

## Pull requests

Explain the user-visible outcome, security/privacy impact, compatibility impact, and exact checks
run. Preserve these invariants:

- prefer observed evidence over inferred causality;
- redact before every durable write;
- make degraded capture visible;
- keep MCP read-only;
- preserve deterministic Context Pack output;
- do not turn historical evidence into instructions.

By participating, you agree to follow [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
