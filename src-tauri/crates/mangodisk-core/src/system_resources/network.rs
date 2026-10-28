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

/// A manual identity is sticky across disconnects. Automatic selection prefers
/// a physical default route, then retains a connected physical fallback rather
/// than chasing whichever adapter happens to transfer most bytes this second.
pub fn select<'a>(
    interfaces: &'a [InterfaceSample],
    manual: Option<&str>,
    previous: Option<&str>,
) -> (Option<&'a InterfaceSample>, NetworkSelectionReason) {
    if let Some(id) = manual {
        return (
            interfaces.iter().find(|sample| sample.interface.id == id),
            NetworkSelectionReason::Manual,
        );
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
mod tests {
    use super::*;

    #[test]
    fn persistent_totals_only_include_valid_selected_intervals_and_survive_a_restart() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("traffic.json");
        let traffic = Arc::new(NetworkTraffic::open(path.clone()).unwrap());
        let lan = interface("lan", true, Some(1)).interface;
        let wifi = interface("wifi", true, Some(2)).interface;
        let counters = |received| NetworkCounters {
            received,
            transmitted: received * 2,
            ..Default::default()
        };
        let mut delta = NetworkDelta::default();
        delta.set_traffic(traffic.clone());
        let reason = NetworkSelectionReason::Manual;
        assert!(delta
            .sample(&lan, counters(100), 1000, reason, 1000)
            .is_none());
        let value = delta
            .sample(&lan, counters(200), 2000, reason, 2000)
            .unwrap();
        assert_eq!(value.cumulative.unwrap().received_bytes, "100");
        assert!(delta
            .sample(&lan, counters(10000), 20000, reason, 20000)
            .is_none());
        assert!(delta
            .sample(&lan, counters(10), 21000, reason, 21000)
            .is_none());
        delta.reset();
        assert!(delta
            .sample(&lan, counters(500), 22000, reason, 22000)
            .is_none());
        delta
            .sample(&lan, counters(600), 23000, reason, 23000)
            .unwrap();
        assert!(delta
            .sample(&wifi, counters(1000), 24000, reason, 24000)
            .is_none());
        delta
            .sample(&wifi, counters(1100), 25000, reason, 25000)
            .unwrap();
        assert_eq!(traffic.total("lan").unwrap().received_bytes, "200");
        assert_eq!(traffic.total("wifi").unwrap().received_bytes, "100");
        traffic.flush(true).unwrap();
        drop(delta);
        drop(traffic);
        let traffic = Arc::new(NetworkTraffic::open(path).unwrap());
        let mut delta = NetworkDelta::default();
        delta.set_traffic(traffic.clone());
        assert!(delta
            .sample(&lan, counters(50000), 1000, reason, 30000)
            .is_none());
        let value = delta
            .sample(&lan, counters(50001), 2000, reason, 31000)
            .unwrap();
        assert_eq!(value.received_bytes_since_start, 1);
        assert_eq!(value.cumulative.unwrap().received_bytes, "201");
        traffic.reset("lan", 1000, 31500).unwrap();
        let value = delta
            .sample(&lan, counters(50002), 3000, reason, 32000)
            .unwrap();
        assert_eq!(value.received_bytes_since_start, 2);
        assert_eq!(value.cumulative.unwrap().received_bytes, "0");
        let value = delta
            .sample(&lan, counters(50003), 4000, reason, 33000)
            .unwrap();
        assert_eq!(value.cumulative.unwrap().received_bytes, "1");
    }

    #[test]
    fn session_totals_exclude_pauses_rollbacks_and_changes_without_losing_link_metadata() {
        let lan = interface("lan", true, Some(1)).interface;
        let wifi = interface("wifi", true, Some(2)).interface;
        let mut delta = NetworkDelta::default();
        let counters = |received, transmitted| NetworkCounters {
            received,
            transmitted,
            receive_link_bps: Some(1_000_000_000),
            transmit_link_bps: None,
        };
        let mut sample = |interface: &NetworkInterface, rx, tx, time| {
            delta.sample(
                interface,
                counters(rx, tx),
                time,
                NetworkSelectionReason::Manual,
                1000,
            )
        };
        assert!(sample(&lan, 1000, 2000, 0).is_none());
        let first = sample(&lan, 1100, 2300, 1000).unwrap();
        assert_eq!(
            (
                first.received_bytes_since_start,
                first.transmitted_bytes_since_start
            ),
            (100, 300)
        );
        assert_eq!(first.receive_link_bps, Some(1_000_000_000));
        assert_eq!(first.transmit_link_bps, None);
        assert!(sample(&lan, 2000, 3000, 10_000).is_none());
        let resumed = sample(&lan, 2010, 3020, 11_000).unwrap();
        assert_eq!(
            (
                resumed.received_bytes_since_start,
                resumed.transmitted_bytes_since_start
            ),
            (110, 320)
        );
        assert!(sample(&lan, 0, 0, 12_000).is_none());
        assert_eq!(
            sample(&lan, 10, 10, 13_000)
                .unwrap()
                .received_bytes_since_start,
            120
        );
        assert!(sample(&wifi, 8000, 8000, 14_000).is_none());
        assert_eq!(
            sample(&wifi, 8010, 8010, 15_000)
                .unwrap()
                .received_bytes_since_start,
            10
        );
        delta.reset();
        assert!(delta
            .sample(
                &wifi,
                counters(9000, 9000),
                16_000,
                NetworkSelectionReason::Manual,
                1000
            )
            .is_none());
        assert_eq!(
            delta
                .sample(
                    &wifi,
                    counters(9010, 9010),
                    17_000,
                    NetworkSelectionReason::Manual,
                    1000
                )
                .unwrap()
                .received_bytes_since_start,
            20
        );
    }
    use mangodisk_platform::system_resources::network::InterfaceKind;

    fn interface(id: &str, physical: bool, metric: Option<u32>) -> InterfaceSample {
        InterfaceSample {
            interface: NetworkInterface {
                details: None,
                id: id.into(),
                name: id.into(),
                kind: InterfaceKind::Ethernet,
                physical,
                connected: true,
                default_route_metric: metric,
            },
            counters: None,
        }
    }

    #[test]
    fn selector_hides_internal_and_inactive_links_but_keeps_saved_identity() {
        let mut idle = interface("inactive", true, None);
        idle.interface.connected = false;
        let mut other = interface("internal", true, None);
        other.interface.kind = InterfaceKind::Other;
        let rows = [
            interface("vpn", false, Some(0)),
            interface("lan", true, Some(10)),
            interface("wifi", true, Some(20)),
            idle,
            other,
        ];
        assert_eq!(
            catalogue(&rows, None)
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            ["lan", "wifi"]
        );
        assert_eq!(
            catalogue(&rows, Some("inactive")).last().unwrap().id,
            "inactive"
        );
        assert_eq!(catalogue(&rows, Some("vpn")).first().unwrap().id, "vpn");
        assert_eq!(catalogue(&rows, Some("missing")).len(), 2);
        assert_eq!(
            select(&rows, Some("vpn"), None).0.unwrap().interface.id,
            "vpn"
        );
    }

    #[test]
    fn automatic_selection_avoids_virtual_double_counting_and_is_stable() {
        let rows = [
            interface("vpn", false, Some(0)),
            interface("lan", true, Some(10)),
            interface("wifi", true, Some(20)),
        ];
        assert_eq!(select(&rows, None, None).0.unwrap().interface.id, "lan");
        assert_eq!(
            select(&rows, Some("vpn"), None).0.unwrap().interface.id,
            "vpn"
        );
        assert!(select(&rows, Some("missing"), None).0.is_none());
        let fallback = [interface("a", true, None), interface("b", true, None)];
        assert_eq!(
            select(&fallback, None, Some("b")).0.unwrap().interface.id,
            "b"
        );
        assert_eq!(
            select(&fallback, None, None).1,
            NetworkSelectionReason::PhysicalFallback
        );
    }

    #[test]
    fn wired_default_route_survives_wifi_disconnect_and_reconnection() {
        let lan = interface("en7", true, Some(0));
        let mut wifi = interface("en0", true, Some(1));
        wifi.interface.kind = InterfaceKind::Wifi;
        let mut rows = [wifi, lan];
        for connected in [true, false, true] {
            rows[0].interface.connected = connected;
            let (selected, reason) = select(&rows, None, Some("en0"));
            assert_eq!(selected.unwrap().interface.id, "en7");
            assert_eq!(reason, NetworkSelectionReason::DefaultRoute);
            let catalogue = catalogue(&rows, None);
            assert_eq!(catalogue[0].id, "en7");
            assert_eq!(catalogue.len(), if connected { 2 } else { 1 });
        }
    }

    #[test]
    fn rates_use_actual_elapsed_time_in_both_directions() {
        let interface = interface("lan", true, None).interface;
        let mut delta = NetworkDelta::default();
        let reason = NetworkSelectionReason::Manual;
        assert!(delta
            .sample(
                &interface,
                NetworkCounters {
                    received: 100,
                    transmitted: 200,
                    ..Default::default()
                },
                0,
                reason,
                1000
            )
            .is_none());
        let value = delta
            .sample(
                &interface,
                NetworkCounters {
                    received: 3100,
                    transmitted: 6200,
                    ..Default::default()
                },
                1500,
                reason,
                1000,
            )
            .unwrap();
        assert_eq!(value.received_bytes_per_second, 2000.0);
        assert_eq!(value.transmitted_bytes_per_second, 4000.0);
        let idle = delta
            .sample(
                &interface,
                NetworkCounters {
                    received: 3100,
                    transmitted: 6200,
                    ..Default::default()
                },
                2500,
                reason,
                1000,
            )
            .unwrap();
        assert_eq!(idle.received_bytes_per_second, 0.0);
    }

    #[test]
    fn resets_interface_changes_and_sleep_do_not_make_peaks() {
        let one = interface("one", true, None).interface;
        let two = interface("two", true, None).interface;
        let mut delta = NetworkDelta::default();
        let reason = NetworkSelectionReason::Manual;
        let counters = NetworkCounters {
            received: 100,
            transmitted: 100,
            ..Default::default()
        };
        assert!(delta.sample(&one, counters, 0, reason, 1000).is_none());
        assert!(delta.sample(&two, counters, 1000, reason, 1000).is_none());
        assert!(delta
            .sample(
                &two,
                NetworkCounters {
                    received: 1,
                    transmitted: 1,
                    ..Default::default()
                },
                2000,
                reason,
                1000
            )
            .is_none());
        assert!(delta.sample(&two, counters, 60000, reason, 1000).is_none());
        assert!(delta.sample(&two, counters, 61000, reason, 1000).is_some());
        delta.reset();
        assert!(delta.sample(&two, counters, 62000, reason, 1000).is_none());
    }
}
