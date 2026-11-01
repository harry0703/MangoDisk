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

## Nested export of existing analysis

Export starts at the currently browsed directory. Depth 1 contains exactly the
published list, including rows outside the virtual viewport; depths 2–6 add
available directory and file detail beneath its directory rows. The default is
2 levels and 500 entries. Limits are 200, 500, 1000, 2000, or zero for all available
detail within the chosen depth. Directories and files share one scan-wide export
budget; the root container and accounting buckets do not consume entries.

Selection is breadth-first. At each level all eligible parents compete by
size, with stable native-path ties. The current layer is selected before deeper
layers. A bounded heap prevents wide directories from allocating an unbounded
candidate list for a limited export. Choosing all available detail explicitly
allows a larger temporary report; it never expands scan-time file retention.

Core verifies the native session's index generation, mutation revision, timestamp
and byte metric before reading the existing navigation, directory and retained
file indexes. No source enumeration, metadata refresh or automatic rescan is
allowed. Expired sessions and unavailable or superseded indexes return
`analysisExportUnavailable`; an explicit new analysis is a separate user action.
Opening the dialog uses local display metadata only. Report preparation runs on
a blocking worker and freezes its result before the native picker opens.

The GUI remembers format, depth, entry limit and full-path choices in its existing settings
document. Preferences load at application startup, so opening the dialog adds no
settings IO. Canceling export, navigating or starting another scan does not reset
these choices; scan IDs and report destinations are not stored as export preferences.

Formats are `json` (default) and `markdown`. Older requests and saved preferences
without a format retain JSON. Both formats use the same frozen report, selection
budgets and atomic-save boundary. The native picker uses the selected extension
(`.json` or `.md`); the format is not an additional JSON report field.

Markdown streams the tree with portable English evidence labels, exact integer
bytes, measurement times, size status and separate remaining/omission groups.
Names and optional paths are JSON-escaped strings inside variable-length code
spans, so special names cannot inject Markdown structure, links or raw HTML.
It does not read source files or build another full report text in memory.

Reports use `schemaVersion: 5`; versions 1–4 are earlier directory export formats.
Consumers must reject unsupported versions. Reports are write-only evidence,
not imported app state or deletion authority. Header fields are `byteMetric` and
`scannedAtMs`; the `root` object contains nested `children` arrays. Nodes contain
`name`, `bytes`, `isDirectory`, directory `fileCount`, optional `modifiedAtMs`,
and optional absolute `path` when requested. Default names match the displayed
list and can be lossy on filesystems that allow non-UTF-8 names. Folder and file
names can still disclose personal information.

Directory bytes and file counts retain their recursive measured totals. For each
node, the sum of immediate exported child totals and remaining buckets equals
that node's measured totals; ancestor and descendant values must not be added
together. Hard-link aliases retain scan-time charging, including zero-byte aliases.
A directory measured at a different time has its own `scannedAtMs`; ordinary nodes
omit the duplicate timestamp. This preserves composite caches after subtree refresh.

`remaining` is emitted only when needed, with disjoint buckets:

- `depthLimit`: children below the selected depth;
- `entryLimit`: available entries omitted by the shared export budget;
- `listLimit`: retained root entries outside the currently published list;
- `detailsNotRetained`: scanned files whose names were not retained in either
  the current list or the file index.

Each bucket contains measured `bytes` and recursive `fileCount` for its omitted
branches; `directDirectoryCount`, when present, counts omitted immediate child
directories, not all descendants. Missing fields/buckets are zero. A depth-limited
node omits `children`; an expanded node includes its known children, possibly an
empty array. Empty arrays do not prove that an inaccessible directory is empty.
Nonzero `omissions` and non-complete `sizeStatus` separately describe actual scan
coverage; export folding never increments scan omission counts. Omission counts
are recursive and must not be summed across ancestor and descendant nodes.

Directory/file accounting uses checked integer arithmetic. Inconsistent or stale
source records reject the export instead of silently inventing residual totals.
No file contents, fingerprints, internal IDs, export timestamps or prose notes
are serialized. Standard mode reports native allocated bytes; fast mode reports
logical bytes. Native 64-bit integers stay in Rust without a WebView JSON round trip.

Only the native picker supplies the destination. A sibling temporary file is
flushed and synced before atomic publication; failed writes preserve the existing
destination. The transport receipt uses schema version 2 with `entryCount`,
output `path` and `fileBytes`. Logs retain scan ID, selected depth/budget/path mode,
row count, destination, write stage, native errors and elapsed time, never report
contents. The formal CLI remains unchanged.

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
