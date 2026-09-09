# Application uninstall diagnostics

The uninstall scan protocol is version 11. Its `uninstallDiagnostic` field is
an optional typed reason for a rejected Windows registration. It is null when
an executable registration was accepted. Catalogs are transient snapshots:
rebuild them after upgrading instead of reusing an earlier scan schema.
Uninstall plans, inspection schemas, and persisted history are unchanged.

## Investigating a disabled or failed application

1. Find `application_uninstall_details_opened` when the user expands an item.
   Its redacted `application_id` links that interaction to inventory, Core, and
   execution logs. Technical IDs and rejection details are not shown in the UI.
2. Find `windows_uninstall_registration_rejected` for that reference. Its scope,
   registry view, typed reason, parse detail, and native error code distinguish
   missing commands, malformed commands, inaccessible executables, and conflicts.
   `quiet_command_present` explains whether a vendor registered only a silent
   command; it does not authorize silently replacing the interactive uninstaller.
3. Find `application_uninstall_candidate_blocked` for the final catalog decision.
   One registry view can be rejected while another valid view supplies the final
   uninstaller, so a rejected source is not itself the final application status.
4. For execution failures, follow the application ID and plan ID through
   `application_uninstall_native_started`, `windows_uninstall_registration_changed`,
   `application_uninstall_native_execution_failed`, and
   `windows_uninstall_postflight`, and `application_uninstall_item_finished`.
   Postflight records both the vendor exit code and the observed registration
   state, including nonzero exits. The item result also links to the Core
   operation ID. Installer exit codes must never be attributed to an application
   by timestamp proximity alone.

Keep these events available at INFO/WARN. Do not add raw registry keys, command
lines, installation paths, or application names to make correlation easier.
New rejection branches must supply a stable reason or detail, with regression
coverage, rather than returning an unexplained `None`.

## Windows safety and fallback

An orphaned registration means known registered paths could not be found on an
accessible drive root. Permission errors, unrecognized commands, conflicting
registrations, and dangling links do not prove removal. This classification
does not authorize deleting similarly named application data. A separate,
confirmed remove-entry action deletes the selected uninstall registration and
its owned child keys. Registered files, uninstall commands, installer flags,
running processes, and unrelated catalog changes do not restrict this explicit
action. No application files or installer product databases are modified.

Registry transactions keep target snapshot validation and deletion atomic.
Registry links are never followed. Machine entries use the dedicated version-1
helper with ID/digest arguments only, revalidating the same target after UAC.
Dry-run resolves and snapshots the same registry tree without writing. Already
absent entries are successful no-ops. Completion verifies the registration is
absent and never reports deleted files or reclaimed space.

Follow `application_record_removal_started`,
`windows_application_record_removal_resolved`,
`windows_application_record_removal_failed`, and
`application_record_removal_finished` for removal diagnostics. Failures include
a finite execution stage, reason, and native code, including elevated failures.
User cancellation stays cancellation through Core and the command adapter.
An uncertain write returns `details.mutationState=mayHaveChanged`; the UI
refreshes the catalog instead of inviting a retry against a stale entry.
After verified success, the UI removes only that record and preserves the current
view. `record_removed_from_catalog` reports the removed/remaining counts with
`rescan=false`. The adapter retains diagnostics for the other rows but invalidates
execution-cache reuse; the next explicit preparation obtains fresh evidence.
`application_uninstall_catalog_record_removed` confirms this cache transition.
The system fallback opens the fixed Windows installed-apps destination; it does
not execute an uninstaller or report reclaimed space. Rescan after using it.

The Windows command grammar and environment expansion tests run on macOS too.
Windows native registry, permissions, process launch, and shell behavior still
require Windows validation; a cross-target Rust type check is not a substitute.

Native fixture validation is opt-in because it briefly registers test entries:

```sh
cargo test -p mangodisk-platform live_record_removal_preserves_files_rejects_stale_snapshots_and_verifies_deletion -- --ignored
```

## System-item visibility on Windows

`systemKind` is presentation evidence, not uninstall permission. The application
page starts with system items hidden, matching startup management. Showing them
restores the ordinary row actions. Hiding them removes their batch selections;
search, status counts, select-all and summary bytes use the same display scope.
Explicit record removal does not consult this classification.

Core classifies Windows system-signed packages, an exact list of built-in package
families (including publisher IDs), shared media/language/integration packages,
and narrowly matched Microsoft shared-runtime registrations. Coverage includes WSL,
Quick Assist, legacy Mail/Calendar, the Bing Windows search extension, SQL Server
CLR Types, and versioned Windows App Runtime Main/Singleton/DDLM packages. Runtime
identities include the publisher ID and bounded version/channel/architecture
syntax; a runtime-looking display name alone is insufficient. Microsoft publishing, a display-name prefix, installation under
Program Files, or an unknown classification alone must not hide an application.
Maintain the family/runtime rules in `uninstall/system_classification.rs` and
increment its rule version when coverage changes. Unknown entries remain visible;
this is not a promise that they are safe to remove. macOS behavior is unchanged.

