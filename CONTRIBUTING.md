# Contributing to MangoDisk

Thank you for contributing to MangoDisk. This guide defines the contribution
workflow; repository instructions remain authoritative for implementation
details.

## Before making a change

1. Read [`AGENTS.md`](AGENTS.md) and the nearest subtree instructions.
2. Keep Core, Platform, Tauri, CLI, and Vue responsibilities separate.
3. Prefer validated TOML when extending ordinary cleanup coverage.
4. Explain ownership, failure behavior, and tests before introducing a
   capability that crosses three or more domains.

## Development requirements

- Use Node.js 24 and the pnpm version declared in `package.json`.
- Use the Rust toolchain declared by the repository.
- Windows Explorer packaging honors `WindowsSdkDir` from the developer shell.
  Set it to the installed SDK root when the SDK uses a custom location; otherwise
  the build uses the standard Windows Kits directory.
- Keep user-facing text synchronized across every supported locale resource.
- Use generic deterministic paths in tests.
- Never commit credentials, personal paths, private file names, raw scan
  results, build outputs, or local dependency directories.
- Preserve dry-run, protected-path, preflight, confirmation, and verification
  boundaries for every destructive flow.

## Validation

Run the full local validation before submitting a change:

```sh
pnpm check
cargo test --manifest-path src-tauri/Cargo.toml -p mangodisk-core
```

Cross-platform changes require the applicable checks on macOS and Windows. If a
platform is unavailable, identify that unvalidated scope in the pull request.
Performance changes should describe a reproducible workload and before-and-after
result without committing raw machine reports or private datasets.

Run filesystem permission tests from a normal Windows desktop terminal. SSH sessions
with enabled backup/restore privileges can bypass fixture ACLs and invalidate their
permission-denial assertions; do not suppress those failures.

For native privacy execution validation on a disposable Windows machine, run:

```powershell
$env:MANGODISK_PRIVACY_VALIDATION_DIR = Join-Path $PWD 'validation/privacy'
cargo test --manifest-path src-tauri/Cargo.toml -p mangodisk-core --test windows_privacy_execution_validation native_privacy_execution_matrix -- --ignored --nocapture
```

This opt-in test creates unique browser profiles under the current account's native
profile directories and selects only its generated sources. It covers exclusions,
process state, locks, junctions, drift, cancellation, history failures, platform
isolation, and diagnostic content boundaries. Generated profiles are removed afterward; evidence stays in the
chosen directory and must not be committed.

Windows service switches change automatic startup, not the current running state.
Only eligible third-party services are writable; protected or uncertain entries
remain read-only. Disabling preserves the delayed-start setting in the
administrator-owned `MangoDiskStartupRestoreV1` service registry value. Unknown
or malformed backup versions fail before mutation; successful re-enabling removes
the backup. Startup helper protocol v4 rejects older requests before execution.
For service-control changes, run the ignored `windows::startup::service_control`
tests in a disposable elevated Windows VM. The existing-service test requires an
explicit `MANGODISK_TEST_SERVICE_NAMES` allowlist (semicolon-separated); choose
noncritical third-party services and verify that configuration and runtime state
are restored afterward.

On macOS, validate that closing hides the window, Dock reopening restores it,
and Quit exits immediately, including during a scan, using the bundled app.

## Pull requests

- Keep each pull request focused on one coherent purpose.
- Explain observable behavior, safety impact, and validation evidence.
- Add tests for safety boundaries, protocol changes, persistence, and
  regressions.
- Update contributor-facing README guidance in the same change as its behavior.
- Do not hide failures with allow attributes, global machine settings, or
  undocumented fallbacks.

Security vulnerabilities must follow [`SECURITY.md`](SECURITY.md) rather than a
public issue.
