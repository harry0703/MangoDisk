# Application uninstall diagnostics

The uninstall scan protocol is version 10. Its `uninstallDiagnostic` field is
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
The system fallback opens the fixed Windows installed-apps destination; it does
not execute an uninstaller or report reclaimed space. Rescan after using it.

The Windows command grammar and environment expansion tests run on macOS too.
Windows native registry, permissions, process launch, and shell behavior still
require Windows validation; a cross-target Rust type check is not a substitute.

Native fixture validation is opt-in because it briefly registers test entries:

```sh
cargo test -p mangodisk-platform live_record_removal_preserves_files_rejects_stale_snapshots_and_verifies_deletion -- --ignored
```
