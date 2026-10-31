# Permanent deletion

Domain services authorize selections, protected paths, exclusions, and required
application state. The filesystem boundary checks the live regular-file or
directory type and link policy, then removes entries at their original paths.
It does not create sibling directories, rename targets, or attempt rollback.

A lightweight identity check rejects ordinary same-path replacements observed
before deletion. Concurrent mutation after that check remains possible: deletion
is not a filesystem transaction. Unix deletion does not require content-read
access. Windows identity queries request attributes only and share deletion.

Large-file and analysis selections authorize the current regular target despite
normal size or timestamp changes. Duplicate files retain one size/mtime comparison
against the scan. Duplicate directories use scan-established content equality and
a directory metadata check; deletion does not rehash descendants. Changes that do
not affect those metadata checks are an accepted risk. Exact duplicate scanning
continues to require content equality.

Cancellable directory traversal counts observed removals, including zero-byte
files. Cancellation and failures leave surviving entries in place. Native tree
removal uses scan estimates on success and marks progress unknown on failure;
it never walks the remainder solely to infer how much was deleted.

Large-file and duplicate batches reconcile already absent selections separately
from successful deletions, without claiming reclaimed storage. Per-item permission
and busy failures do not abort unrelated selections. Diagnostics retain escaped
paths, stages, native error codes, partial-progress status and batch summaries;
they exclude file contents and bound repeated candidate diagnostics. Legacy
transport failure stages remain readable for history compatibility.
