//! Validated frequency units, table identities and IOReport state ordering.
use super::super::valid_mhz;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CoreKind {
    Efficiency = 0,
    Performance = 1,
    Super = 2,
}

pub(super) const KINDS: [CoreKind; 3] =
    [CoreKind::Efficiency, CoreKind::Performance, CoreKind::Super];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FrequencyLayout {
    M(u32),
    A18Pro,
}

impl FrequencyLayout {
    pub(super) fn identify(model: &str) -> Option<Self> {
        match model {
            "Apple A18 Pro" => Some(Self::A18Pro),
            _ => generation(model).map(Self::M),
        }
    }

    pub(super) fn divisor(self) -> f64 {
        match self {
            Self::M(generation) => divisor(generation),
            // macmon #57 and Stats #3625 contain A18 Pro firmware dumps: Hz,
            // despite the chip sharing a silicon generation with the kHz-based M4.
            Self::A18Pro => 1_000_000.0,
        }
    }

    pub(super) fn table_keys(self, clusters: Option<&[u8]>) -> Option<[[Option<u8>; 2]; 3]> {
        match self {
            Self::M(generation) => table_keys(generation, clusters),
            Self::A18Pro => Some([[Some(1), None], [Some(5), None], [None, None]]),
        }
    }

    pub(super) fn uses_cluster_metadata(self) -> bool {
        matches!(self, Self::M(5..))
    }

    pub(super) fn subgroup(self) -> &'static str {
        if self.uses_cluster_metadata() {
            "CPU Core Performance States"
        } else {
            "CPU Complex Performance States"
        }
    }

    pub(super) fn cluster(self, name: &str) -> Option<CoreKind> {
        match self {
            Self::M(generation) => cluster(name, generation),
            // Apple specifies four efficiency and two performance cores on Neo.
            // Its ECPU/PCPU channels must never be labelled as M5-style Super.
            Self::A18Pro => classify_channel(name, false, false),
        }
    }

    pub(super) fn complete_tables(self, tables: &mut [Vec<f64>; 3]) -> bool {
        match self {
            Self::M(generation) => complete_tables(generation, tables),
            Self::A18Pro => false,
        }
    }
}

fn generation(model: &str) -> Option<u32> {
    let model = model.strip_prefix("Apple M")?;
    let (number, variant) = model.split_once(' ').unwrap_or((model, ""));
    if !matches!(variant, "" | "Pro" | "Max" | "Ultra") {
        return None;
    }
    match number {
        "1" => Some(1),
        "2" => Some(2),
        "3" => Some(3),
        "4" => Some(4),
        "5" => Some(5),
        "6" => Some(6),
        _ => None,
    }
}

fn divisor(generation: u32) -> f64 {
    if generation <= 3 {
        1_000_000.0
    } else {
        1000.0
    }
}

pub(super) fn decode_table(bytes: &[u8], divisor: f64) -> Option<Vec<f64>> {
    if bytes.is_empty() || bytes.len() > 512 || !bytes.len().is_multiple_of(8) {
        return None;
    }
    let values: Option<Vec<f64>> = bytes
        .as_chunks::<8>()
        .0
        .iter()
        .map(|chunk| {
            let mhz = f64::from(u32::from_le_bytes(chunk[..4].try_into().ok()?)) / divisor;
            valid_mhz(mhz).filter(|value| *value >= 100.0)
        })
        .collect();
    // Firmware orders states by voltage/performance ordinal, not by frequency.
    // Base M5 boost states include a 4428 -> 4416 MHz dip; sorting or rejecting
    // that table would lose the validated IOReport state-to-frequency mapping.
    values
}

