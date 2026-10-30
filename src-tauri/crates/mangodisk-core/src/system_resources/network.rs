use super::network_traffic::{NetworkTraffic, TrafficTotal};
use mangodisk_platform::system_resources::network::{
    InterfaceSample, NetworkCounters, NetworkInterface,
};
use serde::Serialize;
use std::sync::Arc;

/// Only usable physical links belong in the ordinary selector. Keep a saved
/// manual identity visible even when disconnected or backed by a VPN.
pub fn catalogue(interfaces: &[InterfaceSample], manual: Option<&str>) -> Vec<NetworkInterface> {
    let mut rows = interfaces
        .iter()
        .filter(|sample| {
            let interface = &sample.interface;
            manual == Some(interface.id.as_str())
                || (interface.physical
                    && interface.connected
                    && matches!(interface.kind,
                mangodisk_platform::system_resources::network::InterfaceKind::Wifi
                | mangodisk_platform::system_resources::network::InterfaceKind::Ethernet))
        })
        .map(|sample| sample.interface.clone())
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        right
            .connected
            .cmp(&left.connected)
            .then(
                left.default_route_metric
                    .unwrap_or(u32::MAX)
                    .cmp(&right.default_route_metric.unwrap_or(u32::MAX)),
            )
            .then(left.name.cmp(&right.name))
            .then(left.id.cmp(&right.id))
    });
    rows
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NetworkSelectionReason {
    Manual,
    DefaultRoute,
    PhysicalFallback,
    Disconnected,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkRate {
    pub cumulative: Option<TrafficTotal>,
    pub interface: NetworkInterface,
    pub received_bytes_per_second: f64,
    pub transmitted_bytes_per_second: f64,
    pub received_bytes_since_start: u64,
    pub transmitted_bytes_since_start: u64,
    pub receive_link_bps: Option<u64>,
    pub transmit_link_bps: Option<u64>,
    pub selection_reason: NetworkSelectionReason,
}

/// Prefer the saved link while connected, otherwise use the automatic policy.
/// Throughput and public-address discovery never determine link availability.
fn select<'a>(
    interfaces: &'a [InterfaceSample],
    manual: Option<&str>,
    previous: Option<&str>,
) -> (Option<&'a InterfaceSample>, NetworkSelectionReason) {
    if let Some(selected) = manual.and_then(|id| {
        interfaces
            .iter()
            .find(|sample| sample.interface.id == id && sample.interface.connected)
    }) {
        return (Some(selected), NetworkSelectionReason::Manual);
    }
    let candidates = || {
        interfaces
            .iter()
            .filter(|sample| sample.interface.connected && sample.interface.physical)
    };
    if let Some(route) = candidates()
        .filter(|sample| sample.interface.default_route_metric.is_some())
        .min_by_key(|sample| (sample.interface.default_route_metric, &sample.interface.id))
    {
        return (Some(route), NetworkSelectionReason::DefaultRoute);
    }
    let chosen = previous
        .and_then(|id| candidates().find(|sample| sample.interface.id == id))
        .or_else(|| candidates().min_by_key(|sample| &sample.interface.id));
    let reason = if chosen.is_some() {
        NetworkSelectionReason::PhysicalFallback
    } else {
        NetworkSelectionReason::Disconnected
    };
    (chosen, reason)
}

/// Debounce background source changes without modifying the persisted preference.
/// Initial selection and explicit preference changes do not wait for confirmation.
#[derive(Default)]
pub(super) struct NetworkSelection {
    manual: Option<String>,
    pending: Option<PendingSelection>,
}

struct PendingSelection {
    id: String,
    since_ms: u64,
    last_ms: u64,
}

impl NetworkSelection {
    pub(super) fn interrupt(&mut self) {
        self.pending = None;
    }

