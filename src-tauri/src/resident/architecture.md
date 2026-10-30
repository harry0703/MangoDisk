# Resident system status

The resident adapter combines independent Core readings into native menu-bar/tray
entries, a cross-platform floating monitor, and one reusable detail panel. It does not own filesystem cleanup,
application termination, or memory-reclamation algorithms.

## Ownership

| Area | Responsibility |
| --- | --- |
| Platform `system_resources` | Native counters, interface identity, local volume identity and capacity |
| Core `system_resources` | CPU/network/disk I/O deltas, selection policy, freshness, bounded trends and memory use cases |
| `sampling_policy` / `sampling_dispatch` / `sampling_schedule` | Typed task demand, bounded dispatch, deadlines, generations and source recovery |
| `sampling_workers` / `sampling_reading` | Worker-owned native resources and individually timestamped observations |
| `disk_sensor_sampling` | Independently scheduled physical health and temperature reads |
| `process_cpu_sampling` | Independent application CPU cadence and coalesced immediate refresh |
| `runtime` | Tauri-managed desktop state and application lifecycle wiring |
| `runtime/demand` | Sampling demand derived from committed preferences and panel visibility |
| `runtime/coordinator` | Worker scheduling, generation checks, expiry and publication cadence |
| `runtime/observation` | Accepted observation application and bounded CPU/GPU recovery state |
| `presentation` | Bounded, coalesced visible-window publication and native display updates |
| `tray_display/display_entry` | Native display models shared by the tray, menu bar, taskbar and panel |
| `tray_display/display`, `format`, `temperature`, `power`, `fan` | Pure display assembly, numeric formatting and source-aware metric presentation |
| `tray_display/color_state` | Pure, source-aware color hysteresis shared by native displays |
| Other `tray_display` modules | Localization, native entries and Windows bitmap ownership |
| `taskbar_display` | Windows shell geometry, task-button space lease, painting primitives and fallback |
| `taskbar_display/native` | Native-thread window ownership, creation and shell restart lifecycle |
| `taskbar_display/native/messages`, `interaction` | Reentrant Win32 dispatch, pointer hit testing and context menus |
| `taskbar_display/native/update`, `rendering` | Live placement, lease coordination, visibility and native paint recovery |
| `floating_monitor/window`, `menu` | Cross-platform floating-window lifecycle, coalesced summary publication and native menus |
| `floating_monitor/placement`, `geometry` | Monitor-relative positions, work-area recovery, snap and bounded layouts |
| `windows_frame` | Shared frame recalculation before physical sizing of borderless Windows surfaces |
| `preference_schema` | Current preference models, validation and pure release upgrades |
| `preferences` | Revision conflicts, native application and persistence rollback |
| `panel` / `main_window` | Independent presentation, source anchoring, focus and background-launch behavior |
| Vue resident settings store | Serialized optimistic edits and rollback to committed preferences |
| Vue `md-status-display-settings` | Committed enable state and settings navigation |
| Vue `md-resident-display-editor` | Metric choices, GPU sample freshness and Windows temperature consent |
| Vue `md-resident-metric-order` | Local reorder preview, pointer cleanup and keyboard focus |
| `ResidentDisplayCatalogService` | One settings view’s schema/revision checks, subscription retries and disposal |
| Vue tray panel | Presentation of actual readings |

## Validation

`pnpm rust:test:desktop` builds the required desktop helper and runs the desktop
host's library tests, including resident preference validation, native display
calculations, sampling demand and publication ordering. `pnpm rust:test` includes
this entry point, so `pnpm check` and `pnpm build` also execute the host tests.
Core tests remain responsible for resource calculations and memory use cases.

Display formatting and preference decoding remain desktop-owned pure modules.
They do not depend on a running Tauri application, and they do not move native
UI concepts into Core or Platform. Window lifecycle, sampling-thread ownership
and native presentation remain explicit desktop adapters.

The floating monitor shares `window` lifecycle, `placement` persistence,
`geometry` calculations and `sizing` validation across macOS, Windows and Linux.
The native menu assembles controls and commits preferences; `menu_preferences`
owns its supported choices and pure field updates. Windows resource-panel
anchors, metric clicks and click-through live in `interaction`; AppKit and
Win32 and GTK visibility and mouse state remain in their native adapters.
Native context menus group font, bold, background, opacity and dividers
under Appearance. Windows keeps topmost, position lock and edge snapping as
root-level shortcuts alongside the shared layout and configuration actions.
The settings display-mode control is shared while each platform retains its
own persisted mode and supported choices.

## Linux floating monitor

`linuxDisplayMode` independently selects the existing tray surface (default), a
passive floating monitor, or both. Both mode keeps the same enabled readings in
the tray and floating monitor. Floating mode always retains the brand tray entry as a
recovery and configuration path. Left clicks never open resource panels.

Linux reuses the shared display entries, measured fixed columns, appearance menu,
settings editor and monitor-relative position bookmark. The GTK adapter keeps the
window out of the taskbar and pager and prevents focus changes when it appears.
The native move operation owns dragging; placement captures only an explicit drag
after button release. Appearance changes do not replace the saved position.
Before remapping a hidden GTK window, placement reasserts the bookmark even when
the layout is unchanged. GTK receives the initial position; mapped surfaces use
one GDK geometry request to avoid clamping a resize against the previous size.

Ubuntu 22 GNOME/X11 is the reference desktop. Absolute placement and position
restoration depend on the compositor's window-management capabilities; Wayland
compositors may restrict them. No compositor extensions, global desktop settings
or Windows interaction controls are introduced.

## macOS floating monitor

`macosDisplayMode` independently selects menu bar (default), floating, or both.
Floating-only mode keeps a brand menu-bar entry for configuration and quitting.
It reuses the formatted overview readings, localized font measurement, fixed
columns, typography and palette controls, without enabling detailed sampling.

The macOS surface only displays data. It uses non-focusable metric elements,
ignores metric commands, and provides no click-through mode. Its native context
menu shares appearance actions with Windows, plus layout, settings and position
reset; Windows-only interaction choices are not exposed.
AppKit orders the borderless window without activating the main application.
The window joins ordinary Spaces, stays out of window cycling and Mission Control
tiles, and does not opt into fullscreen auxiliary presentation. Moving uses the
native drag loop; placement waits for mouse release and persists only explicit
drags. Programmatic screen, DPI and layout moves never overwrite its bookmark.
macOS always keeps the monitor on top and movable, without edge snapping. Only
appearance settings are exposed. Shared Windows interaction choices are retained
in storage but do not enable interactions on macOS.

Native window titles follow the locale refresh independently of font measurements,
including selections such as CPU whose label is identical across languages.

Snapshot schema 9 separates native context-menu availability from metric clicks,
which remain Windows-only. Native adapters own visibility and mouse-button state; layout and
monitor-relative persistence share the same bounded geometry.

## Windows floating monitor

`windowsDisplayMode` selects taskbar, floating, or both; macOS and Linux keep their
existing surfaces. The monitoring enable switch and metric/source/color choices
remain shared. Floating mode releases the taskbar lease and never restores numeric
tray entries as a fallback. A brand icon remains available for settings and quitting. The floating window has
no manual hide action; changing display mode controls whether it is present.
Close requests do not hide or destroy the strip; explicit Quit exits normally.

Bootstrap completes the initial native display apply before background publication
can acquire display state. Floating WebView prewarming runs after bootstrap to
avoid a main-thread/native-call lock cycle during cold startup.

The floating surface is a compact status strip without a persistent title bar.
It uses 20 logical-pixel rows by default and inline labels and values, with right-click
or Shift+F10 for its native menu. The entire surface starts a native move after
a four-pixel pointer threshold; ordinary clicks toggle mapped resource details, and
a completed drag suppresses its trailing click. Fixed cells keep targets
stable as values change; small work areas scroll instead of moving off-screen.
Clicking the current resource closes its visible panel; a different resource
switches the content, and reopening selects the requested resource. Native entry
focus transitions retain toggle intent until the click is delivered; abandoned
presses and blank-cell clicks dismiss the unfocused panel after release.

Edge snapping is on by default and configurable through settings or the native
context menu. Disabling it preserves the exact release point inside the work area;
offscreen positions still clamp to its bounds. Older floating preferences gain an
enabled snapping flag without resetting layout, opacity, or other choices.

Compact cells use 20 logical-pixel rows. The WebView measures each localized label
and bounded numeric/unit samples in the actual fonts, then sends validated item
measurements to the native layout adapter. Native layout balances at most five
columns, sizes each column to its widest item, and bounds the window to the work
area, including a scrollbar allowance when needed. Vertical layout uses the widest
item. Only language, item selection, fonts, or DPI changes request measurement;
live readings never resize columns. A fixed complete-reading slot uses one numeric
style for digits and units and keeps its right edge stable across value, unit,
missing-reading changes. Each font role has its own measurement node, configured
before insertion; switching a connected clone's class can retain stale font metrics
in WebView2. Measurement failure uses diagnosed 108-pixel cells and retries on subsequent snapshots.
Localized compact labels identify every metric, including upload and download;
accessible labels retain their full descriptions. Fan values omit the redundant
RPM suffix on this surface. Ordinary fan readings reserve four digits; larger values
use compact thousands (for example, 12.3k), while accessible text and resource details
retain exact readings. Paired fan rows reserve their own font budget only when that
display mode is selected; changing modes requests measurement. Multiple fan rows fit
within the same 20-pixel cell.

