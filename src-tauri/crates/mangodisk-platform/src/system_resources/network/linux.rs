use std::{collections::HashMap, path::Path};

use super::{InterfaceKind, InterfaceSample, NetworkCounters, NetworkInterface};
use crate::{PlatformError, PlatformErrorCode, PlatformResult};

#[derive(Default)]
pub struct Reader;

impl Reader {
    pub fn read(&mut self) -> PlatformResult<Vec<InterfaceSample>> {
        let routes = default_routes();
        let mut samples = Vec::new();
        for entry in std::fs::read_dir("/sys/class/net")
            .map_err(|error| failure("list interfaces", error))?
        {
            let entry = entry.map_err(|error| failure("read interface", error))?;
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            let root = entry.path();
            let kind = interface_kind(&root, &name);
            let physical = matches!(kind, InterfaceKind::Ethernet | InterfaceKind::Wifi);
            let connected = read_trimmed(root.join("operstate"))
                .is_some_and(|state| matches!(state.as_str(), "up" | "unknown"));
            let counters = if connected && physical {
                read_counter(root.join("statistics/rx_bytes"))
                    .zip(read_counter(root.join("statistics/tx_bytes")))
                    .map(|(received, transmitted)| NetworkCounters {
                        received,
                        transmitted,
                    })
            } else {
                None
            };
            samples.push(InterfaceSample {
                interface: NetworkInterface {
                    id: name.clone(),
                    name: name.clone(),
                    kind,
                    connected,
                    physical,
                    default_route_metric: routes.get(&name).copied(),
                },
                counters,
            });
        }
        samples.sort_by(|left, right| left.interface.id.cmp(&right.interface.id));
        if samples.is_empty() {
            return Err(unavailable());
        }
        Ok(samples)
    }
}

fn interface_kind(root: &Path, name: &str) -> InterfaceKind {
    if root.join("wireless").exists() {
        InterfaceKind::Wifi
    } else if root.join("device").exists() {
        InterfaceKind::Ethernet
    } else if name == "lo"
        || std::fs::canonicalize(root)
            .ok()
            .is_some_and(|path| path.starts_with("/sys/devices/virtual/net"))
    {
        InterfaceKind::Virtual
    } else {
        InterfaceKind::Other
    }
}

fn default_routes() -> HashMap<String, u32> {
    let Ok(content) = std::fs::read_to_string("/proc/net/route") else {
        return HashMap::new();
    };
    let mut routes = HashMap::new();
    for line in content.lines().skip(1) {
        let Some((interface, metric)) = parse_default_route(line) else {
            continue;
        };
        routes
            .entry(interface.to_string())
            .and_modify(|current: &mut u32| *current = (*current).min(metric))
            .or_insert(metric);
    }
    routes
}

fn parse_default_route(line: &str) -> Option<(&str, u32)> {
    let mut fields = line.split_whitespace();
    let interface = fields.next()?;
    let destination = fields.next()?;
    fields.next()?;
    let flags = u32::from_str_radix(fields.next()?, 16).ok()?;
    fields.next()?;
    fields.next()?;
    let metric = fields.next()?.parse::<u32>().ok()?;
    let mask = fields.next()?;
    (destination == "00000000" && mask == "00000000" && flags & 1 != 0)
        .then_some((interface, metric))
}

fn read_counter(path: impl AsRef<Path>) -> Option<u64> {
    read_trimmed(path).and_then(|value| value.parse().ok())
}

fn read_trimmed(path: impl AsRef<Path>) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_string())
}

fn failure(action: &str, error: std::io::Error) -> PlatformError {
    PlatformError::new(
        PlatformErrorCode::OperationFailed,
        format!("{action} failed: {error}"),
    )
}

fn unavailable() -> PlatformError {
    PlatformError::new(
        PlatformErrorCode::OperationFailed,
        "Linux network interfaces are unavailable",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_parser_requires_an_active_default_route() {
        assert_eq!(
            parse_default_route("enp0s3 00000000 0102A8C0 0003 0 0 100 00000000 0 0 0"),
            Some(("enp0s3", 100))
        );
        assert_eq!(
            parse_default_route("enp0s3 0002A8C0 00000000 0001 0 0 0 00FFFFFF 0 0 0"),
            None
        );
        assert_eq!(
            parse_default_route("enp0s3 00000000 0102A8C0 0000 0 0 100 00000000 0 0 0"),
            None
        );
    }
}