To investigate a missing item, distinguish native exclusions from UI filtering:

- `windows_packaged_application_filter_summary` reports distinct excluded packages
  and overlapping native flag counts; protected/framework packages remain outside
  the uninstall catalog even when the checkbox is enabled.
- `windows_uninstall_inventory_filter_summary` reports exclusions by registry view
  and finite reason. These counters are produced when inventory is refreshed.
- `application_system_classified` links each positive decision to the redacted app
  ID and rule version. `application_system_classification_summary` is emitted on
  each Core scan, including scans reusing cached platform inventory.
- `system_filter_applied` persists the checkbox state and total/visible/hidden
  counts; `selection_pruned` records how many selected applications were removed.
  These numeric/boolean fields are intentionally part of the native log message:
  arbitrary frontend log context is console-only. Search text is never logged.

The added AppX inventory envelope is internal schema version 1. Its producer and
consumer ship together; incompatible envelopes fail the inventory read explicitly.
The public version-11 scan remains transient and must be rebuilt after upgrade.

Registered Windows `.bat` and `.cmd` uninstallers use the same explicit confirmation,
Shell launch, process tracking, and registration postflight as executable uninstallers.
Their original arguments remain unchanged; MangoDisk does not construct a `cmd /c`
command or change file associations. Machine-wide batch registrations request Windows
UAC elevation for the whole script so HKLM cleanup runs with the same rights as
vendor tools. Current-user batch registrations retain the ordinary user token.
The typed `batch-script` identity participates in
registration and plan fingerprints. A successful process exit alone never proves
that the application was removed.

`windows_registered_batch_uninstaller_recognized` identifies accepted script entries.
`windows_registered_batch_uninstall_requested` records registration scope, elevation
intent, target kind, and argument presence before launch. Rejections distinguish `unsupported_target_extension`,
`target_not_regular_file`, and metadata errors with a finite `target_kind` field.
These events omit paths and command text. Validate the real Shell path on Windows
with `cargo test -p mangodisk-platform registered_batch_fixture -- --ignored`;
that test creates and removes only disposable HKCU registrations and script files.

A native uninstaller can return an error after removing its exact registration.
The additive `nativeInstallerFailedAfterRemoval` action reason preserves that failure
in results and history while allowing the frontend to remove the verified stale row.
It does not imply that all files were removed, estimate released space, or authorize
associated-file cleanup. Unknown or still-present registration states retain the row.

Registered DLL uninstallers use an explicit `rundll32` command kind. Only absolute
System32/SysWOW64 Rundll32 paths resolved against the native Windows system directory
are accepted; the registered host bitness is preserved. DLLs must be regular local
files with a `.dll` suffix and a named export. Relative DLL lookup, network/device
paths, alternate streams, and ambiguous separators are rejected. Inventory never
loads vendor libraries. DLL paths are environment-expanded once and quoted; export
case and remaining vendor arguments are preserved. Machine registrations request
UAC for the host, and all results retain process/registration postflight validation.

`windows_registered_rundll32_uninstaller_recognized` identifies supported records.
`windows_registered_rundll32_uninstall_requested` records host kind, a finite export
category, parameter presence, and elevation intent. Rejections distinguish host
provenance, argument parsing, and DLL metadata without logging paths or commands.
The explicit Windows test `registered_rundll32_fixture` compiles disposable x64/x86
DLLs and tests real System32/SysWOW64 launches against its own HKCU records. It needs
both Rust MSVC targets and runs with `cargo test -p mangodisk-platform
registered_rundll32_fixture -- --ignored`.

For the `NVI2.DLL,UninstallPackage` calling convention, NVIDIA Installer 2 result
`0xE0E00001` means cancelled and `1` means restart required. Other DLLs/exports retain
the generic result policy. Cancellation is accepted only while the same registration
is confirmed installed; a nonzero exit after removal retains partial-removal handling.
Postflight logs use unsigned decimal and hexadecimal exit codes. Vendor cancellation
logs retain the application reference and differ from UAC cancellation. Full scans
log each system classification; execution preflight logs classification totals only.

Reference: [NVIDIA Installer 2.0 Command Line Guide, return codes (page 9)](https://cdck-file-uploads-global.s3.dualstack.us-west-2.amazonaws.com/nvidia/original/3X/e/e/eefd529f360cd00050d5c2b0798a3b8c5c721ce3.pdf).
