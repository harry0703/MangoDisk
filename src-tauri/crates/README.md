# Rust crates

This directory contains reusable Rust capabilities:

- `mangodisk-core` owns product domains, use cases, rules, indexing, cleanup,
  history, and reporting.
- `mangodisk-platform` implements macOS and Windows contracts for volumes,
  paths, links, system exclusions, application inventory, and permanent deletion.
- `mangodisk-cli` is a sibling adapter over Core use cases.

The Tauri crate only assembles the application, converts command arguments,
and forwards progress events. It does not own platform policy or scanning
behavior.

## HTTP clients

Application HTTP clients use `mangodisk_core::http_client::builder()` to send
the shared `MangoDisk/<version> (<OS> <OS version>; <architecture>)` user agent.
The application version comes from the workspace; basic system identity comes
from Platform and is cached for the process lifetime. Missing OS versions use
`unknown`; native version labels are bounded and made safe for HTTP comments.
Callers configure their own timeouts, redirects, retries, and authentication.
Third-party clients use `http_client::default_headers()`; the Tauri updater
receives these headers at plugin registration for update checks and downloads.
AI custom headers can override `User-Agent` for their own requests.

First-party update discovery, feedback and public-IP requests opt into
`http_client::OfficialRequestMetadata::headers()`. The desktop collector owns
locale, distribution, OS facts and read-only reuse of an existing installation ID;
Core owns header names, bounded encoding and optional-value validation. Missing
identity or other telemetry never blocks a request, and the generic HTTP builder
never acquires this metadata. Feedback retains its existing multipart payload,
with the same request and installation IDs in its headers. Update discovery does
not follow redirects. Update downloads keep only the shared user agent because
package URLs may redirect to a third-party CDN.
Desktop Mock App tests use the same Windows manifest as the application, including
Common Controls v6 and `asInvoker`; no machine-wide DLL configuration is required.

## Scan exclusions

Adapters select per-module scopes and pass an immutable `ScanExclusionOptions`
into Core. Exact name rules distinguish files from folders, match case-sensitively
at every depth on all platforms, and accept no paths or wildcards. Native walkers
prune matching directories before descending; cached results use the same policy.
Name rules are included in the index configuration fingerprint. Windows volume-wide
NTFS layout aggregates cannot represent name exclusions, so active name rules use
the directory-walking fallback, as path exclusions already do. The fallback reason
is logged explicitly; unfiltered scans retain their existing native fast path.
Disk-analysis sessions retain the resolved exclusion paths used by traversal, so
parent deletion cannot bypass exclusions expressed through system path aliases.
Storage and cleanup share `filesystem::exclusion_paths` for exclusion validation
and resolution. Missing descendants retain the resolved existing parent; ambiguous
parent components and user links are rejected. Disconnected volume roots remain
configured. Fixed system aliases and path identity comparisons belong to Platform;
domain modules own only scope, pruning, and cache policy.

Legacy cleanup path exclusions protect project artifacts only. Cleanup name rules
also protect declarative and custom rules. Atomic directory removal checks for
protected descendants before deletion and again during staged removal. Specialized
cleaners that cannot preserve names return an explicit, unselectable `excluded`
status. The deep-cleanup adapter skips application leftovers with active name
rules and rejects execution after the exclusion snapshot changes.

Frontend exclusion preferences migrate schema 2 to schema 3 by preserving path
scopes and adding an empty name list. No new exclusions are enabled by migration.

For a repeatable storage workload, build the `scan_exclusion_benchmark` Core
example in release mode and supply an isolated fixture directory plus `none`,
`paths`, or `names`. It creates 6,144 dependency files, 24 source files, and two
64 MiB duplicates. Alternate baseline and candidate processes over the same
fixture, discard warmups, and compare repeated measurements with matching result
counts. Keep fixture data and raw measurements outside tracked source.

## Read-only scan metadata

Initial directory discovery uses `filesystem::metadata::scan_entry_metadata`.
Windows reuses file facts from directory enumeration instead of opening every
file again. Directories retain a live no-follow query before descent; Unix retains
its existing no-follow metadata query. Analysis, initial result assembly, and
generic duplicate discovery share this primitive. Reopened analysis lists,
destructive preflight, and content verification must keep
their live path or handle queries and must not use enumeration snapshots.

Windows file-space measurement retains `GetCompressedFileSizeW` for every file.
WOF compression can expose ordinary attributes in both directory and live
metadata, so those flags do not authorize substituting logical length for native
usage. Directory `AllocationSize` must not be substituted without separately
validating that change in accounting semantics. Include WOF compression in scan
regression fixtures; conventional NTFS compression does not cover this case.

The `storage_scan_benchmark` Core example measures an existing read-only fixture
in `analysis`, `large`, or `duplicates` mode. Set `MANGODISK_BENCHMARK_STATE_ROOT`
to a separate state directory and build the example in release mode. Preserve
baseline and candidate executables, run them as the same ordinary user, alternate
their order, discard warmups, and compare repeated medians together with counts,
bytes, skipped entries, and result digests. The digest describes returned result
rows; it is not a proof of every file in a truncated UI projection. Use regression
tests for safety and full-content correctness. Analysis reports its full traversal
file count separately as `analysis_files_observed`. Include both a wide tree and one
large directory; benchmark special files separately. Do not clear OS caches or
change machine security settings to manufacture a speedup.

Windows analysis fallback can distribute directory work across a bounded worker
pool. The coordinator owns directory folding, fingerprints, and deterministic
candidate selection; workers retain native file-space queries. A large selected
root can split its direct files into batches, with live no-follow checks before
queued files are measured. Windows ARM64 keeps descendant directories as independent tasks to avoid
additional queued per-file metadata queries.
Windows x64 also splits large descendants, which improved the measured NTFS workload. Initial
result assembly can parallelize uncached direct-file usage queries; reopened lists
retain live queries. Cancellation disconnects queues and joins workers before
returning. If worker creation fails before dispatch, analysis uses serial traversal.

For repeatable experiments, `MANGODISK_WINDOWS_ANALYSIS_WORKERS=1..16` overrides
the analysis fallback's worker count. Invalid values log a warning and use automatic
selection. Automatic selection keeps HDD, network, removable, and unknown device limits
conservative. SSD analysis uses up to four workers on ARM64 and twice available
CPU parallelism capped at sixteen on x64. These are measured workload policies,
not a claim of universal optimality; generic content-scan limits remain unchanged. This override does not change native NTFS layout,
large-file scanning, duplicate content checks, or privileged capabilities. Test
ordinary and elevated execution separately: NTFS layout can have different scope
and allocation semantics, so its time alone is not an equivalent ordinary-user
speedup.
