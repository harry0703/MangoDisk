use std::{
    collections::HashMap,
    ffi::CStr,
    mem::size_of,
    ptr,
    time::{Duration, Instant},
};

use core_foundation::{
    base::{CFType, TCFType, ToVoid},
    boolean::CFBoolean,
    dictionary::CFDictionary,
    string::CFString,
};
use system_configuration::{
    dynamic_store::{SCDynamicStore, SCDynamicStoreBuilder},
    network_configuration::{get_interfaces, SCNetworkInterfaceType},
};

use super::{
    InterfaceKind, InterfaceSample, NetworkCounters, NetworkInterface, NetworkInterfaceDetails,
};
use crate::{PlatformError, PlatformErrorCode, PlatformResult};

#[path = "macos/wifi.rs"]
mod wifi;

fn addressed_interfaces() -> PlatformResult<HashMap<String, NetworkInterfaceDetails>> {
    let mut list = ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return Err(unavailable());
    }
    struct Addresses(*mut libc::ifaddrs);
    impl Drop for Addresses {
        fn drop(&mut self) {
            unsafe {
                libc::freeifaddrs(self.0);
            }
        }
    }
    let _owned = Addresses(list);
    let mut result = HashMap::<String, NetworkInterfaceDetails>::new();
    let mut current = list;
    let mut count = 0;
    while !current.is_null() {
        count += 1;
        if count > 4096 {
            return Err(unavailable());
        }
        let address = unsafe { &*current };
        if !address.ifa_addr.is_null() && !address.ifa_name.is_null() {
            let name = unsafe { CStr::from_ptr(address.ifa_name) }
                .to_string_lossy()
                .into_owned();
            let details = result.entry(name).or_default();
            let ip = unsafe {
                match i32::from((*address.ifa_addr).sa_family) {
                    libc::AF_INET => Some(std::net::IpAddr::V4(
                        (*(address.ifa_addr.cast::<libc::sockaddr_in>()))
                            .sin_addr
                            .s_addr
                            .to_ne_bytes()
                            .into(),
                    )),
                    libc::AF_INET6 => Some(std::net::IpAddr::V6(
                        (*(address.ifa_addr.cast::<libc::sockaddr_in6>()))
                            .sin6_addr
                            .s6_addr
                            .into(),
                    )),
                    libc::AF_LINK => {
                        let link = address.ifa_addr.cast::<libc::sockaddr_dl>();
                        let data = std::mem::offset_of!(libc::sockaddr_dl, sdl_data);
                        let length = usize::from((*address.ifa_addr).sa_len);
                        if length >= data {
                            let start = data + usize::from((*link).sdl_nlen);
                            let end = start + usize::from((*link).sdl_alen);
                            if (*link).sdl_alen == 6 && end <= length {
                                let bytes = std::slice::from_raw_parts(
                                    address.ifa_addr.cast::<u8>().add(start),
                                    6,
                                );
                                if bytes.iter().any(|byte| *byte != 0) {
                                    details.hardware_address = Some(
                                        bytes
                                            .iter()
                                            .map(|byte| format!("{byte:02x}"))
                                            .collect::<Vec<_>>()
                                            .join(":"),
                                    );
                                }
                            }
                        }
                        None
                    }
                    _ => None,
                }
            };
            if let Some(ip) = ip.filter(|ip| usable_address(*ip)) {
                let ip = ip.to_string();
                if !details.addresses.contains(&ip) {
                    details.addresses.push(ip);
                }
            }
        }
        current = address.ifa_next;
    }
    for details in result.values_mut() {
        details.addresses.sort();
    }
    Ok(result)
}

fn usable_address(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ip) => {
            !ip.is_unspecified() && !ip.is_loopback() && !ip.is_link_local() && !ip.is_multicast()
        }
        std::net::IpAddr::V6(ip) => {
            !ip.is_unspecified()
                && !ip.is_loopback()
                && !ip.is_unicast_link_local()
                && !ip.is_multicast()
        }
    }
}

fn primary_interfaces(store: Option<&SCDynamicStore>) -> Vec<String> {
    // SystemConfiguration exposes local routing state without opening a socket
    // to a public destination. Preserve IPv4-before-IPv6 priority instead of
    // letting interface-name ordering decide between different family primaries.
    // VPN primaries are filtered by physical identity below.
    ["State:/Network/Global/IPv4", "State:/Network/Global/IPv6"]
        .iter()
        .filter_map(|key| {
            let value = store?
                .get(CFString::new(key))?
                .downcast_into::<CFDictionary>()?;
            let key = CFString::new("PrimaryInterface");
            let raw = value.find(key.to_void())?;
            unsafe { CFType::wrap_under_get_rule(*raw) }
                .downcast_into::<CFString>()
                .map(|name| name.to_string())
        })
        .collect()
}

fn link_active(store: Option<&SCDynamicStore>, id: &str) -> Option<bool> {
    let value = store?
        .get(CFString::new(&format!(
            "State:/Network/Interface/{id}/Link"
        )))?
        .downcast_into::<CFDictionary>()?;
    let key = CFString::new("Active");
    let raw = value.find(key.to_void())?;
    unsafe { CFType::wrap_under_get_rule(*raw) }
        .downcast_into::<CFBoolean>()
        .map(bool::from)
}