fn table_keys(generation: u32, clusters: Option<&[u8]>) -> Option<[[Option<u8>; 2]; 3]> {
    // Pinned layouts: Stats ea74107c SystemKit.getFrequencies and macmon 8b2ab564
    // parse_acc_clusters. M5 Max exposes performance states at slot 23, not 22.
    let mut keys = if generation <= 4 {
        [[Some(1), None], [Some(5), None], [None, None]]
    } else {
        [[Some(1), None], [Some(22), None], [Some(5), None]]
    };
    if let Some(bytes) = clusters {
        if bytes.is_empty()
            || bytes.len() > 512
            || !(bytes.len().is_multiple_of(8) || generation == 6 && bytes.len().is_multiple_of(28))
        {
            return None;
        }
        // M6 pmgr-child records append flags and a 16-byte complex name to
        // the compact header. Interpreting the name as more 8-byte records
        // rejects valid firmware (or could select an unrelated voltage table).
        let stride = if generation == 6 && bytes.len().is_multiple_of(28) {
            28
        } else {
            8
        };
        let mut seen = [false; 3];
        for entry in bytes.chunks_exact(stride) {
            if stride == 28 {
                let end = entry[12..].iter().position(|byte| *byte == 0)? + 12;
                let name = std::str::from_utf8(&entry[12..end]).ok()?;
                let matches_kind = match entry[1] {
                    0 => name == "EACC",
                    1 | 2 => name.strip_prefix("PACC").is_some_and(|id| {
                        !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit())
                    }),
                    _ => false,
                };
                if !matches_kind || entry[end..].iter().any(|byte| *byte != 0) {
                    return None;
                }
            }
            let kind = usize::from(entry[1]);
            if kind >= 3 || entry[0] == 0 || seen[kind] {
                return None;
            }
            seen[kind] = true;
            let kind = if generation <= 4 && kind == 2 {
                1
            } else {
                kind
            };
            if keys[kind][0] != Some(entry[0]) {
                // Base M5 metadata can name slot 2 while the live ECPU table stays at 1.
                // M5 Max MCPU metadata identifies slot 23; its legacy slot 22 can instead
                // describe an unused tier, so the explicit MCPU identity takes precedence.
                if generation == 5 && kind == CoreKind::Performance as usize {
                    keys[kind] = [Some(entry[0]), keys[kind][0]];
                } else {
                    keys[kind][1] = Some(entry[0]);
                }
            }
        }
        if generation == 5 && seen == [true, true, false] && keys[1][0] == Some(5) {
            // Base M5 keeps the legacy PCPU record kind (1), but macOS names
            // that tier Super. Its slot 5 must not also advertise an MCPU tier.
            keys[CoreKind::Performance as usize] = [None, None];
        }
    }
    Some(keys)
}

fn cluster(name: &str, generation: u32) -> Option<CoreKind> {
    classify_channel(name, generation >= 5, true)
}

fn classify_channel(name: &str, pcpu_is_super: bool, has_mcpu: bool) -> Option<CoreKind> {
    let core = if let Some((complex, core)) = name.split_once('_') {
        let valid_complex = complex == "EACC"
            || complex
                .strip_prefix("PACC")
                .is_some_and(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()));
        if !valid_complex {
            return None;
        }
        core
    } else {
        name
    };
    for (prefix, kind) in [
        ("ECPU", CoreKind::Efficiency),
        ("MCPU", CoreKind::Performance),
        (
            "PCPU",
            if pcpu_is_super {
                CoreKind::Super
            } else {
                CoreKind::Performance
            },
        ),
    ] {
        if prefix == "MCPU" && !has_mcpu {
            continue;
        }
        if let Some(suffix) = core.strip_prefix(prefix) {
            // Complex channels may omit the first cluster's numeric suffix.
            if suffix.is_empty() {
                return Some(kind);
            }
            let suffix = suffix.strip_prefix(' ').unwrap_or(suffix);
            if !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit()) {
                return Some(kind);
            }
        }
    }
    None
}

fn complete_tables(generation: u32, tables: &mut [Vec<f64>; 3]) -> bool {
    // Stats ea74107ca4012312818d31120461776fb9451780 SystemKit.getFrequencies:
    // M6 MCPU and PCPU channels share the PACC table when no separate MCPU table exists.
    if generation == 6 && tables[1].is_empty() && !tables[2].is_empty() {
        tables[1] = tables[2].clone();
        true
    } else {
        false
    }
}

