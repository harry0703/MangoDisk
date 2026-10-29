//! On-demand public egress discovery, separate from local traffic sampling.
use mangodisk_platform::system_resources::network::{InterfaceSample, NetworkReader};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    cell::RefCell,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    sync::Arc,
    time::Duration,
};

const ENDPOINT: &str = "https://mangodisk.app/api/v1/network/addresses/current";
const MAX_RESPONSE_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AddressFamily {
    Ipv4,
    Ipv6,
}
impl AddressFamily {
    fn matches(self, ip: IpAddr) -> bool {
        matches!(
            (self, ip),
            (Self::Ipv4, IpAddr::V4(_)) | (Self::Ipv6, IpAddr::V6(_))
        )
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NetworkAddress {
    pub schema_version: u8,
    pub ip: String,
    pub family: u8,
    pub country_code: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AddressError {
    Timeout,
    Network,
    Unavailable,
    InvalidResponse,
}
impl std::fmt::Display for AddressError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for AddressError {}

struct FamilyResolver(AddressFamily);
impl Resolve for FamilyResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let family = self.0;
        Box::pin(async move {
            let addresses = tokio::net::lookup_host((name.as_str(), 0))
                .await?
                .filter(move |address| family.matches(address.ip()))
                .collect::<Vec<_>>();
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddressData {
    ip: String,
    family: u8,
    country_code: Option<String>,
}
#[derive(Deserialize)]
struct Envelope {
    success: bool,
    data: Option<AddressData>,
}

fn parse(body: &[u8]) -> Result<NetworkAddress, AddressError> {
    let envelope: Envelope =
        serde_json::from_slice(body).map_err(|_| AddressError::InvalidResponse)?;
    if !envelope.success {
        return Err(AddressError::Unavailable);
    }
    let data = envelope.data.ok_or(AddressError::InvalidResponse)?;
    let ip: IpAddr = data.ip.parse().map_err(|_| AddressError::InvalidResponse)?;
    let family = if ip.is_ipv4() { 4 } else { 6 };
    if data.family != family {
        return Err(AddressError::InvalidResponse);
    }
    let country_code = data.country_code.filter(|code| {
        code.len() == 2 && code.bytes().all(|byte| byte.is_ascii_uppercase()) && code != "XX"
    });
    Ok(NetworkAddress {
        schema_version: 1,
        ip: ip.to_string(),
        family,
        country_code,
    })
}

fn request_error(error: reqwest::Error) -> AddressError {
    if error.is_timeout() {
        AddressError::Timeout
    } else {
        AddressError::Network
    }
}

pub async fn probe(
    family: AddressFamily,
    metadata: &crate::http_client::OfficialRequestMetadata,
) -> Result<NetworkAddress, AddressError> {
    // DNS filtering and source binding constrain the transport, including the
    // connection to a proxy. A proxy can still choose a different remote family;
    // the validated response remains authoritative for the displayed family.
    let source = match family {
        AddressFamily::Ipv4 => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        AddressFamily::Ipv6 => IpAddr::V6(Ipv6Addr::UNSPECIFIED),
    };
    let client = crate::http_client::builder()
        .dns_resolver(Arc::new(FamilyResolver(family)))
        .local_address(source)
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(request_error)?;
    probe_url(&client, ENDPOINT, metadata).await
}

async fn probe_url(
    client: &reqwest::Client,
    url: &str,
    metadata: &crate::http_client::OfficialRequestMetadata,
) -> Result<NetworkAddress, AddressError> {
    let mut response = client
        .get(url)
        .headers(metadata.headers())
        .header(reqwest::header::CACHE_CONTROL, "no-cache")
        .send()
        .await
        .map_err(request_error)?;
    if !response.status().is_success() {
        return Err(AddressError::Unavailable);
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(AddressError::InvalidResponse);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(request_error)? {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(AddressError::InvalidResponse);
        }
        body.extend_from_slice(&chunk);
    }
    parse(&body)
}

thread_local! {
    // Native configuration objects stay on the worker that created them. Tokio's
    // blocking pool may reuse that worker; no CF object crosses a thread boundary.
    static CONTEXT_READER: RefCell<NetworkReader> = RefCell::new(NetworkReader::default());
}

/// Inspect local configuration without packet capture or external traffic.
pub fn network_context_key() -> Result<String, mangodisk_platform::PlatformError> {
    CONTEXT_READER.with(|reader| reader.borrow_mut().read().map(|rows| context_key(&rows)))
}
fn context_key(rows: &[InterfaceSample]) -> String {
    let mut rows = rows
        .iter()
        .map(|sample| {
            let row = &sample.interface;
            let mut addresses = row
                .details
                .as_ref()
                .map(|details| details.addresses.clone())
                .unwrap_or_default();
            addresses.sort();
            (&row.id, row.connected, row.default_route_metric, addresses)
        })
        .collect::<Vec<_>>();
    rows.sort();
    // A digest is an opaque change token, never an interface diagnostic identity.
    let serialized =
        serde_json::to_vec(&rows).expect("network context contains only primitive values");
    Sha256::digest(serialized)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
#[path = "network_addresses_tests.rs"]
mod tests;
