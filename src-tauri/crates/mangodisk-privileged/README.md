# Optional Windows capabilities

MangoDisk runs as the signed-in user. This separately installed Rust service
owns the narrowly scoped operations that require Windows system privileges.
Installing, repairing, or removing it requires an explicit user action and UAC.
It is not installed by application startup or by ordinary resource monitoring.

## Capability boundary

The shared `mangodisk-privileged-protocol` crate defines versioned, typed
requests. Version 1 exposes only health information and CPU temperature.
App releases may share the same compatible service protocol; build versions
are diagnostic metadata and do not force reinstallation on every app update.
Incompatible protocol changes require an explicit repair.
Unknown operations, fields, versions, oversized frames, mismatched replies,
and invalid readings are rejected. Runtime IPC cannot install components,
select registers, execute commands, or accept filesystem paths.

The named pipe rejects remote connections. Interactive clients receive data
IO rights without permission to create another pipe instance. Clients compare
the native pipe-server PID with SCM and verify the protected service image.
Interactive users may query the service process image but cannot access its
memory, duplicate handles, terminate it, or modify its installation.

Future capabilities must add explicit request and reply types, validation,
resource limits, and capability-specific tests. A privileged mutation needs a
separate authorization and consent design; permission to read hardware facts
does not authorize cleanup, command execution, or arbitrary file operations.
Keep collectors independent and avoid a generic elevated command dispatcher.

## CPU temperature

The collector supports single-package x64 Intel DTS and AMD Zen families 17h,
19h, and 1Ah. Intel readings validate the sensor-valid bit, TjMax, and the
reported distance below TjMax. AMD reads the fixed `THM_TCON_CUR_TMP` SMN
register, applies its range selectors, and removes documented early-Zen Tctl
model offsets to report Tdie. It does not substitute ACPI thermal zones or
estimate temperature from utilization. ARM64, older AMD families, multiple CPU
packages, and environments without valid native sensor data remain unsupported.

The collector uses signed PawnIO with pinned IntelMSR and AMDFamily17 modules.
Installation deploys the module selected by the host's CPUID identity. Only
fixed temperature registers are read; no register or write command crosses IPC.
Intel reads run on the same processor and restore the thread's affinity. AMD's
SMN index/data transaction is protected by the shared `Global\Access_PCI` mutex
with a bounded wait, coordinating with other monitoring applications. Selecting
an SMN index does not change thermal controls, clock settings, or power limits.
Clients share a four-second cache whose original sample time is retained.
Idle collection releases the module and driver handle after fifteen seconds;
the service performs no periodic sampling without requests. Failed reads
discard the previous sample and use a bounded retry backoff.

The service starts automatically after installation so ordinary application
startup does not show UAC. Disabling the feature stops MangoDisk requests.
Removing the service retains PawnIO by default because other applications may
depend on it. Explicitly selecting driver removal also uninstalls PawnIO and
its verified driver-store packages; failures and pending reboots are reported. Installation files live in the protected Program Files directory.
Exclusive installation handles serialize mutations across app instances and
users. Files are staged before stopping the existing service, and failed
replacement restores its previous files and running state.

## Packaging and vendor artifacts

`scripts/build-privileged.mjs` builds the Windows helper before the desktop
executable embeds its exact bytes. Production helpers require Authenticode signing.
Sign the helper before embedding it and include it in release signature checks;
signing only the main application does not satisfy installation verification.
Release tooling may set `MANGODISK_PRIVILEGED_SIGN_COMMAND` to a JSON argv
array with a standalone `{file}` argument for the helper path. The build hook
invokes that signer without a shell and checks the resulting Authenticode
signature before packaging. Unsigned release helpers fail the build.
Unsigned debug helpers are restricted to development builds.

