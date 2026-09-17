# Resident system status

The resident adapter combines independent Core readings into native menu-bar/tray
entries and one reusable detail panel. It does not own filesystem cleanup,
application termination, or memory-reclamation algorithms.

## Ownership

| Area | Responsibility |
| --- | --- |
| Platform `system_resources` | Native counters, interface identity, local volume identity and capacity |
| Core `system_resources` | CPU/network/disk I/O deltas, selection policy, freshness, bounded trends and memory use cases |
| `sampling_workers` / `sampling_schedule` | One bounded worker per metric, demand, deadlines and generation checks |
| `runtime` | Cached snapshots, status transitions and sampling coordination |
| `presentation` | Bounded, coalesced visible-window publication and native display updates |
| `tray_display` | Shared formatting, localization, native entries and Windows bitmap ownership |
| `taskbar_display` | Windows native window, shell geometry, task-button space lease, painting and fallback |
| `preferences` / `preference_schema` | Version migration, revision conflicts, native application and persistence rollback |
| `panel` / `main_window` | Independent presentation, source anchoring, focus and background-launch behavior |
| Vue resident settings store | Serialized optimistic edits and rollback to committed preferences |
| Vue settings / tray panel | Configuration preview and presentation of actual readings |

## Contracts and cadence

Fresh installations enable resident display with Logo, CPU and memory selected;
network and disk remain unselected. Windows starts in taskbar mode, preferring
automatic placement with background enabled. Automatic placement prefers right on
Windows 10, left on centered Windows 11 taskbars, and right on left-aligned ones.
The existing shell inspection refreshes this decision every second; manual choices
remain fixed. Windows 10 reserves the selected edge of its task-button container,
so even uncombined application buttons leave room for the monitor. Windows 11
reserves XAML space at the outer left edge for centered taskbars, beside Start
for left-aligned taskbars, or after the application-button repeater, then
returns that reserved slot directly, so centered buttons do not send the monitor
to an unrelated outer gap. Unknown environments prefer right,
and collision checks still apply. These defaults do not replace saved choices.

Resident preferences use schema version 8; resource snapshots use version 3.
Version 1 preferences retain background and memory-display choices. Version 2
preferences retain all selections and default to the original Windows tray mode.
Version 3 retains that mode and defaults the new position preference to right.
Version 4 retains all choices and enables the new taskbar background option.
Version 5 retains its saved left/right preference; only new installations default
to automatic placement. Version 6 retains automatic/manual choices and defaults
the new compact mode to off. Compact mode reduces horizontal cells from 50/84
to 38/58 DIP (percentage/network), with abbreviated network units
(B/K/M/G/T) and unchanged numeric precision. Labels use 9-DIP text and values
use 12-DIP text in both densities; paint and hit testing share those bounds.
Unknown persisted
versions are rejected for writes. Memory snapshots and release results retain
their separate version 1 contract.

