# Analysis snapshots and scan metrics

## Query and loading boundaries

`query.rs` borrows retained directory/file measurements and the existing navigation
index. It owns child lookup, common entry conversion, top-list eligibility and
snapshot identity checks. It performs no filesystem IO and keeps no additional
long-lived cache. Incomplete retained prefixes explicitly request directory loading;
a scan-time per-directory limit alone does not prove that global eviction kept
all rows needed by the list.

`directory_loader.rs` owns optional current-directory reads, initial metadata
assembly and explicit remainder details. Navigation retains its existing loading
fallback; export never invokes it. Read failures are sampled with native error
codes and summarized separately from scan omissions. Directory measurements remain
scan facts even when a read-only file listing observes later filesystem changes.

`index_projection.rs` assembles private scan results and cached navigation through
one result projection. `hierarchy.rs` uses one bounded tree algorithm for both
sources, without copying selected rows into temporary measurement maps and then
building a second tree. Private scans group eligible candidates once; cached
queries use the existing parent/name index rather than scanning every stored row
for each child. Native keys remain authoritative through counting and selection.

List, chart and export consume the same retained measurements, with distinct
presentation budgets. The 500-row list limit, chart budgets, scan-wide candidate
budget, hard-link metadata and root/session lifetimes retain their separate roles.
No per-directory full file cache or extra global navigation index is introduced.

## JSON export of existing analysis

The desktop result page exports read-only directory statistics from the retained
Core index. It never enumerates the source again. The request is bound to a native
scan ID, index publication generation, mutation revision and byte metric; replaced
or evicted data produces `analysisExportUnavailable` and requires an explicit new
analysis. The export can cover the original retained scan scope or the currently
browsed subtree. Preparing a report freezes its data before the native save dialog
opens. Later scans do not change that prepared report.

Reports use `schemaVersion: 4`. Readers must reject unsupported schema versions;
versions 1–3 were earlier export formats. The JSON contains `root`, `scannedAtMs`,
`byteMetric` and a flat `directories` array. Each ordinary directory contains only
`path`, recursive `bytes` and `fileCount`. The first row is the root and contains the export totals;
ancestor and descendant totals overlap and must not be added together.

Relative paths are the default, with `/` separators and `.` for the directory root.
The report's `root` is the folder name in relative mode or the native absolute path
when explicitly requested. No absolute scan or exclusion prefix is included in
relative mode. Directory names and name exclusions can still reveal personal data.
Lossy native names are marked `pathIsLossy`; rendered paths are not deletion authority.

Exceptional fields are emitted only when needed: per-directory `scannedAtMs` when
a reused measurement differs from the report time, `sizeStatus` for partial or
unavailable data, and nonzero `omissions`. Missing status means complete, missing
omissions means zero. Omission counts describe encountered entries, not unknown
descendants. Nonempty `exclusions` identify skipped paths and configured name rules.
`subtreeOfOriginalScan: true` indicates that hard-link allocation was accounted in
an enclosing scan; bytes charged outside the exported subtree are not included.

Key directory summary is the default. It retains at most 500 directory rows, including
all ancestors of each selected row. Expansion prioritizes the largest share of
bytes, file count or scan omissions, with deterministic shallow/path tie-breaking.
Both modes accept a minimum directory size in the scan's byte metric, defaulting
to zero (no filter). The GUI resolves 10 MB, 100 MB and 1 GB presets using the
platform's existing unit convention. Full export retains every matching directory
without a count limit. The root, incomplete measurements, and ancestors needed
to connect matching or incomplete directories remain eligible regardless of size.
Summary applies its 500-row budget after size filtering; it may still collapse
eligible or incomplete subtrees when that budget is exhausted.

Rows may contain `filtered: { directoryCount, bytes, fileCount }` for subtrees
without size matches or incomplete measurements. Summary may also contain
`collapsed` with the same fields for eligible subtrees hidden by its row budget.
A bucket's reason describes why that entire child subtree is absent; descendants
inside it are not counted again in another bucket. `minDirectoryBytes` appears
only when a positive threshold was selected.
Each bucket combines unexported immediate-child subtrees; `directoryCount`
counts all hidden descendants, while bytes and file counts sum each subtree once.
Each bucket is already included in the row totals. Direct files remain derivable:
subtract exported immediate-child totals and both buckets from the row.
Do not add ancestor rows or omitted-subtree buckets to the export root's totals.
The number of measured directories is the row count plus both buckets’ directory
counts. No synthetic path is used for either bucket.

Filtering and folding never change `omissions` or `sizeStatus`: these describe the scan,
including hidden descendants, rather than export detail. Missing both buckets
means all immediate child directories are shown. When hidden measurements have
mixed timestamps, report-level `scannedAtRangeMs: [earliest, latest]` preserves
that distinction; individual visible rows still carry differing timestamps.
No summary mode, ranking scores, row limits or duplicate global totals are serialized.

Scanning is not atomic and the source can change after measurement. Allocated bytes use the engine's native allocation/fallback;
logical bytes use file lengths. No export timestamp, prose notes, application
metadata, internal IDs, duplicate totals or parent paths are serialized. No file
content, complete file list or per-skipped-entry error list is included.

Export options copy only scan identity and accounting metadata from the session,
not the chart hierarchy or file entries. Both options and report preparation run
on blocking workers; opening the dialog performs no source filesystem IO. Filtering and
selection run only after the user exports. The export dialog dims the chart without a real-time backdrop blur.

Only the native picker supplies the output path. Serialization stays in Rust to
preserve 64-bit counts and avoid sending large reports through IPC. Output is
compact UTF-8 JSON written to a sibling temporary file, flushed and synced,
then atomically published. A failed write preserves the prior destination. Logs
record scan ID, scope/path mode, size threshold, filtered/collapsed counts, revision rejection reason, directory count,
destination, write stage, native failure and elapsed time, never report contents.
The formal CLI remains unchanged.

## Scan modes

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
The Linux native reader preserves permission, read-error and link/mount reasons.
It logs at most 16 IO failure samples per analysis while retaining every omission in the totals.
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
and runs the existing single-item safety checks, direct deletion and cache
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