A transparent native window and transparent document canvas expose the desktop
outside the CSS-rounded surface. `backgroundOpacity` accepts integer percentages
50–100 (default 100); only the surface background is blended, while text and hit
targets retain full opacity. The settings selector and native context submenu
share these preferences. Schema 29 upgrades preserve existing choices and add 100% background opacity.

The Windows floating `clickThrough` interaction setting defaults to false. It ignores native cursor events for that window only; drag, metric and context-menu commands also reject stale interaction requests while enabled. The tray retains an independent localized toggle, registered once for all tray handles, with checked/enabled state refreshed after preference changes. The recovery icon must be visible before cursor events are ignored. Previous preference schemas upgrade with click-through disabled.

Shared WebView2 browser recovery also recreates the floating window from current preferences. Each replacement clears document readiness, text measurements and native placement caches while retaining the saved monitor-relative anchor; layout and cursor-event behavior are reapplied before the window becomes visible. Recovery restores the floating anchor before dependent panels, waiting at most five seconds for placement so a failed layout cannot block other windows. Native show/hide transitions use non-activating display to retain resource-panel focus; both transitions stay with this adapter because Tao's visibility cache remains hidden.

The `dividers` appearance toggle defaults to true and controls dashed separators in both layouts, including trailing empty cells in horizontal grids. Schema 32 upgrades preserve all existing choices and add this setting. Separators are painted without changing cell dimensions.

Windows floating preferences include four font sizes (`standard`, `larger`,
`large`, `extraLarge`), a bold toggle, and seven background choices (`default`,
`darkGray`, `inkBlue`, `warmWhite`, `lightGray`, `black`, `white`). Default follows
the application theme; presets own foreground, border and status colors while
retaining the saved 50–100% background opacity. Existing configurations retain
standard type, regular weight and the default palette when migrated.

Floating snapshot schema 9 carries these settings. Typography uses scales
1, 1.15, 1.3 and 1.5 with matching native row heights 20, 23, 26 and 30 CSS pixels.
The layout request includes font size and bold state so stale measurements cannot
resize a newer appearance. Localized labels and bounded value samples are measured
in the actual WebView fonts; readings and background changes do not determine
geometry. Shared columns and right-aligned complete readings remain stable.

The floating WebView consumes only formatted `DisplayEntry` summaries and resolved column widths (event schema
7), with native color hysteresis. It never enables detailed sampling or process
lists. Hidden windows receive no periodic events. Native menus provide layout,
keep-on-top, lock, edge snapping, font size, bold text, background presets and opacity, reset, and settings actions without clipping to the small
window. Locking prevents native drag requests but leaves metric navigation enabled.
The Windows drag adapter packs signed screen coordinates for `WM_NCLBUTTONDOWN`;
the pinned Tao implementation passes a pointer-valued LPARAM, which cannot start
the native move loop reliably. Keep this compatibility boundary local to the
floating window and recheck it when updating Tao.

`floating-position.json` stores a version-1 monitor-relative bookmark independently
of preference revisions. Invalid bookmarks rebuild to the default position. Work
area and DPI changes clamp and resize the window; user drags snap only after release.
An axis with no remaining movement space retains its saved relative anchor, so a
temporarily constrained layout does not reset the position on a larger display.
Only completion of an explicit native drag marks a position as a user edit. Windows
move notifications caused by RDP, DPI, or work-area changes do not replace it.
Horizontal cells retain stable widths, balance wrapped rows, and scroll when a
small work area cannot hold all selected devices. Details open outside the monitor
surface where space permits. Windows exposes `floating.hideOnFullscreen` in its
behavior settings and context menu, enabled by default for new and migrated
configurations. When enabled, foreign fullscreen windows on its display temporarily
hide the monitor without changing the user's display mode. Disabling it skips
fullscreen detection and restores the monitor immediately. macOS retains its
AppKit fullscreen exclusion independently of this Windows-only setting.

Acceptance must cover Windows 10 and 11: taskbar lease release/restoration, both
layouts and themes, lock/menu/metric interactions, display-mode transitions, cold restart,
fullscreen, DPI/work-area changes, negative coordinates, and unavailable readings.

## Contracts and cadence

See [sampling architecture](sampling-architecture.md) for source ownership, demand,
recovery, observation age and diagnostic contracts.

Fresh installations enable resident display with Logo, CPU, memory, disk usage,
and network selected. Disk and network use automatic selection. Windows starts in
taskbar mode, preferring automatic placement with background enabled. Automatic placement prefers right on
Windows 10, left on centered Windows 11 taskbars, and right on left-aligned ones.
The existing shell inspection refreshes this decision every second; manual choices
remain fixed. Windows 10 reserves the selected edge of its task-button container,
so even uncombined application buttons leave room for the monitor. Windows 11
reserves XAML space at the outer left edge for centered taskbars, beside Start
for left-aligned taskbars, or after the application-button repeater, then
returns that reserved slot directly, so centered buttons do not send the monitor
to an unrelated outer gap. Unknown environments prefer right,
and collision checks still apply. These defaults do not replace saved choices.

Resident preferences use schema version 42; resource readings use version 39.
The system-status editor can restore the native platform defaults through the
existing revision-checked save transaction. Reset preserves the enabled state and
all platform display modes. Floating placement is stored independently and is
not reset; resizing restores its existing monitor-relative anchor. The shared
dialog footer is unchanged; only this editor adds the confirmation action.
On a revision conflict, reset retries the full default options once while retaining
the latest enabled state and display modes. Ordinary edits still rebase only
disjoint fields. Reset completion follows the save result, independently of error
flags that a queued native-preference refresh may clear.
The runtime and IPC accept only the current structure. Persisted configurations from
release tags v1.1.1–v1.1.9 (schemas 7, 8, 10, 21 and 25) and the previous monitoring schemas 28 through 40 upgrade directly to schema 41;
development-only schemas, including 26 and 27, have no migration contract. Each
supported release has a frozen fixture beside the pure decoder. Upgrades retain
saved order, visibility, colors, source choices and consent; newly introduced
metrics start disabled. Loading performs no writes. The next explicit settings
save persists only the current structure, without passing through intermediate
schemas. Unknown versions or malformed data use runtime defaults with diagnostics.
An explicit settings save, including restoring defaults, replaces unknown-version
preferences with the current structure without creating a backup. Malformed data
with a known or missing version still rejects saves. `resident.json` is owned
by the desktop configuration adapter, outside plugin store exit callbacks. Explicit
saves reread and validate the persisted version, write and sync a temporary file
in the same directory, then atomically replace the target. Failed writes preserve
the previous record and do not publish new runtime preferences.
Release structures and the previous monitoring schemas 28, 29, 30 and 31 are migration sources.
They remain readable to preserve existing monitoring installations.
Converters upgrade directly rather than accumulating intermediate version steps.
Display metrics must contain each current metric exactly once; custom order is retained.
GPU display is disabled by default. Compact mode reduces horizontal cells from 38/81
to 34/55 DIP (percentage/network), with abbreviated network units
(B/K/M/G/T) and unchanged numeric precision. Taskbar labels and values use 13 DIP Segoe UI. The shared size converts to physical pixels with nearest-pixel
rounding at the monitor DPI; paint and hit testing share those bounds.
Writes accept only the current validated structure. Memory snapshots use version 5; release results retain their separate version 1 contract.

Preference writes retain native revision checks across WebViews. A stale writer
receives the typed `preferencesConflict` reason and rereads committed preferences.
The frontend rebases only disjoint fields and retries once; arrays remain atomic
fields. Competing edits to the same field, repeated conflicts and storage failures
retain explicit failure and rollback. Ordinary writes add no extra IPC reads.

