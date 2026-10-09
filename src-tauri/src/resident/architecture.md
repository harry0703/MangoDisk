# Resident system status

The resident adapter combines independent Core readings into native menu-bar/tray
entries and one reusable detail panel. It does not own filesystem cleanup,
application termination, or memory-reclamation algorithms.

## Ownership

| Area | Responsibility |
| --- | --- |
| Platform `system_resources` | Native counters, interface identity, local volume identity and capacity |
| Core `system_resources` | CPU/network/disk I/O deltas, selection policy, freshness, bounded trends and memory use cases |
| `sampling_workers` / `sampling_schedule` | Bounded overview workers, demand, deadlines and generation checks |
| `process_cpu_sampling` | Independent application CPU cadence and coalesced immediate refresh |
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

Resident preferences use schema version 11; resource readings use version 16.
Versions 1–8 preferences preserve every saved selection and order, appending GPU
display disabled. New installations also leave GPU display unselected. Resource
version 9 adds the GPU catalogue. Version 9 preferences migrate GPU selection to automatic; version 10 persists fixed selection. Frontends reject mismatched envelopes.
Version 1 preferences retain background and memory-display choices. Version 2
preferences retain all selections and default to the original Windows tray mode.
Version 3 retains that mode and defaults the new position preference to right.
Version 4 retains all choices and enables the new taskbar background option.
Version 5 retains its saved left/right preference; only new installations default
to automatic placement. Version 6 retains automatic/manual choices and defaults
the new compact mode to off. Compact mode reduces horizontal cells from 38/81
to 34/55 DIP (percentage/network), with abbreviated network units
(B/K/M/G/T) and unchanged numeric precision. Taskbar labels and values use 13 DIP Segoe UI. The shared size converts to physical pixels with nearest-pixel
rounding at the monitor DPI; paint and hit testing share those bounds.
Unknown persisted
versions are rejected for writes. Memory snapshots use version 4; release results retain their separate version 1 contract.

CPU overview samples every 2 seconds on both platforms, whether visible or hidden. Memory samples every
3 seconds, network every 1 and disk every 30.
Their freshness limits are respectively 5, 10, 5 and 90 seconds. CPU, memory, network and disk remain active while resident display is enabled,
regardless of native selections or panel visibility. GPU acquires its reader only
while selected for native display or while the panel is open; unselected hidden
GPU monitoring has no native sampling cost. Disabling resident mode stops periodic collection;
explicit device-catalogue requests may still read network and disk metadata.
Each overview worker creates its native reader only when demanded and drops it on
the same worker after demand stops and any in-flight query finishes. This releases
memory-process metadata, Windows network-change subscriptions, and CPU counters.
Re-enabling reacquires readers; hiding the panel preserves demanded readers and cached
rankings, releasing GPU resources when its native display is unselected. Resource acquisition/release logs record transitions rather than samples.
The panel has an overview of all five base readings on macOS/Windows (four on Linux) and separate CPU and memory pages.
Memory process details are requested only by its selected page or startup icon
warming. CPU application counters stay warm while resident mode is enabled, every 4 seconds in the background and every 2 seconds on the CPU page. Opening or explicitly refreshing that page requests a sample immediately while reusing any in-flight query and displaying cached rows. Baseline-only results receive at most two 250 ms retries. Reopening preserves the last
selected tab within the application session; a new process defaults to CPU.
A different metric entry can navigate an already open panel.
Linux AppIndicator does not publish tray click events, so its native menu starts
with a localized System status action that opens the same reusable detail panel.
Linux also lacks tray geometry; the panel uses the existing placement fallback
at the upper-right of the primary monitor's work area.
Linux panel positioning and showing run on the GTK main thread: monitor work-area
queries access GDK directly and can corrupt an X11 connection when called by a
menu or command worker alongside GTK events. Showing failures clear open intent
and record a diagnostic instead of leaving detailed sampling active.
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

Application rankings are immutable shared snapshots in Rust. Cloning a reading
shares their allocations without changing the resource-reading JSON contract. Unchanged
coordinator ticks only check freshness; they do not rebuild histories or lists.
Query logs separate each sensor's bounded timing samples and discarded generations.

