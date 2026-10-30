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
    let (chosen, reason) = select(&rows, Some("missing"), None);
    assert_eq!(chosen.unwrap().interface.id, "lan");
    assert_eq!(reason, NetworkSelectionReason::DefaultRoute);
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

#[derive(Default)]
struct SelectionSequence {
    policy: NetworkSelection,
    current: Option<String>,
}
impl SelectionSequence {
    fn sample(
        &mut self,
        rows: &[InterfaceSample],
        manual: Option<&str>,
        time: u64,
    ) -> Option<String> {
        let (selected, _) = self
            .policy
            .select(rows, manual, self.current.as_deref(), time);
        let id = selected.map(|sample| sample.interface.id.clone());
        if id.is_some() {
            self.current.clone_from(&id);
        }
        id
    }
}

#[test]
fn manual_disconnect_and_restore_require_confirmation_but_do_not_erase_preference() {
    let mut rows = [
        interface("lan", true, Some(2)),
        interface("wifi", true, Some(1)),
    ];
    let mut sequence = SelectionSequence::default();
    let manual = Some("lan");
    assert_eq!(sequence.sample(&rows, manual, 0).as_deref(), Some("lan"));
    rows[0].interface.connected = false;
    assert_eq!(sequence.sample(&rows, manual, 1000), None);
    assert_eq!(sequence.sample(&rows, manual, 1500), None);
    assert_eq!(
        sequence.sample(&rows, manual, 2000).as_deref(),
        Some("wifi")
    );
    rows[0].interface.connected = true;
    assert_eq!(
        sequence.sample(&rows, manual, 3000).as_deref(),
        Some("wifi")
    );
    assert_eq!(sequence.sample(&rows, manual, 4000).as_deref(), Some("lan"));
    assert_eq!(sequence.policy.manual.as_deref(), manual);
}

#[test]
fn brief_link_flaps_and_failed_enumerations_do_not_confirm_a_switch() {
    let mut rows = [
        interface("lan", true, Some(1)),
        interface("wifi", true, Some(2)),
    ];
    let mut sequence = SelectionSequence::default();
    sequence.sample(&rows, Some("lan"), 0);
    rows[0].interface.connected = false;
    assert_eq!(sequence.sample(&rows, Some("lan"), 1000), None);
    rows[0].interface.connected = true;
    assert_eq!(
        sequence.sample(&rows, Some("lan"), 2000).as_deref(),
        Some("lan")
    );
    rows[0].interface.connected = false;
    assert_eq!(sequence.sample(&rows, Some("lan"), 3000), None);
    sequence.policy.interrupt();
    assert_eq!(sequence.sample(&rows, Some("lan"), 4000), None);
    assert_eq!(
        sequence.sample(&rows, Some("lan"), 5000).as_deref(),
        Some("wifi")
    );
    rows[0].interface.connected = true;
    assert_eq!(
        sequence.sample(&rows, Some("lan"), 6000).as_deref(),
        Some("wifi")
    );
    rows[0].interface.connected = false;
    assert_eq!(
        sequence.sample(&rows, Some("lan"), 7000).as_deref(),
        Some("wifi")
    );
    rows[0].interface.connected = true;
    assert_eq!(
        sequence.sample(&rows, Some("lan"), 8000).as_deref(),
        Some("wifi")
    );
    assert_eq!(
        sequence.sample(&rows, Some("lan"), 9000).as_deref(),
        Some("lan")
    );
}

#[test]
fn explicit_preferences_take_effect_immediately_and_keep_virtual_manual_support() {
    let rows = [
        interface("lan", true, Some(1)),
        interface("vpn", false, Some(0)),
    ];
    let mut sequence = SelectionSequence::default();
    assert_eq!(sequence.sample(&rows, None, 0).as_deref(), Some("lan"));
    assert_eq!(
        sequence.sample(&rows, Some("vpn"), 100).as_deref(),
        Some("vpn")
    );
    assert_eq!(sequence.sample(&rows, None, 200).as_deref(), Some("lan"));
    assert_eq!(
        sequence.sample(&rows, Some("missing"), 300).as_deref(),
        Some("lan")
    );
    let only_virtual = [interface("vpn", false, Some(0))];
    assert_eq!(sequence.sample(&only_virtual, None, 400), None);
    assert_eq!(
        sequence.sample(&only_virtual, Some("vpn"), 500).as_deref(),
        Some("vpn")
    );
}

#[test]
fn automatic_route_changes_are_confirmed_and_pauses_or_clock_reversals_restart_confirmation() {
    let mut rows = [
        interface("lan", true, Some(1)),
        interface("wifi", true, Some(2)),
    ];
    let mut sequence = SelectionSequence::default();
    sequence.sample(&rows, None, 0);
    rows[1].interface.default_route_metric = Some(0);
    assert_eq!(sequence.sample(&rows, None, 1000).as_deref(), Some("lan"));
    assert_eq!(sequence.sample(&rows, None, 20_000).as_deref(), Some("lan"));
    assert_eq!(sequence.sample(&rows, None, 500).as_deref(), Some("lan"));
    assert_eq!(sequence.sample(&rows, None, 1500).as_deref(), Some("wifi"));
}

