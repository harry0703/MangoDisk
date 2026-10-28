//! Public Mac17,5 firmware and residency captures, independent of the test host.
use super::*;

// Raw pmgr bytes from Stats #3625, macOS 26.6.2:
// https://github.com/exelban/stats/issues/3625#issuecomment-5734902758
const EFFICIENCY_TABLE: &str = "002ca330160300000005f3431b03000000de42571b03000000b7926a250300000090e27d3e030000004e7b9061030000";
const PERFORMANCE_TABLE: &str = "008a582c1603000000131a4b16030000003d8f601b0300000016df7320030000004de4822a03000000ba57933903000000a037a06103000000a1cead7503000000a265bb930300000052d7c6a703000000ccdad0b6030000001070d9cf0300000003e0dfde03000000f64fe6e8030000007de3e9f203000000e9bfecf2030000008b0af1f2030000";

fn bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).unwrap())
        .collect()
}

fn layout() -> FrequencyLayout {
    FrequencyLayout::identify("Apple A18 Pro").expect("Neo has an identified frequency layout")
}

#[test]
fn a18_pro_firmware_tables_use_hz_and_two_core_classes() {
    let layout = layout();
    assert_eq!(layout, FrequencyLayout::A18Pro);
    assert_eq!(layout.subgroup(), "CPU Complex Performance States");
    assert!(!layout.uses_cluster_metadata());
    assert_eq!(
        layout.table_keys(None),
        Some([[Some(1), None], [Some(5), None], [None, None]])
    );
    assert_eq!(
        decode_table(&bytes(EFFICIENCY_TABLE), layout.divisor()).unwrap(),
        [816.0, 1140.0, 1464.0, 1788.0, 2112.0, 2424.0]
    );
    assert_eq!(
        decode_table(&bytes(PERFORMANCE_TABLE), layout.divisor()).unwrap(),
        [
            744.0, 1260.0, 1620.0, 1944.0, 2196.0, 2472.0, 2688.0, 2916.0, 3144.0, 3336.0, 3504.0,
            3648.0, 3756.0, 3864.0, 3924.0, 3972.0, 4044.0,
        ]
    );
    for table in [EFFICIENCY_TABLE, PERFORMANCE_TABLE] {
        assert!(
            decode_table(&bytes(table), 1000.0).is_none(),
            "kHz would inflate MHz by 1000"
        );
        assert!(decode_table(&bytes(table)[..bytes(table).len() - 1], layout.divisor()).is_none());
    }
    for model in [
        "Apple A17 Pro",
        "Apple A18",
        "Apple A18 Pro Max",
        "Apple A19 Pro",
        "Apple A180 Pro",
    ] {
        assert_eq!(FrequencyLayout::identify(model), None, "{model}");
    }
}

#[test]
fn a18_pro_captured_residency_produces_active_clocks_without_a_super_class() {
    // Complex channels from https://github.com/vladkens/macmon/issues/57,
    // macOS 26.5. ECPM/PCPM counters describe other power domains, not CPU clocks.
    struct Capture {
        channel: &'static str,
        table: &'static str,
        states: &'static [(&'static str, i64)],
        expected_mhz: f64,
    }
    let captures = [
        Capture {
            channel: "ECPU",
            table: EFFICIENCY_TABLE,
            states: &[
                ("IDLE", 1415260),
                ("V0P5", 471620),
                ("V1P4", 95028),
                ("V2P3", 39330),
                ("V3P2", 0),
                ("V4P1", 519897),
                ("V5P0", 1754),
            ],
            expected_mhz: 1465.9317204506092,
        },
        Capture {
            channel: "PCPU",
            table: PERFORMANCE_TABLE,
            states: &[
                ("IDLE", 2303439),
                ("V0P16", 29004),
                ("V1P15", 0),
                ("V2P14", 182705),
                ("V3P13", 1597),
                ("V4P12", 0),
                ("V5P11", 0),
                ("V6P10", 0),
                ("V7P9", 0),
                ("V8P8", 0),
                ("V9P7", 0),
                ("V10P6", 0),
                ("V11P5", 0),
                ("V12P4", 0),
                ("V13P3", 0),
                ("V14P2", 0),
                ("V15P1", 0),
                ("V16P0", 26237),
            ],
            expected_mhz: 1781.5927495272247,
        },
    ];
    let mut values: [Vec<f64>; 3] = Default::default();
    let mut activity: [Vec<Option<[f64; 4]>>; 3] = Default::default();
    let expected = [vec!["ECPU".into()], vec!["PCPU".into()], vec![]];
    for capture in captures {
        let kind = layout().cluster(capture.channel).unwrap() as usize;
        let frequencies = decode_table(&bytes(capture.table), layout().divisor()).unwrap();
        let mut states: Vec<_> = capture
            .states
            .iter()
            .map(|(name, ticks)| ((*name).into(), *ticks))
            .collect();
        let value = active_average(&states, &frequencies).unwrap().unwrap();
        assert!((value - capture.expected_mhz).abs() < 0.000001);
        values[kind].push(value);
        activity[kind].push(activity_ratios(&states));
        for (name, ticks) in &mut states {
            if name != "IDLE" {
                *ticks = 0;
            }
        }
        assert_eq!(active_average(&states, &frequencies), Ok(None));
        assert_eq!(activity_ratios(&states), Some([0.0, 1.0, 0.0, 0.0]));
        states.pop();
        assert_eq!(
            active_average(&states, &frequencies),
            Err("state_table_count_mismatch")
        );
    }
    let sample = complete_sample(values, activity, &expected, &expected);
    assert!(sample.average_mhz.is_none());
    assert!(sample.efficiency_mhz.is_some());
    assert!(sample.performance_mhz.is_some());
    assert!(sample.super_mhz.is_none());
    assert_eq!(
        sample
            .activity
            .iter()
            .map(|value| value.class)
            .collect::<Vec<_>>(),
        [
            CpuFrequencyClass::Efficiency,
            CpuFrequencyClass::Performance
        ]
    );
    for name in [
        "ECPM",
        "PCPM",
        "PCPM_IDLE",
        "ECPU0_SRAM",
        "MCPU",
        "MCPU0",
        "PACC0_MCPU0",
    ] {
        assert_eq!(layout().cluster(name), None, "{name}");
    }
}