pub(super) fn active_average(
    states: &[(String, i64)],
    frequencies: &[f64],
) -> Result<Option<f64>, &'static str> {
    if frequencies.is_empty() {
        return Err("frequency_table_missing");
    }
    let mut active = Vec::new();
    for (name, residency) in states {
        if *residency < 0 {
            return Err("negative_residency");
        }
        if matches!(name.as_str(), "IDLE" | "DOWN" | "OFF") {
            continue;
        }
        let (voltage, performance) = name
            .strip_prefix('V')
            .and_then(|name| name.split_once('P'))
            .ok_or("unknown_state_name")?;
        let index = voltage
            .parse::<usize>()
            .map_err(|_| "unknown_voltage_ordinal")?;
        let level = performance
            .parse::<usize>()
            .map_err(|_| "unknown_performance_ordinal")?;
        if index != active.len() || Some(level) != frequencies.len().checked_sub(index + 1) {
            return Err("state_table_order_mismatch");
        }
        active.push(*residency as f64);
    }
    if active.len() != frequencies.len() {
        return Err("state_table_count_mismatch");
    }
    let total: f64 = active.iter().sum();
    if total <= 0.0 {
        return Ok(None);
    }
    valid_mhz(
        active
            .iter()
            .zip(frequencies)
            .map(|(time, freq)| time / total * freq)
            .sum(),
    )
    .map(Some)
    .ok_or("frequency_out_of_range")
}

pub(super) fn complete_channel_coverage(expected: &[String], seen: &[String]) -> bool {
    !expected.is_empty()
        && expected.len() == seen.len()
        && expected
            .iter()
            .all(|name| seen.iter().filter(|candidate| *candidate == name).count() == 1)
        && seen.iter().all(|name| {
            expected
                .iter()
                .filter(|candidate| *candidate == name)
                .count()
                == 1
        })
}

