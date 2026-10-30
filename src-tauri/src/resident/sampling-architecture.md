# Resident sampling

The desktop coordinator owns when read-only telemetry is requested. Platform
readers own native resources and capability-specific caches. Core owns validation,
source identity, deltas, freshness, bounded history and snapshots. Presentation
coalesces those snapshots independently of sampling and native UI work.

## Sources and demand

`sampling_policy::Task` is the diagnostic identity and cadence registry.
`runtime/demand` derives policy from committed settings, panel visibility and
existing Windows temperature consent. Each task projects only the fields that
change its own observation; opening frequency details cannot invalidate an
in-flight temperature or battery query.

| Source | Normal interval | Owner and demand |
| --- | --- | --- |
| CPU counters | 2 s | CPU reader; resident monitoring or existing CPU temperature consent |
| CPU temperature | macOS 4 s; Windows client 2 s | Independent temperature reader; Windows service owns its 4 s hardware cache |
| CPU frequency | 2 s | Independent frequency reader; CPU details or selected frequency entries |
| CPU/system power and macOS fans | 2 s | Power-sensor worker; independent demand flags and observation timestamps |
| Battery | 2 s live; 30 s preparation | Battery worker; Power page, selected battery entries or resident preparation |
| GPU | 2 s; native temperature cache 4 s | Adapter-owned usage, detail, temperature, power and Windows fan readers |
| Memory overview | 3 s | Machine-wide memory/pressure only; no process enumeration |
| Application memory | 3 s while requested | Process-only reader; Memory details or bounded icon warm-up |
| Application CPU | 4 s background; 2 s visible | Independent interval counters and coalesced explicit refresh |
| Network counters | 1 s | Interface reader and topology subscription |
| Disk capacity/catalogue | 30 s | Mounted-volume metadata and selected capacity |
| Disk I/O | 2 s | Independent physical counters, active while resident monitoring is enabled |
| Disk sensors | 30 s dispatch | Shared immutable volume catalogue; native health cache 5 min success/unsupported, 30 s failure |

A task acquires its reader lazily on the owning thread. Stopping demand releases
it after the current native query exits. A timeout never creates another reader
behind an uncancellable call. Selected disk health waits for the capacity source
identity; fixed-device temperature can proceed independently. Physical device
changes invalidate source-bound health even when the logical volume ID is reused.

## Bounded execution and recovery

`sampling_dispatch` uses one pending slot per task and nonblocking sends. A task
has at most one in-flight query. A changed demand, source or resume epoch rejects
old results while retaining that slot until the old query actually finishes.
Normal deadlines are spaced from completion; missed intervals are not replayed.
Baseline-only CPU/GPU/frequency observations have at most two quick 250 ms retries.

Confirmed thread exit invalidates its in-flight generation and schedules a bounded
30 s restart. Other sources continue. A merely slow thread keeps its original
slot. Application CPU retains its own interval-baseline and refresh state machine
with the same exit/restart rule.

Clock reversal, a wall/monotonic clock discontinuity or a long coordinator pause
invalidates old generations, resets counter baselines and reopens native readers.
Original sample times are retained. There is no synthetic catch-up history.

Temperature numeric gating preserves the native handle and normal interval.
Transport failures reopen with 4/8/16/30 s bounded backoff; unsupported or denied
capabilities retry after 30 s. Previously identified sensors that temporarily
vanish remain recovery failures. Windows service sensor backoff is not repeated
at the client layer. Cached service errors do not postpone the native deadline.

## Observation and presentation contracts

`sampling_reading::Observed` records each native sub-reading's completion time and duration.
One slow GPU sub-query cannot make a later, valid temperature appear older than
it is or refresh an earlier expired usage reading. CPU temperature carries the
native sample timestamp; Windows service cache replay never creates a new sample.

CPU/GPU temperature is live for 10 s. A transient failure may retain the last
actual reading in detail panels for at most 30 s with its original timestamp and
explicit failed/stale state. Native live indicators still show a dash. Retained
readings never create trend points. Recovery creates an explicit history gap;
source changes, device removal and revoked capability invalidate cached values.
Power, memory rankings and disk health retain their existing typed availability
and freshness contracts. Unavailable data never becomes a fabricated zero.

`resident_query_summary` separates native duration, queue delay, delivery delay
and rejected generations by task, with bounded timing storage and periodic
summaries. Resource acquisition/release, worker exit/restart and temperature
recovery diagnostics log transitions rather than every poll. Native diagnostic
codes and useful source names are preserved; process payloads and network
addresses are not diagnostic fields.

## Network source selection

A connected manual interface takes priority. When it disconnects or disappears,
Core temporarily uses the automatic physical/default-route selection policy;
the saved preference remains unchanged. A stable return restores the manual
interface. Background source changes require matching successful observations
at least one second apart, with no gap over five seconds. Failed enumeration,
pausing or resuming cancels pending confirmation. Initial selection and explicit
preference changes take effect immediately. Zero traffic and public-address
lookup failures do not indicate a disconnected interface. When the previous
source is unavailable, a still-connected pending physical fallback finishes
confirmation even if automatic route rankings change. This prevents route churn
from leaving the panel disconnected indefinitely; manual recovery still wins.

Panels and live indicators use the actual sampled interface. The selector keeps
the saved preference and its existing disconnected state without introducing
fallback notices. Source changes rebuild rate baselines and clear the current
trend; same-source interruptions create a history gap. Persistent traffic totals
remain keyed by interface and exclude baseline, disabled and invalid intervals.
Selection diagnostics record policy/fallback transitions without network addresses.

## Adjacent workflows

Public-address discovery remains opt-in and on demand, with serialized context
inspection, one flight per address family, generation rejection and an in-memory
one-hour cache. Main-window volume refresh retains its focus/visibility policy
and existing native capacity cache. Traffic history persistence, update discovery,
scan/cleanup and configured memory reclamation keep their own lifecycle and
side-effect boundaries. They are not dispatched as telemetry sensor jobs.

Use pure demand/generation/cache tests for protocol and recovery boundaries;
opt-in native tests cover real SMC readings, sensor reopening and blocked-source
isolation. Native measurements describe the tested machine and workload, not
support for every sensor model. On a machine with a stable active physical network
link, run `cargo test -p mangodisk-core native_network_fallback -- --ignored --nocapture`
to verify fallback rates against three real native counter observations without
changing system interfaces or preferences. Cross-platform changes require the applicable
checks and real execution on macOS and Windows.