A slow native query stays in flight until it returns. Changing demand invalidates
its generation without spawning replacement threads. Native panel visibility explicitly stops chart animation even when WebView2 leaves
`document.hidden` false. Hidden WebViews receive no periodic reading events and reload the published cache when opened.
A prewarmed panel receives one initial CPU-history seed; hidden rows do not request icons.
Native show/hide events control rendering independently of focus; Windows can show
a taskbar popup before granting focus. The listener seeds actual window visibility
and ignores a late seed after a newer event, so cached rows appear on that first frame.
Publications from one completion burst are coalesced over 50 ms and limited to once
per second. A separate presentation worker uses one bounded wake slot and reads
one immutable snapshot for both native display and visible-window events. It stores
that same revision for IPC reads, preventing reopening from getting ahead of the
native display. Preference and explicit open/refresh requests may publish immediately; a slow native UI operation
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
Windows reads interface state and byte counters every tick. Default-route preferences
are cached for at most thirty seconds and invalidated by native route/interface
notifications. Callback state is process-lifetime and never touches UI or reader
pointers; worker-owned subscriptions cancel outside callbacks. Registration failure
retains per-tick metadata queries. Route-query failure clears the preference and
retries next tick, with diagnostics only on failure/recovery transitions.
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
receives native UI regression checks. On macOS, no menu remains attached to the
status item: a secondary press creates a menu using the current locale, presents
it on the main thread, and removes it even if presentation fails. This follows
[tray-icon #365](https://github.com/tauri-apps/tray-icon/pull/365): macOS 27 swallows
primary clicks while `NSStatusItem.menu` is non-null. Apple documents this
[menu/action precedence](https://developer.apple.com/documentation/appkit/nsstatusitem/menu).
The adapter leaves Tauri's event-listener callback before entering native menu
tracking and holds no display-state or tray-icon borrow across that nested event
loop. Primary presses toggle the panel once; releases do not toggle it again.
The complete tray-handle lifetime stays on the main thread, including lookup,
cloning, and destruction. Tauri's `Send` wrapper contains tray-icon's non-atomic
`Rc`; dispatching only its native methods does not make background ownership
safe. Workers pass application handles, entry IDs, and prepared display data,
without cloning detailed process lists for native rendering. The existing
bounded presentation queue preserves snapshot order across preference commits;
display locks are acquired only after main-thread dispatch. Diagnostic handle
counts are queued without blocking sampling. A scoped tracking guard rejects
secondary-menu reentry and resets after dismissal, failure, or cancellation.
Blur during a primary press over the entry is deferred to the toggle; keyboard
switching and outside clicks still dismiss it. Preference and locale refreshes
never reattach a macOS menu. The panel explicitly uses AppKit's
[stationary collection behavior](https://developer.apple.com/documentation/appkit/nswindow/collectionbehavior-swift.struct/stationary)
so Show Desktop cannot sweep a newly opened panel offscreen. This replaces only
the mutually exclusive Mission Control flags and preserves workspace/fullscreen
policy. Check opening after clicking the wallpaper to reveal the desktop,
primary open/close, secondary menu dismissal
and actions, locale refresh, and entry recreation in a packaged app when changing
this path; macOS 15 regression results do not establish macOS 27 compatibility.
Windows uses at most six retained tray handles and
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

Stable taskbar layout and reservation exchanges run at most once per second;
100 ms visibility/collision checks and explicit reading/shell changes remain
immediate. Hidden or failed layouts retry without this throttle.

Windows can show the same readings in either retained tray icons or one native
layered Win32 child of Explorer's taskbar: the Windows 10 rebar, or the
Windows 11 taskbar itself. Only the native monitor surface is parented; main
and detail WebViews remain independent. On Windows 10, an unelevated companion
process reserves space by moving/shrinking `MSTaskListWClass` inside
`MSTaskSwWClass`, leaving the outer container unchanged;
Explorer still owns application-button layout and overflow. Closing the private
stdin pipe releases this lease on disable, normal exit, or abrupt GUI-process exit.
A session-local mutex serializes companion lifetimes, including restoration, so
rapid disable/re-enable cannot overlap allocations. Restoration only touches the
same shell process/control and our last applied axis; a newer Explorer layout wins.
Cross-axis DPI changes preserve the current thickness while restoring our axis.
TrafficMonitor reserves the outer `MSTaskSwWClass` container. MangoDisk only
resizes the inner task list, so TrafficMonitor never sees MangoDisk's contraction
as a new outer baseline. Manual left placement remains before application buttons
rather than following their growing UIA bounds. Both the GUI and companion check
for foreign rebar surfaces; unknown integrations use fresh UIA gaps instead of
assuming which window they mutate. The companion also rejects a reserved slot
that overlaps a peer's native bounds, including owner-drawn controls.
TrafficMonitor's named top-level fallbacks are enumerated across displays:
they still resize task buttons when embedding fails. Only fallbacks on the host's
monitor participate; offscreen and secondary-display windows do not affect it.
Popup children use their actual ancestor parent because `GetParent` can return
the dialog owner. Hidden peers remain detected during initialization, but do not
contribute occupied pixels. An unavailable inner task list uses read-only gaps.
Windows 11 performs the same native peer discovery under `Shell_TrayWnd`.
Any foreign panel, including TrafficMonitor, disables XAML reservation because
there is no independent inner/outer reservation boundary. Both the GUI and the
companion check for peers; shared hosts use fresh UIA gaps combined with native
peer bounds. This covers owner-drawn panels that UIA does not expose. Peer removal
allows reservation again; an undersized manual-edge gap falls back to the tray.
Coexistence acceptance on both Windows versions must require two visible native
panels and assert their rectangles do not intersect for left, right and automatic
placement, including application-button changes and shell recreation. Checking
only MangoDisk's visibility and screen half does not establish coexistence.
After releasing a companion, placement waits asynchronously for its exit and a
UIA snapshot whose collection started after that exit, including disable/re-enable.
A release stalled beyond three seconds exposes tray fallback while continuing to
wait; it never starts a competing lease or reuses a pre-release snapshot.
Restoration preserves a newer Explorer layout and never expands the outer
container over TrafficMonitor. A genuinely full host still falls back to the tray.
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
unshared Windows 11 snapshot performs it, including after environment detection recovers.
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
the previous white-on-black GDI intensity-to-coverage conversion. GDI-compatible
grid fitting and matching measuring mode keep small glyph advances on physical
pixels. System gamma and grayscale contrast are preserved; flat pixel geometry
and zero ClearType level avoid colored fringes on an unknown backdrop. Native
field-fit tests use GDI-compatible metrics as well. Regular Segoe UI
uses the same 13 DIP label and value size as the opaque path; GDI uses
normal weight 400 and both paths retain the same DPI-converted sizes and cell rectangles. Factories, target and DPI-specific text
format stay on the native window thread; a failed frame discards them for recovery. Background pixels use alpha 1/255
rather than zero so clicks still reach the entire cell; hover raises that alpha
to 28/255. ClearType remains enabled only for the opaque, known-background path.
Mode transitions change the native window style and invalidate its surface.
The transparent surface is presented before publishing its bounds for interaction,
because a newly layered window has no hit-testable pixels. Allocation/presentation failure
hides the surface and activates the existing tray fallback; diagnostics record
the failing stage and recovery, not every frame.

Percentage labels and values use equal-height rows inside the normal 36 DIP
cell, matching the two network rows at the same font size.

Windows taskbar network columns keep a fixed 81 DIP width, or 55 DIP in compact mode. The arrow, right-aligned
value and unit occupy independent fields; upload arrows are red and download arrows
blue, matching macOS. Side taskbars retain separate value/unit lines. Shared text-run
geometry drives both opaque GDI drawing and transparent DirectWrite drawing. Taskbar rates
use one decimal below 100, omit trailing `.0`, and round larger rates to integers.
The 30-DIP numeric field also fits rounded 1000 without clipping or changing units;
shared tray text and tooltips keep their existing precision.

### Overview history and disk activity

Resource readings use schema version 16 and nested `SystemResourceSnapshot` payloads use version 4; frontend adapters reject mismatched versions. Memory history records occupancy from the existing three-second sampler. CPU, GPU and memory use a fixed 0–100% scale. Network and disk activity share a symmetric scale: upload/write above zero, download/read below it. Gaps remain blank. The frontend buffers one sampling interval plus 250 ms before revealing each completed segment from the right; numeric readings remain live. Core retains up to 80 seconds / 96 samples so a reopened chart can reconstruct the buffered minute and offscreen endpoints. The frontend retains two additional intervals at the left edge. During a brief delivery delay, the playhead waits for completed data and catches up at no more than 1.1× speed; genuinely expired data still scrolls out. Pausing demand preserves existing readings and history with their original timestamps, while source changes clear the corresponding history. Rate scales hold their range for 30 seconds before a substantial reduction, and range changes ease over 600 ms using a shared SVG group. Continuous SVG updates are capped at 30 frames per second regardless of display refresh rate, without changing sample cadence or live numeric updates. Horizontal scrolling uses that group’s native transform instead of a composited CSS bitmap, preserving vector strokes at fractional positions. Reduced-motion mode applies scale changes immediately and disables continuous scrolling; hidden or fully expired charts stop their frame loop.

Memory pressure is independent of occupancy. macOS reads `kern.memorystatus_vm_pressure_level` in the existing three-second memory sample and decodes dispatch flags 1/2/4 as normal/warning/critical. Unknown flags, unexpected lengths and read errors publish unavailable; Windows and Linux publish unsupported. The panel hides unsupported pressure and marks non-ready samples as not updated. Diagnostics record the first observation, transitions, failures and recovery rather than every sample.

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

### GPU activity

GPU activity samples every two seconds with a five-second freshness limit. Its
independent worker acquires native resources lazily and releases them after
GPU demand stops and in-flight work completes. Windows baseline recovery uses
at most two 250 ms retries for a quick first value. Hidden panels stop chart animation;
GPU history is retained with the same bounded window as CPU. The overview places
CPU and GPU side by side; GPU entries share the existing panel navigation. Native GPU percentages
reuse the existing compact layout, rounded usage colors and unavailable dash.

Core selects the busiest readable GPU by default, with stable identity-based ties.
A fixed selection never falls back to another adapter; a missing selection is
reported as disconnected. Selection changes clear history and re-prime native
intervals. Automatic history represents the system-wide maximum even when the
leading GPU changes. Settings list hardware adapters even when their counters
are unavailable. Catalogue-only demand refreshes metadata without acquiring the
PDH query; opening configuration with GPU display disabled does not start
utilization sampling. Windows identities use PCI vendor/device and bus/device/function
plus physical-adapter index; macOS uses the IOService registry path. Neither
persists an ephemeral LUID or registry entry ID. Session adapters with invalid
PCI addresses are excluded instead of sharing a persisted identity. The card and native tooltip
identify the sampled device. Invalid observations never become idle zeros.

Windows keeps one language-neutral `\\GPU Engine(*)\\Utilization Percentage` PDH
query and reuses a bounded, aligned native buffer. Instances are grouped by
adapter LUID, physical GPU and engine; process activity is summed per engine.
DXGI and public D3DKMT adapter queries cache hardware identities and standard
WDDM engine types for 30 seconds. The aggregate takes the busiest standard
engine, excluding driver-defined `DXGK_ENGINE_TYPE_OTHER` nodes. This matches
the tested Windows 10 summary where a driver-defined Graphics_1 curve can be
much higher than the overall percentage. It is a summary compatibility policy,
not an assertion that OTHER engines are idle: compute workloads exposed only
through custom engines are outside this reading. Do not infer engine roles from
names or use the current four visible charts to classify engines. Software
adapters are excluded. Missing metadata fails closed instead of reverting to an
incomparable all-engine maximum. Driver/build differences require hardware
validation; exact instantaneous equality is not guaranteed across sample windows.

On macOS, CPU and memory enumerate `proc_listallpids` and read fresh
`proc_pid_rusage(RUSAGE_INFO_V2)` counters through one minimal native reader. They
share executable metadata keyed by PID, creation time, and executable-image UUID.
Exited identities are evicted; failed identity queries are not cached by PID alone.
Readers acquire the cache lazily, and the registry holds only a weak reference so
resetting the last active reader releases all cached metadata.
Path lookups validate identity again before attaching measurements. Transient path
failures retry after two seconds; successful paths refresh after thirty seconds.
LaunchServices display names remain freshly queried, including per-application
WebKit names. Enumeration failures retain historical readings through the existing
failure path; they never publish an empty success or substitute RSS for footprint.

A bounded, isolated `/bin/ps` query supplements processes owned by
other users, including WindowServer and virtual machines. Only unreadable PIDs
are queried; readable processes retain the cheaper native path. It reads cumulative CPU
**time**, never the lifetime-average `%cpu`, environment, or command-line arguments.
The command has a 500 ms deadline and 1 MiB output limit. Its creation identity has
one-second resolution and counters have 10 ms resolution; the native path retains
subsecond identity and counters. The kernel task remains explicitly unavailable,
as it cannot be read through this path. Unreadable counters are omitted, never idle.
Registered macOS applications supply their display names, including individual VM names.

Windows reads cumulative CPU times in one `NtQuerySystemInformation` process snapshot,
so system/service counters do not require opening each process handle or elevation.
PID 0 is idle capacity and is excluded from the work ranking. Image paths remain
optional: limited-access image queries validate the exact creation identity on the
same handle, then cache both success and denial until that identity exits. Reused
PIDs receive new queries. CPU and memory readers share a PID/creation-keyed image
cache; numeric snapshots remain fresh and independent. The registry owns only a
weak reference, so dropping the last reader releases cached metadata. Warm snapshots
reuse image results rather than repeating process-handle path queries.
The native entry point is resolved dynamically. The Windows 10/11 CPU prefix layout
is checked against this process's public `GetProcessTimes` before/after counters on
every sample. Entry offsets, UTF-16 pointers and lengths are checked within the
returned buffer; retries are capped at four and retained buffer storage at 16 MiB.
An unavailable or incompatible native snapshot logs a bounded fallback transition
and uses limited-access `GetProcessTimes`; that fallback can omit system processes,
which remain available in diagnostic coverage counts rather than a permanent user-facing count. No new privileges or helper are needed.

Core computes monotonic deltas keyed by PID and creation identity. macOS CPU rows remain individual processes. Windows CPU rows and memory rows use the same application identity and aggregation function: known executable paths group together, unknown paths and `svchost.exe` service hosts remain separate. Service hosts never expose application quit. CPU groups sum all valid members before ranking, retain up to 50 member PID/start-time/usage details, and never expose an app-wide quit action. macOS process percentages
use 100% per logical core and can exceed 100%; Windows uses 100% across all logical
cores, matching each platform's customary system-tool scale. The CPU overview and
trend still use the whole-machine 0–100% scale. Typed `usageScale` communicates the
list's units. Relative row bars compare with the largest row, not the overview.
New processes, counter rollback, scale/topology changes, and intervals outside
200–5000 ms require a new baseline. Maps are replaced every sample and rankings
publish the 50 highest-usage applications after aggregation. The delta baseline still retains every readable process so a newly busy process can enter the ranking immediately. CPU retains coverage counts for diagnostics; only actionable refresh failures appear in the panel. Process IDs are shown in details.
Compare the same PID in Task Manager's Details view: its Processes view groups
application instances, and independently sampled windows need not show identical
instantaneous values.

CPU and memory share icon identity, disclosure, and file-manager navigation. CPU is
view-only: normal app-wide quit and memory exclusions remain in the memory list.
Resource version 9 replaces the ephemeral IPC protocol; readers reject other
versions. Resident preferences migrate independently to version 11.

Linux process rows request path-specific icons rather than sharing an extensionless
file-type icon. The GUI enables the platform's `linux-desktop-icons` feature;
standalone Core/CLI builds do not acquire GTK dependencies. The adapter indexes registered desktop entries through
GIO, matches canonical executable paths, and resolves their artwork through the
current GTK icon theme. Conflicting entries, script/sandbox launchers, and processes
without registered artwork retain a neutral process glyph. The running MangoDisk
executable uses its bundled artwork after registration of its exact path.
Shared-launcher checks apply to both the desktop command and its canonical
executable, so a differently named symlink cannot assign a script's artwork to
unrelated interpreter processes. An isolated desktop-registry regression test
covers this alias case without modifying the user's registered applications.
Metadata and scaled PNG decoding run on icon workers, independently of resource
sampling. Only theme lookup uses the GTK main loop, with a bounded wait and retry
backoff. Desktop indexing and positive/negative path resolution are bounded and
refresh after sixty seconds; theme changes invalidate native resolved paths. Existing
frontend batching and native PNG caching avoid repeated decoding on panel reopen.
Run `cargo test -p mangodisk-platform --features linux-desktop-icons --test linux_file_icons -- --ignored --nocapture`
with the target desktop's `DISPLAY` for cold/repeated timings and real application
coverage. Set `MANGODISK_EXPECT_NATIVE_ICONS=1` on the Ubuntu GNOME test desktop to
assert registered application artwork and daemon fallback. Keep raw output under
the ignored `.local/` directory.

For repeatable sensor-cost measurements, build `resource_sampling_probe` in
`mangodisk-platform` with `--release`, then run `overview`, `memory`, `cpu`,
`cpu-background`, or `cpu-memory` with a duration in seconds (60 by default).
`gpu` isolates two-second GPU sampling, and `overview-gpu` adds it to the
CPU/memory baseline. `gpu_usage_probe <seconds> --lifecycle` prints native
GPU observations and exercises baseline reset and reader reacquisition.
`gpu_usage_probe <seconds> --catalogue` measures metadata-only discovery without
acquiring utilization counters.
`cpu-memory` measures both detail sources in one process with shared metadata.
The optional third argument selects a two- or four-second process CPU interval.
Each run warms up for eight seconds and emits
aggregate single-core CPU percentage, helper CPU time, RSS bounds, and sample latency
percentiles. macOS measurements include the system-tool fallback separately and in
the combined CPU total; helper RSS is transient and excluded from probe RSS bounds.
Use the same build, workload, and duration for comparisons; the probe includes its
own observation overhead and excludes UI, icon I/O, and application aggregation.
Keep raw machine output in the ignored `.local/` directory. Measure the packaged
application separately with its windows hidden and each detail panel open.

Memory process rankings have a separate `memoryProcesses` reading and timestamp in resource protocol version 8. Overview-only samples preserve these rows without extending their freshness. Real empty detail samples clear the list; failed/stale detail results retain previous rows with a notice. The source memory snapshot is version 3; resident publication moves its optional detail payload into the dedicated cache to avoid duplicate IPC data. Frontends reject earlier resource versions.

Windows image lookups retain typed available/denied/exited/unavailable outcomes. Only transient unavailable results retry, at 30-second intervals and at most three total attempts per PID/creation identity. Successful paths and denials do not poll again; exited identities are evicted. Path failures never remove readable CPU counters. The UI explains access restrictions without requesting elevation.

CPU and memory panels share persisted name/usage sorting (frontend settings key
`resourceSortPreferences`, schema 1, invalid/unknown versions reset to defaults).
Both rankings publish at most 50 highest-usage entries and use virtual rendering; name sorting reorders this bounded set. The sortable column labels occupy one header row. Row keys remain stable across value-only updates to avoid invalidating virtual measurements; startup icon warming stays capped at
30 entries. Expanding details freezes the displayed identities and member order
while measurements continue. Missing samples display unavailable values and disable
actions rather than claiming the process exited. New identities wait until the
list unlocks. Explicit column sorting applies immediately; closing the panel clears
the interaction lock and resets the scroll position. Native visibility disables virtual
observers while hidden; reopening rebuilds the visible range from offset zero even
when the WebView suppresses hidden scroll events. CPU and memory retain separate sorting preferences.

Visible main-window publications omit process rankings, which are consumed only by
the detail panel. The panel retains value identity for matching sample timestamps
so network-only ticks do not rebuild the CPU or memory lists; status changes and
explicit clears still apply. Native entries and overview values share the same
published revision as before.

## Memory measurement semantics

Memory rankings publish `usageKind` and nullable `usedBytes`, rather than calling every
platform metric resident memory. macOS uses the process footprint ledger (including
compressed attribution); Windows uses private working set, excluding shared pages and
commit. Linux retains RSS. Groups reuse the CPU identity fold and sum all readable
members before ranking and publish `readableProcessCount` with each application. A
partially readable group shows its measured sum with a partial-data badge and readable/total
coverage in expanded details; a completely unreadable group remains unknown. Shared macOS
WebKit XPC images retain separate PID identities and OS display names, because their
processes can belong to unrelated host applications. Windows service hosts remain
separate by PID; other images group by executable path, so runtime hosts may still
group differently from Task Manager. Use matching PID sets for numerical comparisons; group totals do not sum to
system used memory.

macOS overview subtracts native free and file-backed page counts from installed RAM,
including inactive anonymous pages and the physical compressor. The free-page field
excludes speculative pages and is not a pressure classification. Windows overview
retains physical total minus native available memory. No process totals are used to
construct either system overview.

Unreadable GUI applications remain visible without a numeric rank, even when they
would otherwise be lost behind 50 known rows. Their values show a dash and their
expanded detail explains the permission limitation. Other unknown processes occupy
remaining slots. Native failures never fall back to RSS or zero under a footprint or
private-working-set label. macOS system/root processes can remain unreadable at normal
privilege: Activity Monitor has system access which this app does not request. High-cost
`top`/memory-map traversal and privilege elevation are not part of periodic monitoring.

Source snapshot schema 3 and resident schema 8 replace the former resident-byte field;
frontends reject earlier envelopes. `application_memory_probe` captures native PID
measurements and grouped Core publication from the same sample for reproducible
system-tool comparisons without collecting command lines or environments.

## GPU detail observations

The version 11 reading contract adds device-bound GPU histories and optional
telemetry to `gpuDetails`. `gpuDetailAdapterId` identifies the source of
`gpuDetailHistory`, `gpuRendererHistory` and `gpuTilerHistory`.
`gpu` retains a summary with no embedded details. An incompatible frontend
rejects the snapshot rather than presenting partial measurements. Preferences
use version 11; the existing stable device selection is shared by both views.

Only a visible GPU tab requests detail observations. Windows temporarily adds
adapter-wide Dedicated Usage and Shared Usage counters to its persistent PDH
query, removes them when detail demand ends, and maps them by LUID and physical
adapter index. DXGI reports dedicated capacity for a single physical adapter;
linked-adapter capacity and shared capacity remain unavailable. Driver-defined
engines are visible separately and do not change the standard-engine summary.
Memory failures have independent backoff and cannot disable activity readings.
Windows probes temperature through public WDDM `KMTQAITYPE_ADAPTERPERFDATA`
queries during detail demand only. The metadata cache retains its adapter handle;
physical adapter indices keep linked GPUs separate. Temperature is converted from
deci-Celsius, and unsupported or invalid readings are omitted with 30-second backoff.
The panel displays dedicated usage only; shared usage is withheld because its
equivalence to Task Manager has not been validated across drivers.
macOS probes Renderer and Tiler percentages and optional temperature, clocks and
fan percentage from the same IOAccelerator snapshot, including background samples.
This requires no additional native property queries. Core count is discovered
from `gpu-core-count` with the adapter, outside the sampling hot path.
Apple AGX adapters report unified memory architecture; driver allocation fields
are not treated as physical VRAM capacity. Unsupported optional facts are omitted.

Core records every real summary sample in the selected-device trend, including
background samples when details pause. This prevents artificial holes caused by
tab changes without bridging real outages or mixing devices. Renderer and Tiler
histories include every actual engine observation, so macOS tab changes preserve
their continuity. Actual sampling outages remain blank. All three selected-device
histories clear when the adapter changes. The overview retains its history of the
automatic maximum. Both views
use bounded histories and original sample timestamps; failures and expiration do
not become idle values. Native GPU entries and the GPU overview card navigate to
the GPU tab. The detail selector persists through the same optimistic settings
store, including rollback and retained disconnected selections.

Reopening the panel displays the retained sample while native detail collection
resumes. Stale or failed samples keep their original status and are marked as
cached. A warm summary can fill the card before the first detail sample arrives.
Details and histories are displayed only for their matching summary adapter; preference
refreshes run in the background, and a changed fixed selection hides mismatched
measurements immediately. Disconnected or unsupported GPUs do not reuse values.

The panel omits unified-memory placeholder sections and explanatory memory text.
Optional driver telemetry is shown only when supported; absent fields never
become plausible zero measurements. Minute average and peak use valid actual
selected-device observations within the last 60 seconds, excluding missing or
future samples. Neither ANE percentages estimated from nominal power nor display
swap rates are presented as GPU compute utilization or application FPS.

GPU engine presentation uses a stable functional order: 3D, Copy, Video Decode,
Video Encode, Video Processing, then other standard nodes. Only observed types
are shown; zero utilization does not hide an observed engine. Custom engines
use natural native-name order with numeric chunks and stable identity tie-breaks,
independent of locale and live utilization. macOS retains Renderer before Tiler.

GPU discovery logs hardware identities, names and available metadata, plus Windows
node ordinals, native engine types and inclusion in the peak-standard-engine summary.
Topology is logged on discovery/change only, with at most 64 node records per
adapter and an explicit omitted count. Native names are escaped and bounded.
Observation diagnostics record the actual utilization source, optional capabilities,
memory availability and missing/invalid data stages, without per-sample values.
Capability records have a 30-second minimum interval per device; transient changes
retain a count and their latest unavailable stage until the next summary. Device
removal and reader release flush pending transitions before discarding their state;
these lifecycle summaries can precede the periodic deadline. Already emitted changes
are never repeated on release. macOS instantaneous readings need no interval reset:
tab/demand changes preserve the 30-second native catalogue cache until discovery
refreshes it or the owning worker releases the reader. A missing
capability snapshot means details were not collected, not that the hardware lacks
them. The engine mask uses Graphics=1, Copy=2, Decode=4, Encode=8, Processing=16,
Other=32, Renderer=64 and Tiler=128. Diagnostic entries are pruned as devices leave.
Windows temperature query failures retain their native stage/code at Info level,
with unsupported retries and diagnostic state preserved across metadata refreshes.
`gpu_usage_probe <seconds> --details --diagnostics` enables the same diagnostics
on stderr without mixing them into its JSON observation protocol. Combining
`--lifecycle --details --diagnostics` also exercises detail demand transitions
before a reader release, without injecting hardware failures.

### CPU temperature observations

CPU temperature is independent of utilization and frequency. A visible overview
or CPU detail panel requests a reading at most once every four seconds on the
existing CPU worker. Hidden panels issue temperature queries only when macOS
menu-bar temperature display is enabled; changing
demand or resuming discards native sensor handles and starts a fresh observation.
Failed queries retry after 30 seconds. Values expire after ten seconds, and the
frontend checks sample age before displaying a number. The Celsius trend uses
a separate fixed 0–150 degree scale and bounded Core history; this range is not
a health threshold. Failures and changed sensor sets clear temperature history.

macOS reads only mapped CPU SMC keys, caches their metadata, and requires all
selected sensors to be valid before publishing a reading. Apple Silicon
M1–M5 mappings distinguish CPU keys from GPU, PMU, battery and proximity sensors;
unknown chips are unsupported. Apple Silicon publishes the selected cores' average.
Intel prefers the CPU package key TCAD. When absent, machines with one to eight
physical cores can use the highest TC1C–TC8C core temperature, bounded by
`hw.physicalcpu_max` and requiring the complete selected set. Unused slots are
excluded because they can expose plausible-looking sentinel values. A core maximum
is explicitly labeled as such, never as a package reading. These private
interfaces require per-chip and OS validation; available readings do not establish
support for every Mac. Linux recognizes coretemp package/core sensors and k10temp
Tdie, excluding Tctl offsets and anonymous thermal zones. Windows currently has
no verified CPU temperature provider: public ACPI zone readings are intentionally
unsupported rather than mislabeled. No helper, driver or privilege elevation is
installed. Unsupported overview values are hidden, while CPU details explain the
limitation; failed or stale values display an em dash rather than a cached number.

## macOS menu-bar CPU temperature

Preference schema 11 adds `cpuTemperature` to the ordered `metrics` display
items. It is disabled by default and appended to migrated preferences without
changing existing selections or relative order. macOS exposes the same checkbox
and drag/keyboard reordering as other items, independently of CPU utilization.
Display item IDs are separate from the five resource worker IDs. The temperature
tooltip identifies the sensor aggregation and count; temperature does not use
utilization warning colors.
The column keeps a fixed width and displays `—` for unavailable or expired data.

Enabling the column keeps the existing CPU worker's four-second temperature
sampling active while the detail panel is closed. Disabling it restores
panel-only demand; disabling resident mode releases the reader. No extra worker
or process is created. Windows and Linux hide and ignore this macOS display item.
Windows CPU temperature remains unsupported; no third-party driver is installed.