/// Keep non-active residency distinct from an empty or incomplete interval.
pub(super) fn activity_ratios(states: &[(String, i64)]) -> Option<[f64; 4]> {
    let mut totals = [0.0; 4];
    for (name, ticks) in states {
        if *ticks < 0 {
            return None;
        }
        let index = match name.as_str() {
            "IDLE" => 1,
            "DOWN" => 2,
            "OFF" => 3,
            _ => 0, // active_average validates every active state's identity first.
        };
        totals[index] += *ticks as f64;
    }
    let total: f64 = totals.iter().sum();
    (total > 0.0).then(|| totals.map(|ticks| ticks / total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn residency_distinguishes_idle_mixed_active_and_invalid_intervals() {
        let sample = |idle, down, off, active| {
            vec![
                ("IDLE".into(), idle),
                ("DOWN".into(), down),
                ("OFF".into(), off),
                ("V0P0".into(), active),
            ]
        };
        assert_eq!(
            activity_ratios(&sample(30, 70, 0, 0)),
            Some([0.0, 0.3, 0.7, 0.0])
        );
        assert_eq!(
            activity_ratios(&sample(20, 30, 10, 40)),
            Some([0.4, 0.2, 0.3, 0.1])
        );
        assert_eq!(activity_ratios(&sample(0, 0, 0, 0)), None);
        assert_eq!(activity_ratios(&sample(-1, 10, 0, 0)), None);
        let expected = vec!["PCPU0".into(), "PCPU1".into()];
        assert!(complete_channel_coverage(&expected, &expected));
        assert!(!complete_channel_coverage(&expected, &expected[..1]));
        assert!(!complete_channel_coverage(
            &expected,
            &["PCPU0".into(), "PCPU0".into()]
        ));
        assert!(!complete_channel_coverage(
            &["PCPU0".into(), "PCPU0".into()],
            &["PCPU0".into(), "PCPU1".into()]
        ));
        assert!(!complete_channel_coverage(&[], &[]));
    }

    #[test]
    fn all_six_generations_decode_their_identified_frequency_units() {
        for number in 1..=6 {
            for variant in ["", " Pro", " Max", " Ultra"] {
                let model = format!("Apple M{number}{variant}");
                assert_eq!(generation(&model), Some(number));
                let layout = FrequencyLayout::identify(&model).unwrap();
                assert_eq!(layout, FrequencyLayout::M(number));
                assert_eq!(layout.divisor(), divisor(number));
                assert_eq!(layout.table_keys(None), table_keys(number, None));
            }
            let divisor = divisor(number);
            let mut bytes = Vec::new();
            for mhz in [1000, 2000] {
                bytes.extend_from_slice(&((f64::from(mhz) * divisor) as u32).to_le_bytes());
                bytes.extend_from_slice(&0u32.to_le_bytes());
            }
            assert_eq!(decode_table(&bytes, divisor), Some(vec![1000.0, 2000.0]));
        }
        for model in [
            "Apple M7",
            "Apple M6 Unknown",
            "Apple M6Pro",
            "Apple M01",
            "VirtualApple",
        ] {
            assert_eq!(generation(model), None);
            assert_eq!(FrequencyLayout::identify(model), None);
        }
        assert_eq!(decode_table(&[0; 8], 1000.0), None);
        assert_eq!(decode_table(&[0; 7], 1000.0), None);
    }

    #[test]
    fn base_m5_boost_table_preserves_state_ordinals_across_frequency_dips() {
        let mhz = [
            1308u32, 1608, 1932, 2220, 2508, 2820, 3072, 3300, 3516, 3720, 3876, 4020, 4176, 4248,
            4272, 4332, 4428, 4416, 4464,
        ];
        let bytes: Vec<u8> = mhz
            .iter()
            .flat_map(|value| {
                let mut entry = [0; 8];
                entry[..4].copy_from_slice(&(value * 1000).to_le_bytes());
                entry
            })
            .collect();
        let frequencies = decode_table(&bytes, divisor(5)).expect("valid M5 boost states");
        assert_eq!(frequencies, mhz.map(f64::from));
        let states: Vec<_> = mhz
            .iter()
            .enumerate()
            .map(|(index, _)| {
                (
                    format!("V{index}P{}", mhz.len() - index - 1),
                    i64::from(index == 17),
                )
            })
            .collect();
        assert_eq!(active_average(&states, &frequencies), Ok(Some(4416.0)));
    }

    #[test]
    fn m5_max_cluster_metadata_selects_its_separate_performance_table() {
        let bytes = [
            22, 0, 0, 0, 0, 0, 0, 0, 23, 1, 0, 0, 0, 0, 0, 0, 5, 2, 0, 0, 0, 0, 0, 0,
        ];
        assert_eq!(
            table_keys(5, Some(&bytes)),
            Some([[Some(1), Some(22)], [Some(23), Some(22)], [Some(5), None]])
        );
        assert!(table_keys(5, Some(&bytes[..23])).is_none());
        assert!(table_keys(5, Some(&[1, 3, 0, 0, 0, 0, 0, 0])).is_none());
        assert!(table_keys(5, Some(&[1, 0, 0, 0, 0, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0])).is_none());
        assert_eq!(
            table_keys(6, None),
            Some([[Some(1), None], [Some(22), None], [Some(5), None]])
        );
    }

    #[test]
    fn m6_named_cluster_records_preserve_identified_table_candidates() {
        // Mac18,5 firmware extends each record with flags and a 16-byte name.
        let mut bytes = Vec::new();
        for (header, name) in [
            ([2u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], b"EACC".as_slice()),
            ([5, 1, 0, 0, 8, 14, 0, 0, 0, 0, 0, 0], b"PACC0".as_slice()),
        ] {
            bytes.extend_from_slice(&header);
            bytes.extend_from_slice(name);
            bytes.resize(bytes.len() + 16 - name.len(), 0);
        }
        assert_eq!(bytes.len(), 56);
        assert_eq!(
            table_keys(6, Some(&bytes)),
            Some([[Some(1), Some(2)], [Some(22), Some(5)], [Some(5), None]])
        );
        assert!(table_keys(5, Some(&bytes)).is_none());
        assert!(table_keys(6, Some(&bytes[..55])).is_none());
        let mut invalid = bytes.clone();
        invalid[12] = b'X';
        assert!(table_keys(6, Some(&invalid)).is_none());
        invalid = bytes.clone();
        invalid[28 + 1] = 0;
        assert!(table_keys(6, Some(&invalid)).is_none());
        invalid = bytes.clone();
        invalid[27] = 1;
        assert!(table_keys(6, Some(&invalid)).is_none());
    }

    #[test]
    fn base_m5_keeps_identified_tables_before_cluster_metadata() {
        // Base M5 MacBook Air reports this metadata while its live ECPU table is slot 1.
        let bytes = [2, 0, 0, 0, 0, 0, 0, 0, 5, 1, 0, 0, 0, 0, 0, 0];
        assert_eq!(table_keys(5, Some(&bytes)).unwrap()[0], [Some(1), Some(2)]);
    }

    #[test]
    fn base_m5_two_tier_metadata_does_not_duplicate_the_super_table() {
        // macOS 26.5.1 reports Super(4) and Efficiency(6) for this Apple M5 Air.
        let bytes = [2, 0, 0, 0, 0, 0, 0, 0, 5, 1, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            table_keys(5, Some(&bytes)),
            Some([[Some(1), Some(2)], [None, None], [Some(5), None]])
        );
    }

    #[test]
    fn channel_identity_preserves_all_three_m6_core_kinds() {
        assert_eq!(cluster("PCPU1", 3), Some(CoreKind::Performance));
        assert_eq!(cluster("EACC_ECPU 0", 6), Some(CoreKind::Efficiency));
        assert_eq!(cluster("PACC0_MCPU2", 6), Some(CoreKind::Performance));
        assert_eq!(cluster("PACC0_PCPU 0", 6), Some(CoreKind::Super));
        for name in ["PCPM_IDLE", "EACC_CPM", "PACC0", "bad_PCPU0", "PCPU-1"] {
            assert_eq!(cluster(name, 6), None);
        }
    }

    #[test]
    fn channel_identity_accepts_numbered_and_unnumbered_cpu_channels() {
        for generation in 1..=6 {
            for (prefix, expected) in [
                ("ECPU", CoreKind::Efficiency),
                ("MCPU", CoreKind::Performance),
                (
                    "PCPU",
                    if generation >= 5 {
                        CoreKind::Super
                    } else {
                        CoreKind::Performance
                    },
                ),
            ] {
                for suffix in ["", "0", "1", "010", " 0", " 12"] {
                    let name = format!("{prefix}{suffix}");
                    assert_eq!(
                        cluster(&name, generation),
                        Some(expected),
                        "{name}, M{generation}"
                    );
                }
                for suffix in [" ", "  0", "-1", "+1", "1x", "１", "_IDLE"] {
                    let name = format!("{prefix}{suffix}");
                    assert_eq!(cluster(&name, generation), None, "{name}, M{generation}");
                }
            }
        }
        for (name, expected) in [
            ("EACC_ECPU", CoreKind::Efficiency),
            ("EACC_ECPU000", CoreKind::Efficiency),
            ("PACC0_MCPU", CoreKind::Performance),
            ("PACC0_MCPU 0", CoreKind::Performance),
            ("PACC1_PCPU", CoreKind::Super),
            ("PACC1_PCPU010", CoreKind::Super),
        ] {
            assert_eq!(cluster(name, 6), Some(expected), "{name}");
        }
    }

    #[test]
    fn shared_m6_table_never_overwrites_a_separate_table_or_applies_to_m5() {
        let mut tables = [vec![1000.0], vec![], vec![2000.0, 4000.0]];
        assert!(!complete_tables(5, &mut tables));
        assert!(tables[1].is_empty());
        assert!(complete_tables(6, &mut tables));
        assert_eq!(tables[1], tables[2]);
        tables[1] = vec![1500.0];
        assert!(!complete_tables(6, &mut tables));
        assert_eq!(tables[1], [1500.0]);
    }

    #[test]
    fn active_frequency_distinguishes_idle_from_invalid_layouts() {
        let mut states = vec![
            ("IDLE".into(), 800),
            ("V0P1".into(), 25),
            ("V1P0".into(), 75),
        ];
        assert_eq!(active_average(&states, &[1000.0, 2000.0]), Ok(Some(1750.0)));
        states[1].1 = 0;
        states[2].1 = 0;
        assert_eq!(active_average(&states, &[1000.0, 2000.0]), Ok(None));
        states[2].0 = "V1P1".into();
        assert_eq!(
            active_average(&states, &[1000.0, 2000.0]),
            Err("state_table_order_mismatch")
        );
        assert_eq!(active_average(&states, &[]), Err("frequency_table_missing"));
    }
}
