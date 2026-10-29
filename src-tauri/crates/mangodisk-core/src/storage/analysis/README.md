# Analysis scan metrics

`AnalysisScanMode::Standard` is the default for Core and adapters. It reports
native allocated bytes using the existing platform stream or traversal fallback.
On Windows, `Fast` fully traverses the same accessible scope and exclusions, but uses logical
file length from metadata without querying native allocation. Compressed and
sparse file lengths can differ substantially from their disk usage. Both modes
preserve no-follow traversal, hard-link charging, cancellation and delete preflight.

Results and in-memory directory aggregates carry the mode. Navigation, remainder
lists and delete reconciliation use that metric. Reusing an index in another mode
is forbidden; overlapping indexes in different metrics are invalidated rather than
patched. Deletions from another metric invalidate the incompatible index.

macOS and Linux expose one analysis action and always use `Standard`. Core normalizes
legacy fast requests to standard on these platforms and records the applied mode.
Windows defaults to standard on first use, restores the last selected mode on
startup, and discards navigation results when switching modes. Invalid saved
modes fall back to standard. Its labels describe standard bytes as disk usage and fast bytes
as file size. The standard CLI/Core convenience APIs retain their default behavior;
call `AnalysisService::analyze_with_mode_progress` to select a metric explicitly.

Analysis retains Windows-managed volume files in its space accounting. Entry
transport adds an optional `deletionProtection` hint (`systemManagedFile`,
`protectedPath`, or `null`); adapters disable selection and ordinary deletion for
protected entries. Analysis deletion applies the platform's deletion protection
policy to system and application-managed scopes while retaining their space
accounting in read-only analysis. macOS also protects ancestors of reserved
system scopes and volume metadata directories, including external volumes;
reserved names are checked before mutation and during final tree traversal, so
deleting an enclosing directory cannot bypass protection. This guarded traversal
also applies when no user name exclusions are configured. Read-only navigation
into `/usr` can still reach user-managed `/usr/local`. Older cached
results may omit the hint, so Core always checks the platform classification
before opening any deletion target. This covers hibernation, paging, swap, and
DumpStack files at local volume roots, including verbatim and case variants,
local drive device paths, Win32 trailing-dot/space aliases, and file streams.
Verbatim trailing dots and spaces remain literal names rather than aliases.
Use the corresponding system setting instead of deleting these files directly.

## Scan coverage and privilege

Analysis includes readable Windows system directories such as WinSxS, System32 and
Installer. Read access never authorizes deletion: protected-path classification,
link handling and deletion preflight remain independent of analysis inclusion.
Scan policy version 2 is part of native cache fingerprints and frontend cache
acceptance. Native cache identity also includes the process elevation state, so
an unelevated snapshot cannot conceal newly readable directories after restart.
These indexes are session-scoped and incompatible snapshots are rebuilt.

Coverage schema version 1 reports omitted entries by permission denial, read error,
configured or platform exclusion, link/mount boundary, and unclassified reason.
Counts describe encountered omissions, not all descendants of an unreadable
folder. Native readers that cannot identify a reason report it as unclassified.
Directories with no measurable data and read failures expose an unavailable size;
known empty directories retain a complete zero-byte measurement.

On Windows, a permission-denied result from a confirmed non-elevated process can
request an administrator restart. The adapter resolves the scan ID through Core,
then uses the authenticated parent/child restart handshake. The child consumes one
validated directory argument and forces a new scan. Failed or canceled elevation
leaves the existing result available; ordinary restarts do not replay the request.

## Current-directory batch deletion

`AnalysisService::delete_entries_permanently` accepts at most 500 unique paths
from one authoritative scan's direct-child entries. It validates the entire
selection's membership before mutation, holds one filesystem mutation claim,
and runs the existing single-item safety, staging, verification and cache
reconciliation for each item. Protected paths, links/reparse points and scan
exclusions remain enforced in Core regardless of cached UI hints.

The versioned response separates removed paths, failed items with typed reasons,
and paths not attempted. A failure before mutation allows the next item to run.
Possible partial mutation, a recreated original path or expired scan authority
stops the batch and requires fresh analysis; completed deletions cannot be rolled
back. Only final reconciled snapshots are returned when authority remains valid.
`releasedBytes` sums successful selections' original scan-time usage so hard-link
allocation transfers are not counted twice. It is not a live measurement of
reclaimed disk space.

## Interrupted scan diagnostics

Analysis logs identify stage transitions, rate-limited path/count progress, and Windows
worker activity (at most three active task samples per five seconds). Finalization and
result publication have separate events so a missing result can be localized beyond
directory enumeration. Cancellation is an informational outcome. Application startup
records include the process, executable and build profile; normal exit and Rust panic
events distinguish observable shutdown paths. Forced termination and native aborts may
leave no final event and must not be diagnosed as a specific crash from that absence alone.

On a disposable Windows machine, run native validation from a normal desktop token:

```powershell
$env:MANGODISK_ANALYSIS_VALIDATION_ROOT = 'C:\'
$env:MANGODISK_ANALYSIS_VALIDATION_DIR = Join-Path $PWD 'validation/analysis'
cargo test --manifest-path src-tauri/Cargo.toml -p mangodisk-core --test windows_analysis_validation -- --ignored --nocapture
```

The selected root is read-only. Unique fixtures and application storage stay under the
evidence directory. The test covers completion, cancellation, retry, ACL denial and an
invalid root, and verifies logging boundaries. Preserve local evidence outside commits.