CPU overview samples every 2 seconds on both platforms, whether visible or hidden. Memory samples every
3 seconds, network every 1 and disk every 30.
Their freshness limits are respectively 5, 10, 5 and 90 seconds. CPU, memory, network and disk remain active while resident display is enabled,
regardless of native selections or panel visibility. On macOS and Windows, GPU
usage and temperature sampling also remain active while resident mode is enabled,
so hiding the panel or changing display selections does not interrupt histories.
Platforms without a GPU provider retain catalogue-only discovery rather than starting
a periodic provider for a selected placeholder.
Disabling resident mode stops its overview demand. On Windows, explicit CPU
temperature consent can independently keep CPU sampling active. Explicit
device-catalogue requests may still read network and disk metadata.
Each overview worker creates its native reader only when demanded and drops it on
the same worker after demand stops and any in-flight query finishes. This releases
memory-process metadata, Windows network-change subscriptions, and CPU counters.
Re-enabling reacquires readers; hiding the panel preserves demanded readers and cached
rankings. Detailed GPU observations follow the selected panel page, independently
of the overview reader's demand. Resource acquisition/release logs record transitions rather than samples.
The panel has an overview of all five base readings on macOS/Windows (four on Linux),
with CPU, GPU, memory, disk and network detail pages on macOS/Windows. All pages retain
a uniform 610 logical-pixel height.
Explanatory health and network text is available through tooltips. SMART details
include cumulative reads, power cycles, spare capacity and warnings where supported;
filesystem names reuse mounted-volume enumeration. The visible Disk page also
demands the existing thirty-second temperature reading; the menu-bar choice still
controls background demand. Cached health temperatures never replace that live reading. System I/O totals are separate
from SMART lifetime counters. Interface IP/MAC metadata remains local display data,
never diagnostic or external-service payloads. macOS reuses its existing address
enumeration; Windows caches the local unicast table for ten seconds and invalidates
it on native topology changes. No per-process I/O/network tracking is added. Resource
version 25 adds disk health and network session totals/link rates; version 26 adds optional Wi-Fi metadata; version 27 adds lossless ATA attributes and additional standardized NVMe counters; version 28 adds per-interface persisted network traffic totals. Disk health is
read only on its visible detail page, cached for five minutes, and expires after
ten minutes. Reopening the same disk preserves the last health fields while stale
results refresh in the background without a temporary badge. Unsupported
and disconnected readings never reuse old health fields. Transient failures retain
the last successful health fields and their original timestamp with a failed status. Temperature observations
retain their original timestamps across panel demand changes and query storage
at most once per thirty seconds for the same volume and mount. Transient failures retry after thirty seconds; unsupported devices
retry after five minutes. Native disk selection/generation checks reject stale
results, and a health read reuses its SMART temperature when both are due.
Health queries do not enable SMART, run self-tests, install drivers, or elevate.
NVMe lifetime counters retain their 128-bit decimal representation across JSON;
ATA attributes retain their IDs, flags, normalized/current/worst values and all seven vendor bytes. Only verified model/firmware profiles decode units and life percentages; unknown profiles keep raw attributes without guessed summaries. See [SMART decoding](../../crates/mangodisk-platform/src/system_resources/disk_health/decoding.md). Windows
management-only status is explicitly distinguished from detailed SMART data.
Network totals count valid observed intervals for the selected interface. Source
changes start a new total, while pauses and counter resets prime a new baseline
without counting the unobserved interval. Link rates are reported by the native
interface and never described as internet bandwidth. No active traffic probes or
average disk response-time display are added.
Network metadata uses a bounded ten-second CoreWLAN cache on macOS for negotiated transmit rate, PHY mode, channel and signal/noise. Generic IFMIB Wi-Fi baud rates are not displayed as negotiated rates; receive link rate stays absent when the native API supplies only transmit rate. Current interface MAC addresses can differ from factory addresses under private addressing. Traffic totals cover the monitored interface in this process; the UI labels them as session totals. Health percentage is max(0, 100 - NVMe percentage used). Every available health reading retains its temperature metadata slot while a live sample is unavailable, including Windows management-only health, preventing field reordering on tab switches.

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
GTK focus is requested from an idle callback after queued visibility changes.
The callback restores the prewarmed window's focusability and checks current
open intent before presenting it, so outside clicks dismiss the first opening
without a cancelled opening being shown again.
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
Physical links prefer SystemConfiguration’s native link-active state over driver
`IFF_RUNNING` flags, falling back to flags when native state is unavailable;
`IFF_UP` and a usable IP address are still required. Interface-state diagnostics
are emitted only on changes and exclude interface identities, IP and MAC addresses.
When IPv4 and IPv6 use different primary links, automatic selection prefers
the physical IPv4 primary rather than sorting equal priorities by interface name.
Windows reads interface state and byte counters every tick. Default-route preferences
are cached for at most thirty seconds and invalidated by native route/interface
notifications. Callback state is process-lifetime and never touches UI or reader
pointers; worker-owned subscriptions cancel outside callbacks. Registration failure
retains per-tick metadata queries. Route-query failure clears the preference and
retries next tick, with diagnostics only on failure/recovery transitions.
Automatic selection avoids virtual/loopback interfaces; an explicit selection is
never silently replaced. Only local writable/system or user-mounted disks are
listed; a selected volume is identified independently of its current mount name.

Public egress discovery is separate from the selected local adapter and traffic
sampling. Core probes the anonymous versioned MangoDisk endpoint over IPv4 and
IPv6 independently, constraining DNS and local source families while retaining
normal HTTP proxy policy. The actual response family determines its displayed
row; missing country metadata does not hide the IP. Country names are localized
on hover or keyboard focus without flags.

A resident `public_network` owner keeps a memory-only, one-hour cache with
per-family in-flight requests and generations. It has no background timer and
makes no requests at startup. Opening the network panel reuses a fresh cache or
queries an expired one. Cache age starts when a query succeeds; failure of only
one transport family does not expire the other family's usable result. Manual
refresh bypasses cache age. Hidden panels receive no cache events or periodic
work. Already-started requests may finish and populate the native cache.

A visible panel reads versioned snapshots through `monitoring_network_addresses`
and listens for newer revisions; reopening subscribes before reading the native
cache, so older command responses cannot overwrite newer events. Five-second
local configuration observations and online events check for network changes
only while the panel is visible. They do not query an unchanged egress just
because time passes.

The network settings dialog contains only the public IP switch, enabled by
default. Discovery requires both system monitoring and public IP to be enabled.
Preference schema 41 removes the former refresh interval from schema 40 while
preserving the public IP switch and unrelated settings. Disabling discovery clears
addresses and rejects late responses. When disabled, the panel retains the IPv4
row with a localized "Not enabled" state and keeps the settings button available;
refresh is hidden. IPv6 requires an actual address while discovery is enabled.

Before refreshing, Core reads a local interface/address/default-route token,
including virtual interfaces. A changed token invalidates old addresses and
responses. Same-network failed refreshes retain last known values with an explicit
stale indicator. Disabling clears the cache, invalidates pending results, and
stops visible observations; an already sent HTTP request may finish but cannot
repopulate the disabled cache. Repeated requests coalesce per family, and obsolete
completions restart only their own family when a newer request needs it.
Each HTTP probe has a five-second connect timeout, ten-second total timeout,
4 KiB response limit and no redirects or authentication credentials. Public-IP
queries opt into the same first-party metadata headers as update discovery and
feedback. Missing installation identity is omitted; lookup never creates an ID
or prevents address discovery.
Probe diagnostics contain only trigger, family and outcome, never IP addresses.

## Native and interaction boundaries