`vendors/pawnio/` holds versioned, unmodified official binaries and licensing
notices. `src/artifacts.rs` records pinned SHA-256 digests and size limits;
`src/vendor_payloads.rs` uses `include_bytes!` only in the Windows helper binary.
The Windows desktop embeds that signed helper as an opaque image; non-Windows
builds carry neither the helper nor vendor payloads.
The frontend displays component attribution and embeds the unchanged LGPL license
text in a lazy About > Third-party licenses dialog, shared by installed and
single-file portable builds. Viewing and copying work offline; packaging does not
deploy external files under a `licenses` directory. The engineering provenance
notice remains in the source repository.
No runtime vendor download or user-writable vendor staging directory is needed.

Installed and single-file portable builds share the same bootstrap path:
`%LOCALAPPDATA%\MangoDisk\privileged\MangoDiskPrivileged.exe`, resolved for the
interactive user through Windows Known Folders. The desktop stages the embedded
image only when installation, repair, or removal is requested. Installer scripts,
the working directory, and the desktop executable's directory do not supply it.
Identical bytes are reused; outdated or corrupt bytes are replaced atomically.
Staging rejects reparse points and serializes operations across app instances.
Pinned directory and image handles prevent replacement during verification and
elevation. Release builds verify Authenticode before UAC; the elevated helper
also retains its existing self-verification and protected service installation.

Installing executes the staged helper with the fixed
`--install-cpu-temperature` action and no client-supplied path. It validates
embedded bytes, reuses compatible installed PawnIO, and releases the official
installer into its protected directory only when the driver is required.
Removal uses the fixed `--uninstall` action, or `--uninstall-with-pawnio` only
after explicit consent to remove the shared driver. Both run from the staged
helper; callers cannot supply a vendor path or native command.
The installed service image rejects self-removal before modifying SCM state.
Keep the original vendor files unchanged and update digests, notices and integrity
tests with each version.

Enabling fails closed on verification or installation failure; failed repair
restores the previous files and running state. A required reboot is reported
explicitly; enabling succeeds only after a valid native sample.
The UI keeps CPU temperature in the existing reorderable display-item list.
After explicit enablement, the application maintains temperature history at a
four-second cadence independently of display selection or panel visibility.
Hiding resident display does not revoke the Windows capability. Detailed CPU
frequency and process queries remain panel-driven; temperature failures retain
bounded retry backoff. Exiting the application stops its requests, allowing the
service to release its collector after the idle timeout. Removing the MangoDisk
service disables this capability. The shared PawnIO driver is retained unless
the user explicitly chooses to remove it.

The helper and its test executables declare `asInvoker` explicitly to avoid
Windows installer-detection heuristics. Installation and removal request
elevation explicitly; service execution inherits the SCM account.

## Diagnostics

The helper embeds MangoDisk's Windows icon and product metadata. Installation,
repair, removal, driver installer exits, rollback failures, service lifecycle,
and collector status changes are recorded in the Windows Application event log
under `MangoDiskPrivileged`. Events include stages, native errors and elapsed
time where applicable. Control characters are escaped and messages are bounded.
Successful cached requests are not logged individually.

Dependency inspection distinguishes a missing or incompatible PawnIO package from
an installed library whose signature cannot be trusted. The latter blocks setup
instead of triggering driver reinstallation. Native Authenticode status codes
survive bootstrap errors and the elevated helper exit status. The setup dialog
shows untrusted-root errors separately, including the hexadecimal status and
Windows certificate-update troubleshooting guidance. It never imports trust
roots, changes certificate-update policy, or bypasses signature validation.

## Validation

`pnpm check` includes protocol, cache, replacement, and Core regression tests.
The Rust check and Clippy scripts prepare the Windows helper before the desktop
embeds it. For direct Cargo application checks on Windows,
run `node scripts/build-privileged.mjs` first.
Windows validation additionally covers standard-user IPC, protected installation
permissions, failed repair recovery, malformed and stalled clients, idle cost,
and the settings interaction. The `ordinary_client` diagnostic example records
its actual elevation state and exercises the production client implementation.
Virtual machines can verify installation gates and unsupported behavior; they
cannot establish physical sensor accuracy.
