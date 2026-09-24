use std::collections::BTreeSet;

use super::{unavailable, DeviceCounters};

#[derive(Default)]
pub struct DiskIoReader {
    _private: (),
}

impl DiskIoReader {
    pub fn read(&mut self) -> crate::PlatformResult<Vec<DeviceCounters>> {
        let devices = std::fs::read_dir("/sys/block")
            .map_err(|_| unavailable())?
            .filter_map(Result::ok)
            // Whole physical devices already include their partitions. Pseudo
            // devices such as Snap loop mounts and mapped devices would either
            // add unrelated traffic or double-count their physical backing disk.
            .filter(|entry| entry.path().join("device").exists())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect::<BTreeSet<_>>();
        let content = std::fs::read_to_string("/proc/diskstats").map_err(|_| unavailable())?;
        let mut counters = Vec::new();
        for line in content.lines() {
            if let Some(counter) = parse_counter(line, &devices)? {
                counters.push(counter);
            }
        }
        if counters.is_empty() {
            return Err(unavailable());
        }
        counters.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(counters)
    }
}

fn parse_counter(
    line: &str,
    devices: &BTreeSet<String>,
) -> crate::PlatformResult<Option<DeviceCounters>> {
    let mut fields = line.split_whitespace();
    let Some(name) = fields.nth(2) else {
        return Ok(None);
    };
    if !devices.contains(name) {
        return Ok(None);
    }
    let read_sectors = fields
        .nth(2)
        .ok_or_else(unavailable)?
        .parse::<u64>()
        .map_err(|_| unavailable())?;
    let written_sectors = fields
        .nth(3)
        .ok_or_else(unavailable)?
        .parse::<u64>()
        .map_err(|_| unavailable())?;
    Ok(Some(DeviceCounters {
        id: name.to_string(),
        read_bytes: read_sectors.saturating_mul(512),
        written_bytes: written_sectors.saturating_mul(512),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diskstats_parser_reads_sector_fields_without_counting_other_devices() {
        let devices = BTreeSet::from(["sda".to_string()]);
        let line = "8 0 sda 12 3 100 4 20 5 300 6 0 7 8 9 10 11 12";
        let counter = parse_counter(line, &devices).unwrap().unwrap();
        assert_eq!(counter.id, "sda");
        assert_eq!(counter.read_bytes, 51_200);
        assert_eq!(counter.written_bytes, 153_600);
        assert!(parse_counter("7 0 loop0 1 2 3 4 5 6 7", &devices)
            .unwrap()
            .is_none());
    }
}
