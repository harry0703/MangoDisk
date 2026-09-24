//! Local interface counters and identities, without packets, addresses, or traffic probes.

use serde::Serialize;

#[cfg(target_os = "macos")]
#[path = "network/macos.rs"]
mod native;
#[cfg(windows)]
#[path = "network/windows.rs"]
mod native;
#[cfg(target_os = "linux")]
#[path = "network/linux.rs"]
mod native;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InterfaceKind {
    Ethernet,
    Wifi,
    Virtual,
    Other,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkInterface {
    /// OS identity for local preferences only; never include in diagnostics.
    pub id: String,
    pub name: String,
    pub kind: InterfaceKind,
    pub connected: bool,
    pub physical: bool,
    pub default_route_metric: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
pub struct NetworkCounters {
    pub received: u64,
    pub transmitted: u64,
}

#[derive(Debug, Clone)]
pub struct InterfaceSample {
    pub interface: NetworkInterface,
    pub counters: Option<NetworkCounters>,
}

/// A worker-owned sampler retains slow-changing interface metadata between ticks.
#[derive(Default)]
pub struct NetworkReader {
    #[cfg(target_os = "macos")]
    native: native::Reader,
    #[cfg(target_os = "linux")]
    native: native::Reader,
}
impl NetworkReader {
    pub fn read(&mut self) -> crate::PlatformResult<Vec<InterfaceSample>> {
        #[cfg(target_os = "macos")]
        {
            self.native.read()
        }
        #[cfg(windows)]
        {
            native::read()
        }
        #[cfg(target_os = "linux")]
        {
            self.native.read()
        }
        #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
        {
            Err(crate::PlatformError::new(
                crate::PlatformErrorCode::Unsupported,
                "network sampling unsupported",
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    fn native_interfaces_have_unique_identities_and_readable_active_counters() {
        let rows = super::NetworkReader::default()
            .read()
            .expect("native interfaces must be readable");
        let mut identities = std::collections::HashSet::new();
        for row in &rows {
            assert!(!row.interface.id.is_empty());
            assert!(identities.insert(&row.interface.id));
            if row.interface.connected && row.interface.physical {
                assert!(
                    row.counters.is_some(),
                    "connected physical interface needs counters"
                );
            }
        }
    }
}
