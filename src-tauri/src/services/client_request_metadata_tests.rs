use super::*;

#[test]
fn missing_or_invalid_identity_is_optional_and_never_recreated() {
    assert_eq!(install_id(None), None);
    for value in [
        serde_json::json!({}),
        serde_json::json!({"schemaVersion": 2, "installId": uuid::Uuid::new_v4()}),
        serde_json::json!({"schemaVersion": 1, "installId": "invalid"}),
    ] {
        assert_eq!(install_id(Some(&value)), None);
    }
    let id = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        install_id(Some(&serde_json::json!({"schemaVersion":1,"installId":id}))),
        Some(id)
    );
}

#[test]
fn reading_a_missing_installation_store_does_not_create_a_file() {
    let app = tauri::test::mock_builder()
        .plugin(tauri_plugin_store::Builder::default().build())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("missing.json");
    assert_eq!(read_install_id(app.handle(), &path), None);
    assert!(!path.exists());
    std::fs::write(&path, b"partial JSON").unwrap();
    assert_eq!(read_install_id(app.handle(), &path), None);
    let id = uuid::Uuid::new_v4().to_string();
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({"identity":{"schemaVersion":1,"installId":id}}))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(read_install_id(app.handle(), &path), Some(id));
}

pub(crate) fn metadata() -> OfficialRequestMetadata {
    OfficialRequestMetadata {
        locale: "en-US".into(),
        distribution: "installed".into(),
        os_version: Some("fixture-os".into()),
        system_locale: Some("it-IT".into()),
        install_id: Some(uuid::Uuid::new_v4().to_string()),
        request_id: Some(uuid::Uuid::new_v4().to_string()),
        ..Default::default()
    }
}

pub(crate) fn assert_headers(request: &str, metadata: &OfficialRequestMetadata) {
    let request = request.to_ascii_lowercase();
    for (key, value) in metadata.headers().iter() {
        assert!(
            request.contains(&format!(
                "{}: {}\r\n",
                key.as_str(),
                value.to_str().unwrap().to_ascii_lowercase()
            )),
            "missing {key}"
        );
    }
    if metadata.install_id.is_none() {
        assert!(!request.contains("x-mangodisk-install-id:"));
    }
}

type Reply = (&'static str, Vec<u8>, String);
pub(crate) fn serve(
    build: impl FnOnce(&str) -> Vec<Reply>,
) -> (String, std::thread::JoinHandle<Vec<String>>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let replies = build(&origin);
    let handle = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for (status, body, headers) in replies {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && std::time::Instant::now() < deadline =>
                    {
                        std::thread::sleep(std::time::Duration::from_millis(5))
                    }
                    Err(error) => panic!("fixture accept failed: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0; 4096];
            loop {
                let count = stream.read(&mut chunk).unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&chunk[..count]);
                if let Some(end) = bytes.windows(4).position(|row| row == b"\r\n\r\n") {
                    let text = String::from_utf8_lossy(&bytes[..end]);
                    let length = text
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .and_then(|value| value.parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
                assert!(bytes.len() < 65536);
            }
            requests.push(String::from_utf8(bytes).unwrap());
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{headers}Connection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
        }
        requests
    });
    (origin, handle)
}