#[test]
fn no_available_interface_cancels_pending_recovery_and_reappearance_is_confirmed() {
    let mut rows = [interface("wifi", true, Some(1))];
    let mut sequence = SelectionSequence::default();
    assert_eq!(sequence.sample(&[], Some("removed"), 0), None);
    assert_eq!(
        sequence.sample(&rows, Some("removed"), 1000).as_deref(),
        Some("wifi")
    );
    rows[0].interface.connected = false;
    assert_eq!(sequence.sample(&rows, Some("removed"), 2000), None);
    let lan = [interface("lan", true, Some(1))];
    assert_eq!(sequence.sample(&lan, Some("removed"), 3000), None);
    assert_eq!(sequence.sample(&[], Some("removed"), 4000), None);
    assert_eq!(sequence.sample(&lan, Some("removed"), 5000), None);
    assert_eq!(
        sequence.sample(&lan, Some("removed"), 6000).as_deref(),
        Some("lan")
    );
}

#[test]
fn disconnected_manual_link_recovers_data_even_when_available_route_ranks_alternate() {
    let mut rows = [
        interface("manual", true, Some(0)),
        interface("wifi", true, Some(1)),
        interface("ethernet", true, Some(2)),
    ];
    let mut sequence = SelectionSequence::default();
    assert_eq!(
        sequence.sample(&rows, Some("manual"), 0).as_deref(),
        Some("manual")
    );
    rows[0].interface.connected = false;
    for second in 1..=8 {
        rows[1].interface.default_route_metric = Some(if second % 2 == 1 { 1 } else { 2 });
        rows[2].interface.default_route_metric = Some(if second % 2 == 1 { 2 } else { 1 });
        let selected = sequence.sample(&rows, Some("manual"), second * 1000);
        if second == 1 {
            assert!(selected.is_none());
        } else {
            assert_eq!(selected.as_deref(), Some("wifi"));
        }
    }
    // Once routing stabilizes, automatic preference changes still take effect.
    rows[2].interface.default_route_metric = Some(0);
    assert_eq!(
        sequence.sample(&rows, Some("manual"), 9000).as_deref(),
        Some("ethernet")
    );
    rows[0].interface.connected = true;
    assert_eq!(
        sequence.sample(&rows, Some("manual"), 10000).as_deref(),
        Some("ethernet")
    );
    assert_eq!(
        sequence.sample(&rows, Some("manual"), 11000).as_deref(),
        Some("manual")
    );
}

#[test]
fn pending_fallback_cannot_survive_disconnection_or_override_manual_recovery() {
    let mut rows = [
        interface("manual", true, Some(0)),
        interface("wifi", true, Some(1)),
        interface("ethernet", true, Some(2)),
    ];
    let mut sequence = SelectionSequence::default();
    sequence.sample(&rows, Some("manual"), 0);
    rows[0].interface.connected = false;
    assert_eq!(sequence.sample(&rows, Some("manual"), 1000), None);
    rows[1].interface.connected = false;
    assert_eq!(sequence.sample(&rows, Some("manual"), 2000), None);
    assert_eq!(
        sequence.sample(&rows, Some("manual"), 3000).as_deref(),
        Some("ethernet")
    );
    rows[0].interface.connected = true;
    sequence.sample(&rows, Some("manual"), 4000);
    sequence.sample(&rows, Some("manual"), 5000);
    rows[0].interface.connected = false;
    rows[1].interface.connected = true;
    assert_eq!(sequence.sample(&rows, Some("manual"), 6000), None);
    rows[0].interface.connected = true;
    assert_eq!(
        sequence.sample(&rows, Some("manual"), 7000).as_deref(),
        Some("manual")
    );
    assert!(sequence.policy.pending.is_none());
}

#[test]
fn disconnected_fallback_confirmation_restarts_after_pauses_and_clock_reversals() {
    let mut rows = [
        interface("manual", true, Some(0)),
        interface("wifi", true, Some(1)),
        interface("ethernet", true, Some(2)),
    ];
    let mut sequence = SelectionSequence::default();
    sequence.sample(&rows, Some("manual"), 0);
    rows[0].interface.connected = false;
    assert_eq!(sequence.sample(&rows, Some("manual"), 1000), None);
    rows[2].interface.default_route_metric = Some(0);
    assert_eq!(sequence.sample(&rows, Some("manual"), 7000), None);
    assert_eq!(sequence.policy.pending.as_ref().unwrap().id, "ethernet");
    rows[1].interface.default_route_metric = Some(0);
    rows[2].interface.default_route_metric = Some(1);
    assert_eq!(sequence.sample(&rows, Some("manual"), 500), None);
    assert_eq!(sequence.policy.pending.as_ref().unwrap().id, "wifi");
    assert_eq!(
        sequence.sample(&rows, Some("manual"), 1500).as_deref(),
        Some("wifi")
    );
}
