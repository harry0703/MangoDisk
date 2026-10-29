use super::*;
use mangodisk_platform::system_resources::network::{
    InterfaceKind, NetworkInterface, NetworkInterfaceDetails,
};

#[test]
fn validates_actual_family_and_preserves_optional_country() {
    let v4 =
        parse(br#"{"success":true,"data":{"ip":"203.0.113.1","family":4,"countryCode":"SG"}}"#)
            .unwrap();
    assert_eq!(v4.family, 4);
    assert_eq!(v4.country_code.as_deref(), Some("SG"));
    let v6 =
        parse(br#"{"success":true,"data":{"ip":"2001:db8::1","family":6,"countryCode":null}}"#)
            .unwrap();
    assert_eq!(v6.family, 6);
    assert_eq!(v6.country_code, None);
    assert_eq!(serde_json::to_value(v6).unwrap()["schemaVersion"], 1);
}
#[test]
fn rejects_malformed_or_inconsistent_addresses() {
    for (ip, family) in [
        ("203.0.113.1", 6),
        ("2001:db8::1", 4),
        ("1.2.3.4:80", 4),
        ("1.2.3.4, 5.6.7.8", 4),
        ("fe80::1%en0", 6),
        ("999.1.1.1", 4),
        ("127.0.0.1/8", 4),
    ] {
        let body =
            serde_json::json!({"success":true,"data":{"ip":ip,"family":family,"countryCode":null}});
        assert_eq!(
            parse(body.to_string().as_bytes()),
            Err(AddressError::InvalidResponse)
        );
    }
    assert_eq!(
        parse(br#"{"success":false,"error":{"code":"NETWORK_ADDRESS_UNAVAILABLE"}}"#),
        Err(AddressError::Unavailable)
    );
    assert_eq!(
        parse(br#"{"success":true}"#),
        Err(AddressError::InvalidResponse)
    );
}
#[test]
fn unknown_country_does_not_discard_the_address() {
    for code in ["XX", "T1", "cn", "", "UNKNOWN"] {
        let body = serde_json::json!({"success":true,"data":{"ip":"203.0.113.1","family":4,"countryCode":code}});
        assert!(parse(body.to_string().as_bytes())
            .unwrap()
            .country_code
            .is_none());
    }
}
fn sample(id: &str, connected: bool, metric: Option<u32>, address: &str) -> InterfaceSample {
    InterfaceSample {
        interface: NetworkInterface {
            id: id.into(),
            name: id.into(),
            kind: InterfaceKind::Virtual,
            connected,
            physical: false,
            default_route_metric: metric,
            details: Some(NetworkInterfaceDetails {
                addresses: vec![address.into()],
                ..Default::default()
            }),
        },
        counters: None,
    }
}
#[test]
fn context_ignores_order_and_counters_but_detects_vpn_routes_and_addresses() {
    let a = sample("en0", true, Some(0), "192.0.2.1");
    let b = sample("utun1", true, None, "10.0.0.1");
    let before = context_key(&[a.clone(), b.clone()]);
    assert_eq!(before, context_key(&[b.clone(), a.clone()]));
    let mut counters = a.clone();
    counters.counters = Some(Default::default());
    assert_eq!(before, context_key(&[counters, b.clone()]));
    for changed in [
        sample("utun1", false, None, "10.0.0.1"),
        sample("utun1", true, Some(0), "10.0.0.1"),
        sample("utun1", true, None, "10.0.0.2"),
    ] {
        assert_ne!(before, context_key(&[a.clone(), changed]));
    }
    assert_ne!(before, context_key(&[a]));
}
#[tokio::test]
async fn resolver_never_returns_the_other_transport_family() {
    for family in [AddressFamily::Ipv4, AddressFamily::Ipv6] {
        let addresses = FamilyResolver(family)
            .resolve("localhost".parse().unwrap())
            .await
            .unwrap();
        assert!(addresses
            .into_iter()
            .all(|address| family.matches(address.ip())));
    }
}
async fn fixture(
    status: &str,
    body: Vec<u8>,
    extra_headers: &str,
) -> Result<NetworkAddress, AddressError> {
    fixture_metadata(
        status,
        body,
        extra_headers,
        crate::http_client::OfficialRequestMetadata::default(),
    )
    .await
}
async fn fixture_metadata(
    status: &str,
    body: Vec<u8>,
    extra_headers: &str,
    metadata: crate::http_client::OfficialRequestMetadata,
) -> Result<NetworkAddress, AddressError> {
    use std::io::{Read, Write};
    let expected = metadata.headers();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{extra_headers}Connection: close\r\n\r\n",
        body.len()
    );
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = [0; 4096];
        let count = stream.read(&mut request).unwrap();
        let request = String::from_utf8_lossy(&request[..count]);
        let request = request.to_ascii_lowercase();
        for (name, value) in &expected {
            assert!(
                request.contains(&format!(
                    "{}: {}\r\n",
                    name.as_str(),
                    value.to_str().unwrap().to_ascii_lowercase()
                )),
                "missing expected header: {name}"
            );
        }
        if !expected.contains_key("x-mangodisk-install-id") {
            assert!(!request.contains("x-mangodisk-install-id:"));
        }
        assert_eq!(request.matches("user-agent:").count(), 1);
        assert!(!request.to_lowercase().contains("cookie:"));
        assert!(!request.to_lowercase().contains("authorization:"));
        stream.write_all(response.as_bytes()).unwrap();
        let _ = stream.write_all(&body);
    });
    let client = crate::http_client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let result = probe_url(&client, &format!("http://{address}/"), &metadata).await;
    server.join().unwrap();
    result
}
#[tokio::test]
async fn actual_public_requests_include_metadata_with_or_without_an_installation_id() {
    for install_id in [None, Some(uuid::Uuid::new_v4().to_string())] {
        let metadata = crate::http_client::OfficialRequestMetadata {
            locale: "en-US".into(),
            distribution: "installed".into(),
            os_version: Some("15.7".into()),
            request_id: Some(uuid::Uuid::new_v4().to_string()),
            install_id,
            ..Default::default()
        };
        let body = br#"{"success":true,"data":{"ip":"203.0.113.1","family":4}}"#.to_vec();
        assert!(fixture_metadata("200 OK", body, "", metadata).await.is_ok());
    }
}

#[tokio::test]
async fn http_handles_success_failures_redirects_and_bounded_bodies() {
    let body =
        br#"{"success":true,"data":{"ip":"203.0.113.1","family":4,"countryCode":"SG"}}"#.to_vec();
    assert_eq!(fixture("200 OK", body.clone(), "").await.unwrap().family, 4);
    assert_eq!(
        fixture("503 Unavailable", Vec::new(), "").await,
        Err(AddressError::Unavailable)
    );
    assert_eq!(
        fixture("401 Unauthorized", Vec::new(), "").await,
        Err(AddressError::Unavailable)
    );
    assert_eq!(
        fixture("302 Found", Vec::new(), "Location: http://127.0.0.1:1/\r\n").await,
        Err(AddressError::Unavailable)
    );
    assert_eq!(
        fixture("200 OK", vec![b'x'; MAX_RESPONSE_BYTES + 1], "").await,
        Err(AddressError::InvalidResponse)
    );
    assert_eq!(
        fixture("200 OK", b"not json".to_vec(), "").await,
        Err(AddressError::InvalidResponse)
    );
}
#[tokio::test]
#[ignore = "requires explicit production network access; IPv6 may be unavailable"]
async fn live_public_address_discovery() {
    let metadata = crate::http_client::OfficialRequestMetadata {
        locale: "en-US".into(),
        distribution: "installed".into(),
        request_id: Some(uuid::Uuid::new_v4().to_string()),
        ..Default::default()
    };
    let (v4, v6) = tokio::join!(
        probe(AddressFamily::Ipv4, &metadata),
        probe(AddressFamily::Ipv6, &metadata)
    );
    let v4 = v4.expect("production IPv4 request must succeed");
    assert_eq!(v4.schema_version, 1);
    assert!(v4.ip.parse::<IpAddr>().is_ok());
    println!(
        "ipv4_transport=ready actual_family={} country_present={}",
        v4.family,
        v4.country_code.is_some()
    );
    match v6 {
        Ok(value) => println!(
            "ipv6_transport=ready actual_family={} country_present={}",
            value.family,
            value.country_code.is_some()
        ),
        Err(error) => println!("ipv6_transport=unavailable reason={error:?}"),
    }
}

#[test]
#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
fn native_context_stays_on_its_worker_and_has_a_bounded_change_token() {
    let worker = std::thread::spawn(|| {
        let started = std::time::Instant::now();
        let key = network_context_key().expect("native network context must be readable");
        assert_eq!(key.len(), 64);
        assert!(key.bytes().all(|byte| byte.is_ascii_hexdigit()));
        println!("network_context_read_ms={}", started.elapsed().as_millis());
    });
    worker.join().unwrap();
}

#[tokio::test]
async fn response_body_stalls_respect_the_total_request_timeout() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 1024];
        assert!(stream.read(&mut request).unwrap() > 0);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{")
            .unwrap();
        std::thread::sleep(Duration::from_millis(200));
    });
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_millis(80))
        .build()
        .unwrap();
    let result = probe_url(
        &client,
        &format!("http://{address}/"),
        &crate::http_client::OfficialRequestMetadata::default(),
    )
    .await;
    server.join().unwrap();
    assert_eq!(result, Err(AddressError::Timeout));
}