macOS uses one combined entry with a Retina-aware drawing-handler image. Labels
and values occupy two rows; network directions retain colored arrows and explicit
units. Rate digits are right-aligned in a fixed field; units have a separate fixed
origin so digit-count and unit changes cannot move neighboring text. Native column geometry keeps numeric changes
from resizing the entry (220 pt for all metrics plus the brand). Native appearance
and recreated buttons invalidate the image cache; accessibility retains the textual
summary. The adapter
uses Tauri 2.12 native tray access; upgrades require native UI regression checks.
On macOS, the adapter registers a menu with tray-icon 0.25 and updates it when
the locale changes. tray-icon retains the menu, attaches it to the status item
only during native secondary-click tracking, then detaches it on dismissal.
This follows
[tray-icon #365](https://github.com/tauri-apps/tray-icon/pull/365): macOS 27 swallows
primary clicks while `NSStatusItem.menu` is non-null. Apple documents this
[menu/action precedence](https://developer.apple.com/documentation/appkit/nsstatusitem/menu).
The native library owns the menu event loop; the adapter does not manually
invoke status-button actions. No display-state lock or tray-icon borrow spans
that nested loop. Primary presses toggle the panel once; releases do not toggle
it again. Secondary presses hide the panel while the native menu opens.
The complete tray-handle lifetime stays on the main thread, including lookup,
cloning, and destruction. Tauri's `Send` wrapper contains tray-icon's non-atomic
`Rc`; dispatching only its native methods does not make background ownership
safe. Workers pass application handles, entry IDs, and prepared display data,
without cloning detailed process lists for native rendering. The existing
bounded presentation queue preserves snapshot order across preference commits;
display locks are acquired only after main-thread dispatch. Diagnostic handle
counts are queued without blocking sampling.
Blur during a primary press over the entry is deferred to the toggle; keyboard
switching and outside clicks still dismiss it. Preference and locale refreshes
update the retained menu without attaching it to the status item. The panel
uses a real nonactivating `NSPanel` to host its Tauri-owned WebView. It can
become key without becoming the main window or activating the application, so
opening it does not leave another app's fullscreen Space. The hidden Tauri host
retains its original window and delegate; no Objective-C class is replaced.
Each open resolves geometry on AppKit's main thread and applies the requested
frame directly to the panel before showing it. Physical tray/work-area coordinates
are converted to logical AppKit points, including the bottom-left screen origin.
Monitor selection also retains the status-item window's backing scale: Tao's
per-display scaled global origins can overlap when Retina and ordinary screens
are adjacent, so rectangle containment alone does not identify the owning display.
The hidden host is not the positioning authority: Tao queues its geometry setters,
so copying its frame can expose the previous position on the first open. Native
frame diagnostics use AppKit points; the placement log includes physical pixels
and the target display scale.
The main-thread adapter moves the WebView back before releasing the panel on
exit. Visibility queries read the displayed native panel, not its hidden host,
and focus loss still follows the existing entry-click dismissal policy. It
explicitly uses AppKit's
[stationary collection behavior](https://developer.apple.com/documentation/appkit/nswindow/collectionbehavior-swift.struct/stationary)
so Show Desktop cannot sweep a newly opened panel offscreen. The panel explicitly
joins all Spaces as a fullscreen auxiliary window, clearing mutually exclusive
workspace/fullscreen flags. On macOS 13 and later it also uses
`CanJoinAllApplications` to appear over another application's fullscreen Space;
older versions retain the supported Spaces/fullscreen auxiliary flags. This
policy is confined to the tray panel and does not change the main window or the
application's activation policy. Check opening over another application's native
fullscreen window with the main window both visible and hidden, remaining in the
same Space, opening after clicking the wallpaper to reveal the desktop,
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
process reserves space by moving/shrinking `MSTaskSwWClass` under the rebar.
Explorer then lays out its inner `MSTaskListWClass` and pagination controls within
the allocated container. Shrinking only the inner list clips application buttons
without activating pagination. Closing the private
stdin pipe releases this lease on disable, normal exit, or abrupt GUI-process exit.
A session-local mutex serializes companion lifetimes, including restoration, so
rapid disable/re-enable cannot overlap allocations. Restoration only touches the
same shell process/control and our last applied axis; a newer Explorer layout wins.
Cross-axis DPI changes preserve the current thickness while restoring our axis.
TrafficMonitor also reserves `MSTaskSwWClass`; its width watcher can mistake another
program's contraction for a new baseline. Both the GUI and companion therefore
reject reservations whenever a foreign rebar surface is present, including a
hidden initializing panel. Shared hosts use fresh UIA gaps and peer bounds without
resizing Explorer's controls, or fall back to tray icons if no safe gap exists.
Manual left placement remains before application buttons rather than following
their growing UIA bounds.
TrafficMonitor's named top-level fallbacks are enumerated across displays:
they still resize task buttons when embedding fails. Only fallbacks on the host's
monitor participate; offscreen and secondary-display windows do not affect it.
Popup children use their actual ancestor parent because `GetParent` can return
the dialog owner. Hidden peers remain detected during initialization, but do not
contribute occupied pixels. An unavailable button hierarchy uses read-only gaps.
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
Centered task buttons receive a maximum width only when they would overlap the
monitor reservation. Left-side capacity also respects the corridor before the
notification tray: on crowded taskbars Explorer shifts centered buttons left to
avoid the tray, so screen centering alone can leave the Start button covered.
Capacity also respects the group's actual unconstrained visual center, derived
from Start's screen bounds and the group's desired width. The repeater's actual
width can span the entire taskbar and falsely imply screen centering. While
limited, retain the last unconstrained layout width and visual center:
`DesiredSize` and a temporary unconstrained measure still reflect Explorer's
smaller overflow model and can cause repeated expansion and collapse. Poll a
Z-order-independent signature of visible application windows once per second,
excluding windows from the taskbar's Explorer process and tool windows. A changed
signature or changed taskbar width, tray boundary, DPI, or XAML frame width
permits one full layout probe so newly available space can release the limit.
Failed enumeration alone retains the safe limit. Native regression checks include
a stable overflow threshold, opening and closing applications, natural centering
when space is available, tray-corridor overlap, changed monitor reservations, and
restoration on disable or exit.
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
thread performs no inspection while resident display is disabled.

Placement stays within the parent client area and excludes occupied controls with
a margin. During taskbar orientation changes, empty or inconsistent parent bounds
wait for the shell layout to settle instead of activating no-space fallback.
When a valid notification-area rectangle is available inside the shell, its cross
axis defines the visible content band for surface sizing and centering. Both
reserved slots and gap placement retain their original horizontal/vertical edge
allocation, then align within that band. Missing or inconsistent content geometry
retains the host bounds; no fixed pixel correction or process DPI override is used.
Layout diagnostics include shell, host, content and resulting native HWND bounds.
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
main window is not revealed. Right click exposes Open, Settings and Quit.
Panels opened from settings whose previous entry is hidden or removed first use a
remaining taskbar entry, then the monitor fallback. Position diagnostics include
the requested entry, resolved anchor source, and physical coordinates.
Window callbacks guard
against synchronous Win32 message reentry. Display settings share pointer and
keyboard reordering across platforms.


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

Resource readings use schema version 39 and nested `SystemResourceSnapshot` payloads use version 5; frontend adapters reject mismatched versions. Memory history records occupancy from the existing three-second sampler. CPU, GPU and memory use a fixed 0–100% scale. Network and disk activity share a symmetric scale: upload/write above zero, download/read below it. Gaps remain blank. The frontend buffers one sampling interval plus 250 ms before revealing each completed segment from the right; numeric readings remain live. Core retains up to 80 seconds / 96 samples so a reopened chart can reconstruct the buffered minute and offscreen endpoints. The frontend retains two additional intervals at the left edge. During a brief delivery delay, the playhead waits for completed data and catches up at no more than 1.1× speed; genuinely expired data still scrolls out. Pausing demand preserves existing readings and history with their original timestamps, while source changes clear the corresponding history. Rate scales hold their range for 30 seconds before a substantial reduction, and range changes ease over 600 ms using a shared SVG group. Continuous SVG updates are capped at 30 frames per second regardless of display refresh rate, without changing sample cadence or live numeric updates. Horizontal scrolling uses that group’s native transform instead of a composited CSS bitmap, preserving vector strokes at fractional positions. Reduced-motion mode applies scale changes immediately and disables continuous scrolling; hidden or fully expired charts stop their frame loop.

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

Preference version 16 stores independent color conditions for CPU/GPU
usage, memory, disk, and CPU/GPU temperature. Each metric has an enable flag
and zero to sixteen rules: below, inclusive-start/exclusive-end range, or
at least. Conditions cannot overlap; gaps use the system foreground. Thresholds
are whole percentages through 100 or Celsius values through 150. Classification
uses the rounded displayed value; missing readings never match a rule. Native
adapters share classification and theme-aware colors. The global switch gates
all metric rules. Each display item opens a fixed-height editing dialog with a
local draft, Number Field controls, explicit save/cancel, and deletion of every
rule. Closing without saving leaves persisted preferences unchanged.

New installations use green/orange/red defaults. Only adjacent descending
boundaries have three-percentage-point or two-degree hysteresis; gaps immediately
return to the system foreground. The macOS compact setting is independent.
macOS compact mode preserves labels, percentages and direction
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
versions. Resident preferences have an independent schema with explicit release upgrades.

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

Memory process rankings have a separate `memoryProcesses` reading and timestamp in resource protocol version 8. Overview-only samples preserve these rows without extending their freshness. Real empty detail samples clear the list; failed/stale detail results retain previous rows. Failures remain visible, while ordinary refresh is silent. The source memory snapshot is version 3; resident publication moves its optional detail payload into the dedicated cache to avoid duplicate IPC data. Frontends reject earlier resource versions.

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
queries independently of detail demand. The metadata cache retains its adapter handle;
physical adapter indices keep linked GPUs separate. Temperature is converted from
deci-Celsius, and unsupported or invalid readings are omitted. Wholly unavailable
adapter telemetry queries retry after thirty seconds.
The panel displays dedicated usage only; shared usage is withheld because its
equivalence to Task Manager has not been validated across drivers.
macOS probes Renderer and Tiler percentages and optional clocks and fan percentage
from the same IOAccelerator snapshot, including background samples. Identified GPU
temperature has a separate four-second read described below. Core count is discovered
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
Windows GPU utilization retains its PDH query for the first three transient data
failures, retrying after 1, 2, and 4 seconds. Persistent or structural failures
reopen the query after 30 seconds. A new rate-counter baseline does not establish
recovery; the first valid sample records the prior native error, attempt count,
and outage duration. Unchanged failures are summarized rather than logged every
sample, and unchanged memory-capacity diagnostics remain at Debug level.
`gpu_usage_probe <seconds> --details --diagnostics` enables the same diagnostics
on stderr without mixing them into its JSON observation protocol. Combining
`--lifecycle --details --diagnostics` also exercises detail demand transitions
before a reader release, without injecting hardware failures.

### CPU frequency observations

M1–M6 use validated Apple performance-state tables: M1–M3 store Hz, and M4–M6
store kHz. Known table identities take precedence over metadata fallbacks; M5
performance-core metadata identifies its separate renamed slot. Base M5 ECPU
metadata can point at an absent table, so it must not replace the known live table.
Firmware state order is preserved even when boost frequencies dip between adjacent
ordinals; IOReport ordinal and state-count validation still applies. M6 also
searches `pmgr-child` and uses the identified shared PACC table when needed. M5/M6
core channels keep efficiency, performance and super-core observations separate.
Base M5 two-tier metadata uses the legacy PCPU record kind for its Super tier;
slot 5 must not also create a nonexistent Performance class.
Unknown or inconsistent tables/states fail closed with model, stage, table and
channel diagnostics. Idle cores do not become fabricated minimum frequencies. A completed interval with
no active clocks publishes a fresh empty frequency sample, clearing previous clocks;
only an initial or invalid-duration baseline leaves the prior sample age unchanged.
The existing two-second detail demand and 30-second failure backoff are unchanged.

### CPU temperature observations

CPU temperature is independent of utilization and frequency. Its independent
worker requests a reading at most once every four seconds while macOS resident
monitoring or the optional Windows collection service is enabled. Display choices
and panel visibility do not interrupt collection. Stopping collection drops native
sensor handles; resuming starts a fresh observation.
Invalid numeric observations retry on the normal interval. Transport failures
back off from four to eight, sixteen and at most thirty seconds; capability and
permission failures retry after thirty seconds. Values cease to be live after ten
seconds, and every presentation checks sample age before displaying a live number. The Celsius trend defaults
to 0–100 degrees and expands in twenty-degree steps when retained samples exceed
100 degrees; this range is not a health threshold. Transient failures retain
history with an explicit recovery gap. Changed sensor meaning or revoked capability
clears that source history.

macOS reads only mapped CPU SMC keys and caches their metadata. CPU sampling
excludes invalid or unreadable sensors individually; one valid identified sensor
is sufficient. A per-key metadata failure does not block other identified CPU
sensors; only failed metadata is retried on subsequent observations, and recovered
keys rejoin the sample. Absent or unsupported model variants are not polled.
When all sensors fail, native transport/permission errors take precedence over
invalid numeric readings so source recovery can reopen a failed connection.
Apple Silicon reports the valid sensors' average and their actual
count. SMC failure with no valid readings still permits the named CPU HID fallback;
HID CPU readings use the same partial-availability policy. Diagnostics retain the
source, total/valid/invalid counts, affected sensor names, numeric values or native
failure reasons on transitions and at most once per minute for unchanged states.
Sensor enumeration order alone does not count as an availability transition.
GPU group validation remains independent and requires its complete selected set.
Apple Silicon
M1–M6 mappings distinguish CPU keys from GPU, PMU, battery and proximity sensors;
unknown chips are unsupported. Apple Silicon publishes the selected cores' average.
Intel prefers the CPU package key TCAD. When absent, machines with one to eight
physical cores can use the highest TC1C–TC8C core temperature, bounded by
`hw.physicalcpu_max`, using any valid sensors in that identified set. Unused slots are
excluded because they can expose plausible-looking sentinel values. A core maximum
is explicitly labeled as such, never as a package reading. If those readings are
unavailable or contain disabled-sensor sentinels, Intel falls back to the first
valid TC0D/TC0E/TC0F diode reading, then TC0P CPU proximity. The protocol and detail
hints distinguish diode and proximity from core/package measurements. Native
transport failures remain errors rather than triggering a different sensor.
These private
interfaces require per-chip and OS validation; available readings do not establish
support for every Mac. Linux recognizes coretemp package/core sensors and k10temp
Tdie, excluding Tctl offsets and anonymous thermal zones. Windows uses the optional
privileged collection service for single-package x64 Intel DTS and AMD Zen
families 17h, 19h, and 1Ah. AMD reads the fixed SMN temperature register through
the signed PawnIO module, applies range and documented Tctl offsets, and labels
the result Tdie. PCI configuration access uses a shared, bounded native mutex.
Public ACPI zone readings are intentionally unsupported rather than mislabeled. Unsupported
devices omit the entire temperature section. Native live indicators show an em
dash for failed or stale values. Detail panels may retain a diagnostic observation
for up to thirty seconds with its original timestamp and an explicit update state.

## CPU temperature display

The default display order keeps network last, after battery and whole-system power
items. CPU temperature is disabled by default. macOS exposes the same checkbox
and drag/keyboard reordering as other items, independently of CPU utilization.
Display item IDs are separate from the five resource worker IDs. Detail hints
identify the sensor aggregation and count; native hover text shows the current
temperature without sensor explanations. Temperature does not use utilization
warning colors.
The column keeps a fixed width and displays `—` for unavailable or expired data.

The display checkbox does not control collection. macOS samples temperature every
four seconds while resident monitoring is enabled. Windows requires explicit
consent for its optional privileged CPU collection service; once enabled,
it continues independently of panel visibility and resident display selection.
Linux omits the CPU temperature display item. Unsupported devices omit the entire
temperature section in CPU details.


### Cross-platform GPU temperature

GPU temperature has a separate, default-off display item. Preference schema 28
uses `gpuDisplay` and `gpuTemperatureDisplay`, each with `automatic` or `selected`
mode and bounded device IDs for explicit selections. The display editor expands
per-device checkboxes only in selected mode. Single-GPU settings and details hide
redundant selection controls; a disconnected fixed selection retains its recovery
control. Automatic utilization selects the
busiest fresh device. Automatic temperature follows the menu-bar utilization
selection, including multiple devices. Explicit temperature choices are independent.
`gpuAdapter` selects only the detail-panel device. During a released-configuration
upgrade, its saved single-device choice initializes `gpuDisplay`; automatic
temperature follows that same selection. Development-only selection fields have
no migration path and are absent from the current protocol.

Resource schema 31 exposes `gpuDevices`, with each device's original sample
status, timestamps and bounded, shared histories. No extra native polls are
introduced. The detail panel retains its single-GPU layout; utilization,
temperature and curves switch together, independently of menu-bar checkboxes.
Settings, detail selectors, macOS menu-bar and Windows taskbar columns use the
same sequential catalogue positions: GPU0, GPU1, and so on. Linked physical devices
receive separate positions. Device IDs, rather than display numbers, retain
selections and histories when the catalogue changes. Missing selections remain unavailable
rather than falling back to another card. The Windows notification-area fallback
has one icon per metric: it retains the first device value and includes every
device in the tooltip, rather than repeatedly overwriting one native handle.

Display selection projects cached observations without resetting utilization
counters or changing sampling demand. Device switches retain each device's real
timestamps and gaps; removal discards only that device's cache.

The resident GPU worker reads temperature at four-second intervals while
macOS or Windows monitoring is enabled, including when its display item or detail panel is hidden.
The display checkbox controls presentation only and remains selectable without a
reading. Only a fresh, valid observation establishes sensor support. Settings show
the actual unavailable status beside the label and retain device selection; native
columns show a dash until a valid observation arrives.
GPU and CPU cards share their
numeric, tooltip and chart presentation; GPU temperature cards retain selection and explicit unavailable states when adapters are present.
Transient failures preserve original timestamps and real history without adding
points or replacing missing values with zero. Values expire after ten seconds.

Driver temperature is preferred when available. Apple GPUs use generation-specific
SMC thermal-zone averages; Intel integrated GPUs use `TCGC` only when a single
Intel accelerator is identified. Arbitrary SMC prefixes and proximity sensors do
not establish GPU identity. A shared AMD `TGDD` sensor is not assigned to arbitrary
accelerators; AMD GPUs retain identified driver temperature support. Reads use
ordinary process permissions and the shared read-only SMC transport; no fan
control, helper, service or driver installation is involved.

Windows uses taskbar display. A plain tray icon remains the recovery entry when
Explorer is unavailable or the taskbar has insufficient space.

Windows uses the same identified temperature contract, cache, display column and
GPU detail chart as macOS. Public WDDM adapter performance data supplies device
temperature in tenths of a degree Celsius without additional driver installation
or elevation. Adapter identity includes the physical adapter index. Unsupported
driver readings remain unsupported, native query failures remain failed, and
utilization counter failures do not stop temperature sampling. Temperature is
sampled every four seconds while resident monitoring is enabled; failed driver
queries use the existing thirty-second backoff and transition diagnostics.

### Resource detail card layout

CPU and GPU details show utilization and temperature in separate adjacent cards.
Device identity appears only in the utilization card. Both charts share typography,
padding, and chart height without axis labels or grid lines. Overview and detail
summary cards share a 104-DIP height and eight-DIP vertical card spacing. GPU facts
remain outside the scrolling memory and engine detail sections, matching CPU facts.

### macOS battery display

Battery temperature, charging power, and battery power display are disabled by default.
Battery-temperature rules use the defaults described below; power rules start
disabled and empty. Limits are 100°C and 300 W. Menu-bar power uses rounded integer watts in both modes; nonzero
readings below 1 W display as <1W and use zero as their integer color-rule value.
Tooltips retain one decimal place. Integer color thresholds compare against the
displayed, rounded watt value. Native columns reserve three integer digits; whole-system values expand only
when a rounded reading needs four digits (1000–3000 W).
Resource version 21 carries independent freshness and source for all three readings.

Battery telemetry runs on its independent battery worker every two seconds while a
battery item or Power details is requested. With resident monitoring enabled, an
explicit preparation flag permits a cached observation at most every 30 seconds.
CPU-only temperature consent does not grant battery preparation; stopping both
battery demands releases its reader without stopping CPU sampling. SMC TB1T/TB2T provide the available
sensor average, with AppleSmartBattery Temperature divided by 100 as fallback.
SMC open or read failures release the connection and retry on the next scheduled
sample. Diagnostics retain the native failure and record recovery; missing sensor
keys remain distinct from connection failures.
Charging power means net power entering the battery, measured from battery
voltage times signed InstantAmperage, falling back to Amperage when unavailable.
PPBR can report discharge-rail consumption
while the pack charges, so it must not substitute for charging current. Battery
power has a typed charging/discharging/idle state. While disconnected it prefers
SMC PPBR, because pack current can remain zero for a minute after unplugging;
signed pack power is the fallback. While connected, it uses signed pack power,
never substituting discharge-rail consumption for charging current. Adapter input
and rated watts are not substitutes. Charging power reports 0 W when paused or disconnected; missing
flags or unavailable telemetry report unavailable. Values expire after five
seconds. Diagnostics record capability/source and charge-state changes plus a
periodic numeric observation, without battery identity or raw registry contents.

Native battery power headings use BAT with a separate red ↑ for discharging,
blue ↓ for charging, and IDLE for zero current without charging. Arrow colors
indicate direction independently of the value's color rules. Tooltips localize the state; stale readings
clear the state along with the value.


### macOS whole-system power display

Preference version 19 appends whole-system power disabled and preserves saved
selections, relative order, and color rules. Its color rules start disabled and
empty, use watts, and accept thresholds up to 3000 W. Readings are available
on macOS; other platforms retain a selected SYS placeholder. Localized tooltips name the
metric explicitly.

Whole-system power uses the same identified SMC PSTR sensor as Stats System
Total. The transport decodes both Apple Silicon floats and Intel signed sp96
fixed-point values; unsupported encodings are diagnosed separately from absent
keys. It does not require AppleSmartBattery and must not substitute adapter
ratings, battery power, or a fabricated zero when PSTR is unavailable. The
independent power-sensor worker reads it every two seconds while resident monitoring
is enabled, independently of menu selection, panel visibility, or the active tab.
Disabling monitoring releases its own SMC connection. Native failures reopen the
connection on the next scheduled read. Missing, failed, invalid and stale samples remain distinct;
values expire after five seconds. Diagnostics log sensor/state changes and a
bounded periodic watt observation. No private sensor dumps are persisted.


Battery presence is a separate nullable boolean in resource version 21. The
independent battery worker checks AppleSmartBattery and BatteryInstalled during
low-frequency preparation, with the native presence cache limited to thirty seconds. A successful empty match
means absent; a registry error retains the last known state instead of hiding
controls because a sensor failed. The settings catalogue always includes every display
item. Confirmed absence labels battery items as having no battery; selected native
columns remain visible as dashes and never reuse a previous battery value. Unsupported
platforms expose the same choices with a platform-specific status and placeholder,
without enabling native providers. Unselected auxiliary sensors are described as
collected when enabled until an observation establishes their actual status.
Windows CPU-temperature selection still requires its existing explicit capability
consent. Single-device selectors remain hidden.


Battery-temperature color defaults are green below 40°C, orange from 40°C to below
45°C, and red at 45°C or above.
Current-version users can still disable or remove every rule. Charging, battery
and whole-system power defaults remain disabled and empty. Battery help actions
sit immediately after their labels, separate from color controls.


### Disk temperature and network selection

`diskTemperature` has no temperature history and is disabled by default, immediately
after disk usage.
Disk temperature colors default to green below 50°C, orange from 50°C to below
60°C, and red at 60°C or above. These editable display presets do not represent
universal drive health limits. Saved color rules, including disabled or empty
ones, remain unchanged until the user restores defaults and saves. Disk capacity
selects a mounted `diskVolume`, grouped by physical device. Temperature uses
`diskTemperatureDevice`: null follows that volume's physical disk; a fixed hardware
identity selects an unambiguous mounted volume on the chosen disk. These selections
retain independent results, freshness and color state. Display disk numbers never
serve as saved identities. Hardware model and serial (or Windows GPT/MBR identity)
form the device key; unavailable hardware identity uses a conservative volume-bound
fallback, so an unrelated replacement is never selected by its display number.
The volume catalogue exposes models, physical numbers, mounted names/letters and
all backing devices. Spanning volumes stay selectable for capacity but cannot
provide one inferred temperature. Unknown topology is labelled explicitly.
`diskDetailTemperature` remains tied to the capacity/detail volume while
`diskTemperature` follows the independent display choice. Both sources share a
bounded two-entry cache: volumes mapped to the same physical disk reuse successful
observations with their own volume labels and the original timestamp. Failed reads
remain volume-specific. Two distinct sources each retain the original 30-second
cadence, only while demanded. The independent disk-sensor worker collects
temperature every 30 seconds only while its display item and resident monitoring
are enabled, or while disk details are visible. Changing temperature selection rejects
in-flight old-source results without resetting capacity or other metric baselines.
Values expire after 90 seconds and unavailable observations display an em dash.

macOS resolves the selected mounted volume through the IORegistry to one physical
block device and revalidates the volume after reading. Multi-device volumes are
unsupported rather than assigning an arbitrary disk's temperature. Read-only
NVMeSMARTLib and ATA SMART interfaces are destroyed after each query. ATA reads
never enable SMART, alter autosave or run self-tests. Other monitoring tools can
hold exclusive SMART resources; native stage and error codes remain diagnostic
failures rather than triggering a different sensor.

Windows resolves volume extents to one physical disk, preferring native storage
temperature descriptors. If unavailable, an identified NVMe bus uses the SMART
health log; an ATA/SATA bus first uses the read-only storage failure-prediction
query with its metadata handle. Its full SMART sector retains checksum validation.
Unsupported or denied queries fall back to the fixed SMART READ DATA operation,
whose raw device access can require administrator permissions. Invalid native
responses fail closed instead of substituting another reading. No additional
driver is installed and collection does not automatically elevate. Unsupported
virtual, RAID and bridged devices remain unavailable. Queries are bounded, run on
the disk-sensor worker and do not block capacity, CPU/network sampling or the UI. Logs record source or failure transitions
with the volume name and native stage/code, avoiding per-sample repetition.

The network catalogue exposes connected physical Wi-Fi/Ethernet links, ordered
by default-route preference and stable name/identity. It does not filter by
instantaneous traffic. Saved manual identities remain visible across disconnects
and may still refer to a VPN. Counters retain the full native interface set;
automatic selection continues to avoid virtual double counting. On macOS,
carrier flags additionally require a non-loopback, non-link-local assigned IP,
so unused Thunderbolt and Ethernet ports do not masquerade as usable links.

## Windows WebView process recovery

`webview_recovery` listens to WebView2 `ProcessFailed` for the main window,
resident panel, and memory settings. Diagnostics retain the window label,
browser PID, native failure kind, reason, exit code, and recovery action.
Renderer exits reload the affected document. Unresponsiveness requires two
observations spanning at least fifteen seconds, with no observation gap longer
than thirty seconds. Isolated stalls receive a new grace period because WebView2
does not emit a responsive event. GPU and utility process failures
remain under WebView2's own recovery policy. Main-browser failure closes all
associated controllers before reconstructing the application's visible windows
and hidden resident panel. Closing the last broken window during this interval
does not exit the application; explicit Quit still does.

Automatic recovery is limited to two attempts per window or shared browser in
five minutes. Navigation completion confirms renderer reload; missing completion
or exhausted retries produces a localized native warning that remains usable
without Vue. Failures from several controllers share one open warning until it
is dismissed, while retaining each controller's diagnostic. Core operations and
privileged sessions are retained throughout;
recovery never reissues a destructive command. Hidden main windows are recreated
on the next foreground request, and frontend readiness is reset before revealing
a recovered main window. Follow Microsoft's [process-related event guidance](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/process-related-events).

## macOS fan speed

Fan display defaults to highest speed. Selected IDs contain one or two unique,
bounded IDs sorted numerically; missing tachometers never change the saved selection.

The platform fan reader uses the existing read-only Apple SMC connection. FNum
identifies the fan count; F0Ac through F9Ac expose actual RPM. Integer counts,
Intel unsigned fpe2 speeds, and Apple Silicon float speeds have explicit encoding
and size checks. Counts are bounded to ten, RPM to 0–30000, and malformed or
non-finite values fail closed. Zero RPM is valid. A successful zero count denotes
fanless hardware. Exact hardware identifiers for confirmed M1–M5 MacBook Air
models also produce a successful zero-fan sample without opening SMC. The model
identifier is read once from `hw.model`; failed identification falls back to
sensor probing. Unknown, Intel Air, Pro, desktop and future models are never
classified as fanless from a missing FNum key: an unavailable count remains
Unsupported, while native read failures remain Failed. Missing individual
tachometers retain their identity with no fabricated zero reading. Settings and
native tooltips localize only a successful empty sample as “This device has no
fans”; missing, stale or failed observations retain their distinct status. Logs
include the model and distinguish model-confirmed absence from an SMC zero count.

Sampling runs on the two-second power-sensor worker only while resident display
and fan speed are enabled. Disabling releases its independent SMC connection.
Key metadata is cached per connection; missing keys are retried, and native
failures discard the connection and metadata before the next sample. Logs record
capability/error transitions with sensor diagnostics, without per-sample RPM logs.
Core expires readings after five seconds. Highest mode uses the highest valid
RPM with deterministic fan-ID ties. Average mode includes stopped fans as zero
and excludes unreadable tachometers; tooltips report valid/total counts. Selected
mode shows one fan as a heading/value pair, or two fans as equal-height numbered
rows in a single column. Missing selections remain in their physical rows as a
dash. Display text omits RPM units. Settings enforce one or two selections, and
backend validation rejects empty, duplicate, oversized or out-of-range lists. Windows and Linux do not expose this
item. Fan speed has no trend or control interface.

Fan color conditions use RPM with configurable thresholds bounded to 30000.
The default bands are green below 2000, orange from 2000 to below 4000, and
red from 4000 upward. These are speed bands, not hardware health limits.
Highest and average modes color their
aggregate value; paired rows classify independently by fan ID. Missing or stale
values use the system foreground. Descending boundaries hold for 100 RPM to
avoid flicker and reset when the mode, selected fans, or rules change.


### Persisted network traffic

Core `system_resources::network_traffic` accumulates only valid intervals from
`NetworkDelta` for the selected interface. Unobserved gaps, application downtime,
source changes and counter rollbacks never import operating-system totals.
Network identities retain per-interface history across application restarts;
this is monitored traffic, including LAN traffic, rather than Internet billing.

`network-traffic.json` uses schema 1. Missing history starts empty; unknown,
malformed or oversized history is preserved and disables persisted totals.
A lifetime file lease prevents concurrent writers. Background saves occur at
most once per minute, with an additional final save on normal exit. Atomic
same-directory replacement prevents partial JSON. An abnormal termination may
lose up to one unsaved minute. Reset saves before acknowledgement, targets the
confirmed interface and start time, and preserves the current session counters.
Intervals overlapping a reset are excluded. The adapter refreshes totals before
publishing a snapshot so an in-flight sample cannot restore pre-reset values.


### Windows battery and GPU fan sources

Resource readings use schema 36, including the `windowsBattery` source. Windows
queries native battery tags, information, temperature and status through read-only
battery IOCTLs. Presence uses `GetSystemPowerStatus`, cached for 30 seconds, and
unknown flags remain unknown. Enumeration is bounded and refreshed every 30 seconds;
queries retain battery tags so replacement or removal fails closed. UPS and other
short-term devices do not supply laptop battery telemetry. Temperature uses tenths
of Kelvin; power uses signed milliwatts only when capacity is absolute. Relative
units, unknown rates and contradictory direction flags never become watts. Multiple
batteries expose their hottest valid temperature and complete net power; missing
rates cannot silently undercount total power. Sampling stays on the independent battery
worker at the existing two-second interval while requested. Otherwise, resident
monitoring prepares one observation and refreshes it at most every 30 seconds.
Expired optional-sensor observations are described as collected when enabled after
their display items are disabled; actual read failures retain their failure status.

Taskbar battery-power headings retain red discharge and blue charge arrows,
show IDLE when idle, and clear direction when no current value is available.
GPU headings reserve a fixed native-font budget across fractional DPI scales.

Windows GPU fan sampling runs on the existing GPU worker when enabled. WDDM
`FanRPM` is the main GPU fan tachometer; it is independent of fan duty percentages
and requested fan speeds. Reuse adapter observations within 100 ms to avoid duplicate
queries in the same sampling cycle. Disabling the item releases NVAPI library and
initialization ownership while GPU usage and temperature continue normally.
Fan retries are independent of missing temperature and clock capabilities; sharing
a native observation never extends its original cache age. A positive valid reading or `MaxFanRPM` capability
must establish support before zero can represent a stopped fan. NVIDIA devices may
fall back to the installed driver's read-only NVAPI queries, loaded exclusively
from System32. If both sources fail, diagnostics retain the original WDDM reason
and the NVAPI fallback reason while preserving the fallback's error code.
The optional client fan-status V1 ABI follows LibreHardwareMonitor
0.9.6 and reports each actual tachometer; the public legacy tachometer is retained
for drivers without that query. Validate the ABI version, bounded count, unique
cooler IDs and RPM range before accepting any result. Invalid samples remain errors,
not partial or zero readings. Accept only unique NVIDIA PCI bus matches and single-physical-adapter
mappings; ambiguous matches remain unavailable. Successful vendor tachometer reads
establish support for a real zero. Initialization, enumeration and unsupported
per-device reads have 30-second recovery bounds. Highest and average
modes aggregate available GPU tachometers; fixed numeric fan choices remain macOS-only
because Windows GPU enumeration positions are not stable fan identities. Motherboard
and case fans require a separate, reviewed hardware capability and are not inferred
from `Win32_Fan.DesiredSpeed`. No driver, service, register writes or fan controls are
introduced. Windows whole-system power remains unsupported without a verified whole
system meter; CPU/GPU power percentages and battery flow are not substituted.


## Resident CPU frequency

Four optional display items default to off. Windows uses its interval-effective
PDH frequency; Apple Silicon exposes only the installed firmware frequency classes
(Efficiency, Performance, Super), independently selectable and sortable. Intel Mac
and Linux do not gain a live frequency provider. Identity reports the installed
classes without starting IOReport sampling; live values never decide which classes
exist, so idle clusters retain their selected slots.

An independent CPU frequency task owns frequency demand. Opening CPU details or enabling
an applicable frequency item requests the existing reader without enabling process
enumeration or GPU/disk details. Closing details keeps selected frequency items live;
disabling the last item pauses the interval baseline when details are closed.

A single installed frequency class uses CPU as its native label. Multiple installed
classes retain CPUE, CPUP and CPUS even when only one is selected or has an active
reading. Unknown or unsupported class selections retain their class labels until
capabilities are established. Settings and tooltips keep the full localized names. Menu-bar and taskbar columns use G; floating values and localized tooltips retain
GHz. Every frequency uses one decimal place and a fixed width covering 20.0 GHz.
Missing, failed, or expired observations show an em dash, never a static nominal
clock or zero substituted for an unavailable sample. Complete, valid intervals with
no active residency show localized idle text. Frequency has no utilization
color thresholds. Preference schema 38 migrates the previous modes and order while
inserting disabled frequency items; resource schema 36 includes typed frequency
classes and activity, and floating snapshot schema 11 includes their presentation.

### CPU frequency activity

Resource readings schema 36 includes per-class activity residency ratios. Only a
complete set of identified channels with positive-duration samples can confirm
an inactive class; missing channels or zero-duration samples invalidate both the
frequency and activity of that class without discarding other complete classes.
Failures and expired readings remain unavailable. Ratios describe the sampling interval, not an
instantaneous core power state. IDLE, DOWN and OFF retain provider-defined names.

Floating snapshots schema 11 carry typed frequency presentation state and a
localized idle width sample. Both numeric and idle text share a reserved value
slot. Existing menu-bar tooltips retain full names and sampling details. The
floating surface does not install native WebView tooltips: WebKit owns its
tracking rectangles, and overriding them can destabilize pointer handling.


### Power page

Resource-panel primary cards share the page-local `MdResourceCard` surface:
semantic card background and border, the same rounded corners, and 10px/12px
vertical/horizontal insets. Metric components own content layout and sampling
presentation. Standalone section headings align with the panel's content edge;
they do not add another horizontal inset. Auxiliary fact surfaces retain the
muted background to distinguish them from primary cards.

Panel navigation uses `PanelPage`, independently of the five sampling metric IDs.
The `resident_get_panel_page` and `resident_select_panel_page` commands and
`resident-panel-page` event share the same enum, including `overview` and `power`.
Opening Power requests battery sampling on macOS/Windows even when those displays
are absent from menu preferences. Closing it releases that additional battery
demand and retains the native reader for low-frequency preparation; configured
battery displays keep their own demand. macOS system power
keeps sampling while resident monitoring is enabled, so switching tabs or closing
the panel only stops its rendering and does not interrupt history.
Confirmed battery absence hides Power on macOS and Windows. An active or restored
Power selection returns to CPU and releases its additional battery demand. Native
shortcuts resolve the same fallback before toggle detection. Unknown presence does
not hide the page, and a newly detected battery restores the tab without navigating
away from the current page.

Resource schema 34 adds independent `batteryDetails` and `systemPowerHistory`.
Battery details expire after five seconds and never borrow freshness from CPU or
system power. History contains successful system-power samples only, uses the
existing bounded retention, and shows gaps after sampling stops or fails. Power
curves split at gaps longer than three seconds, preserving one missed two-second
sample while tolerating normal sampling jitter. Sleep and unavailable readings
are never filled with interpolated or zero values.
Resource schema 35 adds an optional `gapBefore` flag to trend points. Core marks
the first successful power sample after a failure, even after a quick retry that
fits within the normal time-gap tolerance. False flags are omitted from JSON;
the time axis preserves explicit boundaries during merging and reopening, so
failures while the panel is hidden are still visible when it opens again.

Resource schema 36 adds the `finishing` battery charge state. macOS uses the
public power-source charging, charged, and finishing flags with native pack flags
as a fallback when a public flag is absent. An explicit finishing flag takes
precedence over charged or charging; percentage alone never implies finishing.
Finishing retains the charging icon and any valid time-to-full estimate.

System power is the SMC system rail, not wall-meter input. Battery power is net
pack power (or the existing discharge rail fallback). Adapter input uses PDTR;
reported adapter watts are a separate capability and never a denominator for
live consumption. Native unavailable fields remain null. Capacity retains mAh on
macOS and mWh for absolute Windows battery reports; relative Windows units are
never labelled as mWh. Capacity health is full-charge/design, not an assertion of
macOS's smoothed health estimate. Unknown native time sentinels remain unknown,
and optimized charging is shown only for an explicit OS pause indication.

Panel presentation shows the last valid observation while detail sampling resumes,
including battery/adapter facts, disk health, temperatures and overview readings.
Cached battery and disk-health values retain their native timestamps and show
an update hint with the original observation time while stale. First battery
loads show progress rather than an unknown native charge state. Actual failures
remain visible. Cached values do not become fresh menu indicators
or synthetic trend points. Battery presence and
selected-device changes still invalidate observations. The cache lives in the
resident process; restarting the application starts without previous observations.
The selected disk receives one initial health attempt before its panel opens.
Hidden panels do not repeat SMART queries; visible detail refreshes retain the
five-minute success/unsupported and 30-second failure cadence. Disabling resident
monitoring prevents health warm-up, including catalogue-only requests. Health and
temperature share one bounded sensor worker independently of capacity and disk I/O.
Health/detail warm-up waits for the confirmed capacity identity; fixed-device
temperature can still run independently. Hidden cache replay preserves the original
sample time after a visibility change rejects an in-flight result. Sensor failures
update only sensor statuses and never replace a valid capacity observation.

Resource schema 38 exposes independent `cpuPower` and adapter-identified
`gpuPower` readings with `ioReport` or `appleSmc` sources. Apple Silicon uses
IOReport component energy estimates; Intel uses identified SMC power domains.
Neither reading is SMC whole-system power. Each IOReport owner keeps a nonblocking
baseline and uses the actual sample interval; counter failures, missing channels,
unknown units, clock changes and intervals outside 100–5000 ms never produce a
synthetic zero. CPU and GPU failures remain independent. The existing two-second
workers keep sampling with the panel closed, and readings expire after five
seconds. No ANE subscription is exposed.
The legacy Energy Model path is restricted to Apple Silicon before macOS 27;
macOS 27 legacy counters may freeze, so that layout remains unavailable pending a
validated CLPC implementation. Intel CPU power uses the `PCPC` package sensor;
core (`PC0C`), total (`PCTR`) and package readings are never substituted or summed.
Intel GPU power binds `PG0C` only to a unique AMD/NVIDIA firmware PEG/GFX0 PCI
device. Unknown, external and multiple discrete accelerators do not receive an
ambiguous `PG0C` reading. A uniquely identified Intel accelerator uses `PCPG`,
including on hybrid systems. GPU power follows the same Core-selected adapter as
utilization, including manual choices and automatic busiest-device selection.
Switching between integrated and discrete GPUs replaces the sensor owner rather
than reusing the other rail's reading. These SMC values are instantaneous watts,
without an energy baseline, and missing/invalid readings never become zero.
Initialization and read failures retry after thirty seconds. The shared SMC transport decodes
identified `sp87` signed fixed-point readings as big-endian values divided by 128,
alongside existing `sp78`, `sp96`, float and unsigned fixed-point encodings.
Other platforms retain their current sensors and report component power as
unsupported.

SMC remains the preferred temperature source. Only an unsuccessful CPU/GPU
observation may try explicitly named IOHID CPU/GPU sensors. PMU, NAND, battery and
anonymous sensors are excluded. GPU HID fallback additionally requires one
identified Apple accelerator. Invalid members reject the complete matched group;
CPU and GPU fall back independently and expose `appleHid` as the source. SMC
initialization or transport failure is retried after thirty seconds even while
HID succeeds; transient invalid sensor values can recover on the next interval.
A machine without those HID sensors still reports temperature as unavailable.

Preference schema 39 adds disabled CPU/GPU power display entries and color rules,
inserting new power entries after the matching temperature item while preserving
existing relative order, choices and thresholds. Saved power positions remain
customizable. Floating schema 12 adds their
shared display IDs. The CPU/GPU detail panels show fresh available component
power only; GPU power must match the selected adapter. Native menu indicators
also validate freshness and identity. Metric-specific entries open the matching
page; the combined macOS entry retains the last page.

Component power sits in the utilization card footer beside the device label,
with no reserved field for unavailable readings. Floating power fields preserve
one decimal place from the same validated observation before compact rounding;
menu-bar and taskbar fields retain their existing precision. Fixed measurement
samples reserve the complete decimal field without resizing on every update.
Each native surface owns its color hysteresis, using the precision it actually
displays. Floating power colors must not inherit the compact surface's rounded
threshold match. GPU catalogue removal invalidates cached component power and
its timestamp immediately; a returning device needs a new valid observation.
The GPU panel also requires the power adapter to remain in the current catalogue.
GPU power entries retain their sampled adapter identity on native and floating
surfaces. Multi-GPU displays show the corresponding GPU number, and tooltips name
the source adapter, so an independently selected utilization display cannot make
one GPU's watts appear to belong to another. Single-GPU labels remain unchanged.

Preference schema 42 adds nullable `taskbarMonitor`: null follows the Windows
primary display; a fixed choice retains the monitor device-interface path across
screen reordering and temporary disconnection. Existing schemas upgrade to the
primary-display policy without changing other settings. RDP drivers without a
physical monitor interface use the session display name as a best-effort identity.
The versioned taskbar monitor catalogue exposes current screen numbers and names;
identification shows short-lived, non-activating numeric labels.

Shell geometry and the native child bind to the selected Explorer-owned taskbar.
A missing display or taskbar temporarily falls back to the primary taskbar without
rewriting the saved choice, and selection recovers when it becomes available.
Secondary taskbars use fresh, collision-checked free space; the current XAML/classic
reservation adapters remain restricted to the primary taskbar. Insufficient space
keeps the existing tray fallback instead of modifying another screen's layout.
Fullscreen checks, DPI, hit targets and detail-panel anchors use the selected
screen. macOS menu-bar placement continues to follow AppKit.

### Explicit force quit

Memory application rows retain normal quit and expose force quit through a separate
menu and destructive confirmation naming the selected application. Cancelling
does not invoke Core. Force quit never follows an unsuccessful normal quit
automatically. CPU rows and shared hosts do not expose this action.

The panel-only command accepts an opaque application ID, not a PID or path. Core
resolves it against a fresh full process sample and rejects the current application,
unknown self identity, OS service images, and shared hosts. The platform reuses
its exact-image/bundle process closer with live identity and current-user checks;
Windows also requires the current session and rejects critical processes. It does
not elevate or authorize unrelated executables solely through ancestry.

Result schema 1 separates completed, still-running, unavailable and blocked states.
Completion requires a final process observation with no matching survivors; it
does not guarantee a drop in total system memory or prevent a later supervisor
restart. Both platforms check for a bounded quiet interval and terminate newly observed
matching instances within the shared deadline. A later supervisor restart remains
possible.
Both quit commands bring forward the existing serialized memory/CPU workers.
Expanded memory rows retain their ranking while present, but a ready sample removes
absent applications and unlocks their details. Failed samples and unreadable memory
values do not prove exit. Pending operations retain their row until they settle.
Unmounting a row releases that retention marker, including when the panel hides
before the IPC request returns; native termination continues independently.
Resource schema 39 and nested memory
schema 5 add the shared `canForceQuit` capability and reject incompatible snapshots.
Diagnostics include an operation ID, resolved image, request counts, remaining
process names, elapsed time, and native per-process reasons/codes.