    pub(super) fn select<'a>(
        &mut self,
        interfaces: &'a [InterfaceSample],
        manual: Option<&str>,
        previous: Option<&str>,
        monotonic_ms: u64,
    ) -> (Option<&'a InterfaceSample>, NetworkSelectionReason) {
        let changed = self.manual.as_deref() != manual;
        if changed {
            self.manual = manual.map(str::to_owned);
        }
        let (candidate, reason) = select(interfaces, manual, previous);
        let Some(candidate) = candidate else {
            self.interrupt();
            return (None, reason);
        };
        if changed || previous.is_none() || previous == Some(candidate.interface.id.as_str()) {
            self.interrupt();
            return (Some(candidate), reason);
        }
        let current = previous.and_then(|id| {
            interfaces
                .iter()
                .find(|sample| sample.interface.id == id && sample.interface.connected)
        });
        // A disconnected source must not wait indefinitely for route rankings to
        // settle. Finish confirming a still-connected fallback before following
        // subsequent automatic route changes. Manual recovery keeps priority.
        let pending_fallback = self.pending.as_ref().filter(|pending| {
            current.is_none()
                && reason != NetworkSelectionReason::Manual
                && monotonic_ms
                    .checked_sub(pending.last_ms)
                    .is_some_and(|gap| gap <= 5_000)
        });
        let candidate = pending_fallback
            .and_then(|pending| {
                interfaces.iter().find(|sample| {
                    sample.interface.id == pending.id
                        && sample.interface.connected
                        && sample.interface.physical
                })
            })
            .unwrap_or(candidate);
        let reason = if reason != NetworkSelectionReason::Manual {
            automatic_reason(candidate)
        } else {
            reason
        };
        // Two successful enumerations at least one second apart confirm a new
        // source. A pause, clock reversal or unavailable candidate restarts the check.
        let confirmed = self.pending.as_ref().is_some_and(|pending| {
            pending.id == candidate.interface.id
                && monotonic_ms
                    .checked_sub(pending.last_ms)
                    .is_some_and(|gap| gap <= 5_000)
                && monotonic_ms.saturating_sub(pending.since_ms) >= 1_000
        });
        if confirmed {
            self.interrupt();
            return (Some(candidate), reason);
        }
        match self.pending.as_mut() {
            Some(pending)
                if pending.id == candidate.interface.id
                    && monotonic_ms
                        .checked_sub(pending.last_ms)
                        .is_some_and(|gap| gap <= 5_000) =>
            {
                pending.last_ms = monotonic_ms;
            }
            _ => {
                self.pending = Some(PendingSelection {
                    id: candidate.interface.id.clone(),
                    since_ms: monotonic_ms,
                    last_ms: monotonic_ms,
                })
            }
        }
        // A connected old link remains truthful during confirmation. Never
        // present a disconnected link's cached counters as a live observation.
        let reason = match current {
            Some(current) if manual == Some(current.interface.id.as_str()) => {
                NetworkSelectionReason::Manual
            }
            Some(current) => automatic_reason(current),
            None => NetworkSelectionReason::Disconnected,
        };
        (current, reason)
    }
}

fn automatic_reason(sample: &InterfaceSample) -> NetworkSelectionReason {
    if sample.interface.default_route_metric.is_some() {
        NetworkSelectionReason::DefaultRoute
    } else {
        NetworkSelectionReason::PhysicalFallback
    }
}

#[derive(Default)]
pub struct NetworkDelta {
    previous: Option<(String, NetworkCounters, u64, u64)>,
    traffic: Option<Arc<NetworkTraffic>>,
    session_interface: Option<String>,
    received: u64,
    transmitted: u64,
}

impl NetworkDelta {
    pub fn set_traffic(&mut self, traffic: Arc<NetworkTraffic>) {
        self.traffic = Some(traffic);
    }
    pub fn cumulative(&self, id: &str) -> Option<TrafficTotal> {
        self.traffic.as_ref()?.total(id)
    }
    pub fn sample(
        &mut self,
        interface: &NetworkInterface,
        counters: NetworkCounters,
        elapsed_ms: u64,
        selection_reason: NetworkSelectionReason,
        timestamp_ms: u64,
    ) -> Option<NetworkRate> {
        if self.session_interface.as_deref() != Some(&interface.id) {
            self.session_interface = Some(interface.id.clone());
            self.received = 0;
            self.transmitted = 0;
        }
        let previous =
            self.previous
                .replace((interface.id.clone(), counters, elapsed_ms, timestamp_ms))?;
        if previous.0 != interface.id {
            return None;
        }
        let elapsed = elapsed_ms.checked_sub(previous.2)?;
        if !(100..=5_000).contains(&elapsed) {
            return None;
        }
        let received = counters.received.checked_sub(previous.1.received)?;
        let transmitted = counters.transmitted.checked_sub(previous.1.transmitted)?;
        self.received = self.received.saturating_add(received);
        self.transmitted = self.transmitted.saturating_add(transmitted);
        Some(NetworkRate {
            cumulative: self.traffic.as_ref().and_then(|traffic| {
                traffic.record(
                    &interface.id,
                    received,
                    transmitted,
                    previous.3,
                    timestamp_ms,
                )
            }),
            interface: interface.clone(),
            received_bytes_per_second: received as f64 * 1000.0 / elapsed as f64,
            transmitted_bytes_per_second: transmitted as f64 * 1000.0 / elapsed as f64,
            selection_reason,
            received_bytes_since_start: self.received,
            transmitted_bytes_since_start: self.transmitted,
            receive_link_bps: counters.receive_link_bps.filter(|value| *value > 0),
            transmit_link_bps: counters.transmit_link_bps.filter(|value| *value > 0),
        })
    }

    pub fn reset(&mut self) {
        self.previous = None;
    }
}

#[cfg(test)]
#[path = "network_tests.rs"]
mod tests;