fn connected_link(flags: i32, active: Option<bool>) -> bool {
    // Driver-provided RUNNING flags can differ from SystemConfiguration's link
    // state. Prefer the native link state, retaining flags when it is unavailable.
    flags & libc::IFF_UP != 0 && active.unwrap_or(flags & libc::IFF_RUNNING != 0)
}

fn primary_route_metric(primary: &[String], id: &str) -> Option<u32> {
    primary
        .iter()
        .position(|name| name == id)
        .map(|rank| rank as u32)
}

#[derive(Default)]
pub struct Reader {
    store: Option<SCDynamicStore>,
    physical: HashMap<String, (InterfaceKind, Option<String>)>,
    topology: Vec<String>,
    refreshed: Option<Instant>,
    wifi: wifi::Reader,
    diagnostic_state: Vec<String>,
}
impl Reader {
    pub fn read(&mut self) -> PlatformResult<Vec<InterfaceSample>> {
        if self.store.is_none() {
            self.store = SCDynamicStoreBuilder::new("MangoDisk resource interfaces").build();
        }
        let primary = primary_interfaces(self.store.as_ref());
        let addressed = addressed_interfaces()?;
        let mut mib = [libc::CTL_NET, libc::PF_ROUTE, 0, 0, libc::NET_RT_IFLIST2, 0];
        let mut length = 0;
        unsafe {
            if libc::sysctl(
                mib.as_mut_ptr(),
                mib.len() as _,
                ptr::null_mut(),
                &mut length,
                ptr::null_mut(),
                0,
            ) != 0
                || length > 16 * 1024 * 1024
            {
                return Err(unavailable());
            }
        }
        let mut bytes = vec![0u8; length];
        unsafe {
            if libc::sysctl(
                mib.as_mut_ptr(),
                mib.len() as _,
                bytes.as_mut_ptr().cast(),
                &mut length,
                ptr::null_mut(),
                0,
            ) != 0
            {
                return Err(unavailable());
            }
        }
        bytes.truncate(length);
        let mut offset = 0;
        let mut result = Vec::new();
        let mut flags = HashMap::new();
        while offset + 4 <= bytes.len() {
            // All route messages share only the first four bytes. Address records
            // are shorter than if_msghdr and must be skipped using their own length.
            let size = usize::from(u16::from_ne_bytes([bytes[offset], bytes[offset + 1]]));
            let message_type = i32::from(bytes[offset + 3]);
            if size < 4 || offset + size > bytes.len() {
                return Err(unavailable());
            }
            if message_type == libc::RTM_IFINFO2 && size >= size_of::<libc::if_msghdr2>() {
                let info = unsafe {
                    ptr::read_unaligned(bytes.as_ptr().add(offset).cast::<libc::if_msghdr2>())
                };
                // IFMIB supplies both the 64-bit counters and interface name.
                // Calling if_indextoname for every interface would repeatedly
                // enumerate the entire interface table on macOS.
                let mut data: libc::ifmibdata = unsafe { std::mem::zeroed() };
                let mut count = size_of::<libc::ifmibdata>();
                let mut query = [
                    libc::CTL_NET,
                    libc::PF_LINK,
                    0,
                    2,
                    i32::from(info.ifm_index),
                    1,
                ];
                let valid = unsafe {
                    libc::sysctl(
                        query.as_mut_ptr(),
                        query.len() as _,
                        (&mut data as *mut libc::ifmibdata).cast(),
                        &mut count,
                        ptr::null_mut(),
                        0,
                    )
                } == 0
                    && count == size_of::<libc::ifmibdata>();
                let mut name = data.ifmd_name;
                if valid
                    || !unsafe {
                        libc::if_indextoname(u32::from(info.ifm_index), name.as_mut_ptr())
                    }
                    .is_null()
                {
                    name[libc::IFNAMSIZ - 1] = 0;
                    let id = unsafe { CStr::from_ptr(name.as_ptr()) }
                        .to_string_lossy()
                        .into_owned();
                    flags.insert(id.clone(), info.ifm_flags);
                    let connected = connected_link(info.ifm_flags, None);
                    result.push(InterfaceSample {
                        interface: NetworkInterface {
                            details: None,
                            name: id.clone(),
                            kind: InterfaceKind::Virtual,
                            connected,
                            physical: false,
                            default_route_metric: primary_route_metric(&primary, &id),
                            id,
                        },
                        counters: valid.then_some(NetworkCounters {
                            received: data.ifmd_data.ifi_ibytes,
                            transmitted: data.ifmd_data.ifi_obytes,
                            receive_link_bps: (data.ifmd_data.ifi_baudrate > 0)
                                .then_some(data.ifmd_data.ifi_baudrate),
                            transmit_link_bps: (data.ifmd_data.ifi_baudrate > 0)
                                .then_some(data.ifmd_data.ifi_baudrate),
                        }),
                    });
                }
            }
            offset += size;
        }
        let topology = result
            .iter()
            .map(|sample| sample.interface.id.clone())
            .collect::<Vec<_>>();
        // IOKit-backed interface discovery is much slower than reading counters.
        // Refresh immediately on topology changes and every ten seconds for renames;
        // link state, routing and byte counters are still read on every tick.
        if self.topology != topology
            || self
                .refreshed
                .is_none_or(|time| time.elapsed() >= Duration::from_secs(10))
        {
            self.physical.clear();
            for interface in get_interfaces().iter() {
                let kind = match interface.interface_type() {
                    Some(SCNetworkInterfaceType::Ethernet) => InterfaceKind::Ethernet,
                    Some(SCNetworkInterfaceType::IEEE80211) => InterfaceKind::Wifi,
                    _ => continue,
                };
                if let Some(name) = interface.bsd_name() {
                    self.physical.insert(
                        name.to_string(),
                        (kind, interface.display_name().map(|name| name.to_string())),
                    );
                }
            }

            self.topology = topology;
            self.refreshed = Some(Instant::now());
        }
        let mut diagnostic_state = Vec::new();
        for (index, sample) in result.iter_mut().enumerate() {
            let mut active = None;
            sample.interface.details = addressed.get(&sample.interface.id).cloned();
            if let Some((kind, name)) = self.physical.get(&sample.interface.id) {
                sample.interface.kind = *kind;
                sample.interface.physical = true;
                // Carrier flags alone leave unused Thunderbolt ports apparently
                // connected. An assigned non-link-local IP establishes a usable link.
                active = link_active(self.store.as_ref(), &sample.interface.id);
                sample.interface.connected = connected_link(flags[&sample.interface.id], active)
                    && addressed
                        .get(&sample.interface.id)
                        .is_some_and(|details| !details.addresses.is_empty());
                if let Some(name) = name {
                    sample.interface.name.clone_from(name);
                }
                if *kind == InterfaceKind::Wifi {
                    let metadata = self
                        .wifi
                        .read(&sample.interface.id, sample.interface.connected);
                    if let Some(counters) = &mut sample.counters {
                        // IFMIB's generic baud rate is not the negotiated Wi-Fi rate.
                        // CoreWLAN exposes only transmit rate; never invent a receive rate.
                        counters.receive_link_bps = None;
                        counters.transmit_link_bps = metadata.as_ref().and_then(|value| value.0);
                    }
                    if let Some(details) = &mut sample.interface.details {
                        details.wifi = metadata.map(|value| value.1);
                    }
                }
            }
            diagnostic_state.push(format!(
                    "row={} kind={:?} physical={} connected={} active={:?} default_route={} counters={} flags={:#x}",
                    index,
                    sample.interface.kind,
                    sample.interface.physical,
                    sample.interface.connected,
                    active,
                    sample.interface.default_route_metric.is_some(),
                    sample.counters.is_some(),
                    flags[&sample.interface.id],
                ));
        }
        if self.diagnostic_state != diagnostic_state {
            log::info!(
                "network_interface_state platform=macos stage=sampling interfaces={} primary_count={} state={}",
                result.len(),
                primary.len(),
                crate::diagnostics::text(&diagnostic_state.join("; ")),
            );
            self.diagnostic_state = diagnostic_state;
        }
        self.wifi.retain(&result);
        Ok(result)
    }
}

