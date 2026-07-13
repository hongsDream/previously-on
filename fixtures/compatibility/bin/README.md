# Compatibility driver test doubles

These executable fixtures implement only `codex --version` and the documented App Server `initialize`, `initialized`, and `thread/list` exchange. Their intentionally impossible `99.x` versions make it clear that they verify the offline driver path only. They are never evidence of real Codex compatibility and are not included in release artifacts.