CPU samples every second on Windows and every 2 seconds on macOS; memory
samples every 3 seconds, network every 1 and disk every 30.
Their freshness limits are respectively 5, 10, 5 and 90 seconds. All base metrics
remain active while resident display is enabled, regardless of the selected native
entries or panel visibility. Disabling resident mode stops periodic collection;
explicit device-catalogue requests may still read network and disk metadata.
The panel has an overview of all four base readings and a separate memory page.
Process details are requested only by the memory page or startup icon warming;
opening the overview does not enumerate processes. Reopening preserves the last
selected tab within the application session; a new process defaults to overview.
A different metric entry can navigate an already open panel.
CPU and network require two valid observations; unavailable values remain `—`.
Windows keeps one PDH query on its CPU worker and uses language-neutral
`Processor Information(_Total)` counters. Windows 10 and older Windows 11
builds use `% Processor Utility` (frequency-weighted capacity); Windows 11
26100.3624 and later use `% Processor Time`, following the Task Manager change
introduced by [KB5053656](https://support.microsoft.com/help/5053656).
The update was a gradual rollout, so the build policy cannot detect every
feature-flag state or guarantee identical samples across Task Manager versions.
macOS retains its Mach aggregate tick source. PDH failures fall back to
`GetSystemTimes`, explicitly log the source/stage/native code, and retry after
30 seconds. Only valid intervals are published, and source switches reset tick
baselines. Re-enabling monitoring or resuming after a long pause primes a new
native interval. Percentages saturate at 100 for legitimate turbo utility;
missing, negative and non-finite samples are never converted to idle zeros.
`cpu_source_selected` records the version policy; source/recovery logs and
one-minute `cpu_sample_summary` aggregates (including actual window duration)
support user-log diagnosis without writing every sample at INFO. Summaries reset
when sampling is interrupted or falls back, so resumed windows exclude old peaks.
Independent refresh phases and averaging windows
can still produce transient differences between the two applications.
Each native trend retains at most 96 points over 80 seconds for the 60-second viewport. `observedAtMs` anchors the time
axis even when the latest valid sample is older than the current snapshot.

A slow native query stays in flight until it returns. Changing demand invalidates
its generation without spawning replacement threads. Native panel focus explicitly stops chart animation even when WebView2 leaves
`document.hidden` false. Hidden WebViews receive no periodic reading events and reload the native cache when opened. Native display
updates from one completion burst are coalesced over 50 ms and periodic native
refreshes are limited to once per second. A separate presentation worker uses one
bounded wake slot and reads the latest cached snapshot; a slow native UI operation
cannot block sampling or accumulate old snapshots. Preference changes and periodic
native refreshes share a separate transaction gate; sampling only briefly reads
the committed settings snapshot. Native application, persistence and rollback do
not hold that snapshot lock. Refreshes read settings after acquiring the gate so
an older refresh cannot overwrite a committed display. Failed updates retain the
previous settings, and successful updates log their elapsed time.
`resident_presentation_delayed` and
`resident_sampling_delayed` distinguish UI waits from coordinator stalls; recovery
is logged once, and the periodic sample summary includes maximum loop latency.

macOS interface names and physical-interface classification are cached for at
most 10 seconds, with immediate invalidation when interface topology changes.
Connection state, routes and byte counters are always read on each network tick.
Automatic selection avoids virtual/loopback interfaces; an explicit selection is
never silently replaced. Only local writable/system or user-mounted disks are
listed; a selected volume is identified independently of its current mount name.

## Native and interaction boundaries

macOS uses one combined entry with a Retina-aware drawing-handler image. Labels
and values occupy two rows; network directions retain colored arrows and explicit
units. Rate digits are right-aligned in a fixed field; units have a separate fixed
origin so digit-count and unit changes cannot move neighboring text. Native column geometry keeps numeric changes
from resizing the entry (220 pt for all metrics plus the brand). Native appearance
and recreated buttons invalidate the image cache; accessibility retains the textual
summary. The adapter
uses Tauri 2.11 native tray access; its minor version is constrained so an upgrade
receives native UI regression checks. Windows uses at most six retained tray handles and
bounded native-size bitmaps. Logical entry IDs are stable within MangoDisk; they
are not Windows notification GUIDs and do not guarantee retained shell placement
across restarts or changed selections. Windows controls icon order and overflow.
Hidden Windows entries are absent from the notification area even while their
handles are retained. Update their tooltip only after showing them again; a
native modify call on a hidden entry fails and can reject a preference change.
When opening a Windows panel, move it to its target monitor before applying
physical dimensions; a hidden window can still carry the previous monitor DPI.
The current Windows CPU source reports `unsupported` for multiple processor groups
instead of presenting a partial group as whole-machine usage.

The settings display card owns the resident enable switch; disabling it collapses
its options without resetting metric, source or appearance preferences. Login
startup is configured independently in General. A login launch stays hidden only
when resident display is enabled. Disabling display keeps the main window open;
Windows closes the application when that window is subsequently closed, while
macOS preserves Dock reopening. Explicit Quit always exits the application.

Settings apply directly to the native status surface without a duplicate simulated
preview. The brand toggle is a fixed card
beside the reorderable metric cards; it is not part of metric ordering. macOS metric reordering
uses local pointer events because native file-drop handling is also needed by
cleanup pages. Keyboard reordering restores focus after keyed DOM moves.

Diagnostics record status transitions, network selection reasons, failure
stages/codes, sampling percentiles, discarded generations, tray updates and tray-handle counts. Device identities,
private mount paths and raw machine reports do not belong in application logs or
committed validation reports.

Run `pnpm check`, Core tests, Platform system-resource tests and resident-adapter
tests on applicable macOS and Windows environments. Native acceptance includes
cold/warm opening, overflow anchors, focus, lifecycle/login startup, disconnection,
volume removal, native-size readability and release-build resource measurements.
Exercise all 32 metric/brand combinations in the real notification area, including
brand-hidden transitions and the all-disabled fallback; model-only tests cannot
verify the native entry lifecycle.


## Optional Windows taskbar display

Windows can show the same readings in either retained tray icons or one native
layered Win32 child of Explorer's taskbar: the Windows 10 rebar, or the
Windows 11 taskbar itself. Only the native monitor surface is parented; main
and detail WebViews remain independent. On Windows 10, an unelevated companion
process reserves space by moving/shrinking `MSTaskSwWClass` within the rebar;
Explorer still owns application-button layout and overflow. Closing the private
stdin pipe releases this lease on disable, normal exit, or abrupt GUI-process exit.
A session-local mutex serializes companion lifetimes, including restoration, so
rapid disable/re-enable cannot overlap allocations. Restoration only touches the
same shell process/control and our last applied axis; a newer Explorer layout wins.
Cross-axis DPI changes preserve the current thickness while restoring our axis.
The companion records reservation/restoration receipts in the existing log, even
if the GUI has died. It is neither installed nor elevated and never starts Tauri.
Its newline-delimited JSON protocol is version 1, rejects unknown fields/versions,
and caps packets at 4 KiB. Only the discovered primary task-button HWND is accepted.
On Windows 10, killing the companion itself (or the whole process tree) bypasses
its cleanup; the HWND lease is not a persistent recovery journal. Windows 11 uses
the same companion protocol with an embedded, architecture-matched C++/WinRT DLL.
A temporary XAML diagnostics enumeration matches the island HWND to the primary
Shell_TrayWnd and disconnects after obtaining weak references. Layout mutations
stay on Explorer's UI thread. The active lease observes the companion process,
so whole-process-tree termination also restores the XAML properties. An idle
channel keeps only weak references and runs no timer. Its content-addressed DLL
may remain mapped until Explorer exits; the per-user cache avoids locking update
or uninstall files. A build-specific channel identity prevents reuse of old code.
Old cache files are removed on a later startup when Explorer has released them.
When the app and Explorer have different process architectures (for example, an
x64 build running on ARM64 Windows), child hosting still works but the XAML DLL
cannot load into Explorer. Detect this before launching the companion and use
collision-checked free taskbar space instead. No button space is reserved in this
mode; insufficient free space retains the existing NoSpace status. Architecture
query failures use the same conservative placement and log the native error.
Recreated shell hosts repeat this check; compatible builds retain leased reservation.
Unknown environments leave the architecture check pending. The first usable
Windows 11 snapshot performs it, including after environment detection recovers.
Fullscreen and hidden-shell states are evaluated before space allocation, since
transient accessibility gaps during fullscreen are not evidence of insufficient space.
Windows builds require the MSVC C++/WinRT headers supplied with the Windows SDK.
Child creation temporarily adopts the parent's per-monitor DPI context on the
native thread, then restores the previous thread context. Process DPI is unchanged.
The Windows manifest declares Windows 10/11 compatibility for layered children.
The surface is recreated after Explorer restarts; no WebView or sampler is recreated.
The OS handles top-level taskbar ordering and menu occlusion. Position changes
order the child within its parent, without making it globally topmost.
The Windows 10 primary taskbar supports all four screen edges; the Windows 11
native taskbar uses its standard bottom edge. Horizontal bars use columns;
vertical bars stack cells and split network values/units into four lines. Painting,
hit testing and panel anchors share these rectangles. Window regions are resized
after positioning, since SetWindowPos does not resize a previous rounded
region when switching orientation. Physically impossible surface sizes, unavailable
shell capabilities, or physically unsupported monitor dimensions use the existing
tray entries and expose a typed status to settings. The
saved mode stays unchanged, so a usable layout can recover automatically.

A dedicated MTA thread reads cached UI Automation control bounds once per second.
While the taskbar is offscreen, it checks only its bounds every 200 ms and skips
UI Automation; this normal auto-hide state does not activate tray fallback.
A separate native window thread handles input, paints a small GDI backbuffer and
checks visibility every 100 ms while enabled. Out-of-context foreground
notifications also wake fullscreen checks immediately. Notifications are coalesced,
our own thread is excluded, and hooks are removed before recreating the window.
Subscription failure retains the timer fallback. No desktop-reorder hook or
occlusion retry is needed for a child window.
Gap placement rejects geometry older than three seconds. An active reservation
may continue during a slow UIA query only while the companion has confirmed its
layout within three seconds and live shell bounds, DPI and alignment still match.
A delayed UIA sample and its recovery are logged; stale helper replies are rejected.
Shell calls cannot block Tauri's event loop or resource samplers.
Model updates replace one bounded snapshot, and GDI objects are released after
painting. Disabled taskbar presentation stops the window timer. The shell query
thread performs no inspection while tray mode is selected or resident display is disabled.

Placement stays within the parent client area and excludes occupied controls with
a margin. During taskbar orientation changes, empty or inconsistent parent bounds
wait for the shell layout to settle instead of activating no-space fallback.
Windows 10 manual left/right reserves the start/end of the task-button container
(top/bottom on a vertical taskbar). Button crowding does not activate tray fallback.
The lease remains allocated during fullscreen/auto-hide, avoiding needless button
reflow. With centered Windows 11 buttons, the repeater's positioning margins
remain untouched. Only a natural button span that would collide with the selected
monitor edge activates a maximum-width limit; ordinary layouts retain the
original screen center. Crowded/uncombined buttons use Explorer's constrained
layout and overflow behavior. A constrained lease measures the unconstrained span
at most once per second unless its size or placement changes. It does not arrange
the intermediate state and restores the final constraint before layout. This
avoids oscillating on the visible-only overflow width or retaining recycled
button slots after applications close. The monitor stays at the selected outer
edge, and releasing the lease restores the original maximum width. With
left-aligned Windows 11 buttons, left placement reserves the Start button's right margin and preserves its real
minimum width so its hit-test bounds remain valid; right placement reserves the
application repeater's right margin. The embedded XAML request uses version 2
and rejects other versions; the DLL content key isolates different builds.
Restoring properties is conditional on the last applied value, preserving later
changes made by Explorer or another tool.
Comparison tolerates XAML float-storage precision at fractional DPI; exact
equality would mistake our own margin for an external update and compound it.
Unknown shell versions use the existing non-mutating gap placement.
A changed shell layout is detected on the next inspection. Temporary fullscreen/auto-hide visibility differs from a layout
failure, which activates tray fallback. Fullscreen detection requires a foreground
window covering the taskbar's entire monitor. Frameless windows may retain their
maximized flag, as browsers do. A non-maximized window whose outer bounds exactly
match the monitor is also accepted even if resize styles remain, as in WPS.
A framed maximized window with an auto-hidden taskbar is not classified as
fullscreen. Explorer's WorkerW and Progman desktop
hosts are excluded by class and shell process identity: clicking the wallpaper
must not hide the strip just because the desktop covers the monitor. System-installed
StartMenuExperienceHost and SearchHost CoreWindows are also excluded because
their opening animation can temporarily cover the full monitor.
Visibility transitions log their reason,
foreground window class/style/rectangle, and monitor-strip bounds; unchanged polls
do not repeat these messages. Native embedding logs record parent/window handles,
DPI awareness. Lease logs separately record original/applied rectangles, edge,
release reason and native restoration result. Position failures record the native
error and attempted bounds. No-space transitions include the shell/parent bounds,
required surface size and occupied control count. No control names or application
titles are collected.

Shell menus naturally cover overlapping pixels without hiding the entire strip
or raising the taskbar. Ordinary clicks use the existing focused detail panel,
anchored to the clicked column; a second click toggles it closed. The product
main window is not revealed. Right click exposes Open, Settings and Quit. Window callbacks guard
against synchronous Win32 message reentry. Settings enable pointer/keyboard reordering only where the chosen surface
can honor it (macOS and Windows taskbar mode).


The Windows taskbar background option defaults to opaque. Both modes stay layered
so Explorer composition cannot cover ordinary GDI child painting. Transparent mode
uses DirectWrite grayscale text and premultiplied BGRA. A cached
software Direct2D DC target renders colored glyphs directly into alpha, avoiding
the previous white-on-black GDI intensity-to-coverage conversion. Regular Segoe UI
keeps stroke weight close to the opaque reference; both paths retain the same
physical label/value font sizes and cell rectangles. Factories, target and DPI-specific text
format stay on the native window thread; a failed frame discards them for recovery. Background pixels use alpha 1/255
rather than zero so clicks still reach the entire cell; hover raises that alpha
to 28/255. ClearType remains enabled only for the opaque, known-background path.
Mode transitions change the native window style and invalidate its surface.
The transparent surface is presented before publishing its bounds for interaction,
because a newly layered window has no hit-testable pixels. Allocation/presentation failure
hides the surface and activates the existing tray fallback; diagnostics record
the failing stage and recovery, not every frame.

Windows taskbar network columns keep a fixed 84 DIP width, or 58 DIP in compact mode. The arrow, right-aligned
value and unit occupy independent fields; upload arrows are red and download arrows
blue, matching macOS. Side taskbars retain separate value/unit lines. Shared text-run
geometry drives both opaque GDI drawing and transparent DirectWrite drawing. Taskbar rates
use one decimal below 100, omit trailing `.0`, and round larger rates to integers.
The 28-DIP numeric field also fits rounded 1000 without clipping or changing units;
shared tray text and tooltips keep their existing precision.

### Overview history and disk activity

Resource readings use schema version 3; frontend adapters reject mismatched versions. Memory history records occupancy from the existing three-second sampler. CPU and memory use a fixed 0–100% scale. Network and disk activity share a symmetric scale: upload/write above zero, download/read below it. Gaps remain blank. The frontend buffers one sampling interval plus 250 ms before revealing each completed segment from the right; numeric readings remain live. Core retains up to 80 seconds / 96 samples so a reopened chart can reconstruct the buffered minute and offscreen endpoints. The frontend retains two additional intervals at the left edge. During a brief delivery delay, the playhead waits for completed data and catches up at no more than 1.1× speed; genuinely expired data still scrolls out. Pausing demand preserves existing readings and history with their original timestamps, while source changes clear the corresponding history. Rate scales hold their range for 30 seconds before a substantial reduction, and range changes ease over 600 ms using a shared SVG group. Horizontal scrolling uses that group’s native transform instead of a composited CSS bitmap, preserving vector strokes at fractional positions. Reduced-motion mode applies scale changes immediately and disables continuous scrolling; hidden or fully expired charts stop their frame loop.

Disk capacity belongs to the selected volume. Disk activity is explicitly system-wide block-device I/O, sampled independently every two seconds while resident mode is enabled. macOS reads IOKit block-storage driver counters once per driver; Windows reads localized-independent PDH PhysicalDisk counters once per instance, excluding `_Total`. These counters describe block storage, not per-volume or application file traffic. Missing counters show unavailable rather than zero. Device-set changes, counter rollback, and sleep invalidate the monotonic rate baseline.

`resident_disk_io_state` records capability/freshness transitions and scope; failures log typed codes without device identities. Sampling durations join the bounded periodic summary. Closing the panel or entering Memory preserves disk activity demand and its baseline. Disabling resident mode stops the worker requests and discards the baseline. Healthy idle intervals remain zero-valued samples; unavailable intervals remain gaps.

CPU baseline-only observations retain the last reading until its normal five-second expiry instead of clearing the chart. Recovery can bring forward at most two serialized queries by 250 ms, then returns to normal cadence until a valid interval is available. `resident_cpu_baseline` identifies first observations, invalid intervals, counter resets, and stalled counters; `resident_cpu_recovered` records bounded recovery attempts without raw counters or machine identifiers.

## Memory release preferences

On macOS and Windows, `memory-release.json` stores schema version 1 separately from display
preferences. Unknown versions or malformed data disable automatic release and
block writes rather than replacing the user's settings. Revision checks prevent
the ranking shortcut and the settings window from overwriting each other's edits.

Manual and scheduled release share one serialized action. On Windows, both honor
executable-path exclusions. An exclusion covers all processes with the same executable image,
including later restarts; it does not cover unrelated helper executables or stop
Windows itself from reclaiming memory. Native identity is read from the opened
process handle before trimming, and unreadable identities are skipped whenever
exclusions are active. Automatic release can also skip the foreground image.

New preferences default to a fifteen-minute interval with automatic release off.
Accepted intervals are 3, 5, 15, 30, 60, and 120 minutes; existing saved choices
remain unchanged.

The desktop worker checks deadlines every five seconds, samples memory only when
a check is due, and never accumulates missed checks. Settings changes, manual
actions, sleep gaps, and clock discontinuities reset the interval. No service,
elevation, or operating-system scheduled task is installed. macOS uses the same
scheduler and settings entry point with its native volatile-memory operation;
foreground-process skipping remains Windows-only because the macOS operation is
global. macOS exposes only the timer and threshold; preferences containing an
exclusion list or foreground skipping are rejected, never silently ignored.
The separate settings window remains open when the monitor loses focus.

### Usage colors and menu-bar density

CPU, memory, and disk capacity percentages share configurable warning/critical
thresholds (70/90 by default). Coloring is enabled for new installations and
legacy versions without this preference; an explicit saved opt-out is retained.
Only values change color; labels retain the theme
foreground. Tones enter higher bands at the threshold and clear three percentage
points below it. Unavailable readings and disabled coloring reset the tone.
Changes to color rules reset hysteresis so new thresholds apply immediately.
Changing the sampled disk resets its tone without affecting other metrics.

Version 8 adds these color preferences and an independent macOS compact setting.
Legacy selections and custom order are retained. The former default sequence
and new installations use CPU, memory, disk, then network. macOS compact mode preserves labels, percentages and direction
arrows, shortens network units, and reduces fixed field widths and spacing. Both
densities reserve enough width for their largest numeric values at the native font.

### Background update notice

`services::app_updates` owns both scheduled discovery and explicit checks,
independently of resident sampling and main-window creation. It checks after a
3-second startup delay and then every six hours; failures retry after 1, 5, 30,
and at most 60 minutes while retaining the last successful result. Wall-clock
deadlines include sleep. A 30-second poll or window-focus read catches overdue
checks. Concurrent callers share the active result, including failures; a manual
check after completion explicitly refreshes. Each network operation has a total
timeout and uses the same native locale, distribution, optional existing install
identity, and OS version headers. Discovery does not depend on browser timezone
metadata or create another installation identity.

The versioned, revisioned `app-update-notice` snapshot is readable through
`get_app_update_notice`. Consumers subscribe before reading and reject older
revisions. Every successful check advances the revision, including no-update
results, so both windows clear stale notices. Failed checks retain the prior
result. The resource panel shows one text action in either tab; no tray-menu item
or OS notification is created.

The existing About navigation reads the native cache through
`acquire_app_update(refresh=false)`. Only the main WebView may acquire a cloned
plugin Update in its resource table. The cached native Update remains independent
of that window's resource lifetime. `refresh=true` explicitly requests a check;
opening the update window after discovery does not make another network request.
The frontend continues using the plugin's signed download and install methods,
including portable download URL validation and existing user-controlled actions.
Downloaded resources are not replaced by background notices.

Logs record request source and ID, shared requests, availability, version
changes, elapsed time, retry delay, and retained-result state. Resource acquisition
is distinct from network discovery. Polls and unchanged notice reads do not log.
State remains process-local and is rediscovered after restart, without a new
persisted settings schema or forced WebView creation.