fn unavailable() -> PlatformError {
    PlatformError::new(
        PlatformErrorCode::OperationFailed,
        "network counters unavailable",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distinct_family_primaries_keep_their_priority_independent_of_interface_names() {
        let primary = vec!["wired-primary".into(), "wifi-primary".into()];
        assert_eq!(primary_route_metric(&primary, "wired-primary"), Some(0));
        assert_eq!(primary_route_metric(&primary, "wifi-primary"), Some(1));
        assert_eq!(primary_route_metric(&primary, "unrelated"), None);
    }

    #[test]
    fn shared_or_missing_family_primaries_do_not_invent_another_route() {
        let primary = vec!["same-link".into(), "same-link".into()];
        assert_eq!(primary_route_metric(&primary, "same-link"), Some(0));
        assert_eq!(primary_route_metric(&primary[..1], "same-link"), Some(0));
        assert_eq!(primary_route_metric(&[], "same-link"), None);
    }

    #[test]
    fn native_link_state_overrides_driver_running_flag_with_safe_fallback() {
        let up = libc::IFF_UP;
        let running = libc::IFF_RUNNING;
        assert!(connected_link(up, Some(true)));
        assert!(!connected_link(up | running, Some(false)));
        assert!(!connected_link(running, Some(true)));
        assert!(connected_link(up | running, None));
        assert!(!connected_link(up, None));
    }

    #[test]
    fn usable_links_include_private_ipv4_and_ipv6_without_loopback_or_link_local() {
        for ip in ["192.168.1.2", "10.0.0.2", "fd00::2", "2001:db8::2"] {
            assert!(usable_address(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "0.0.0.0",
            "127.0.0.1",
            "169.254.1.2",
            "224.0.0.1",
            "::",
            "::1",
            "fe80::1",
            "ff02::1",
        ] {
            assert!(!usable_address(ip.parse().unwrap()), "{ip}");
        }
    }
}
