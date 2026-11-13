//! Maintained HTTP implementation; finite response sizes and explicit outcomes.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Delivery { Accepted, Retry(u64), Rejected, QuotaLimited, Revoked }

pub(super) fn classify(status: u16, body: &[u8], retry_after: Option<u64>) -> Delivery {
    if status == 429 { return Delivery::Retry(retry_after.unwrap_or(30).clamp(1, 3600)); }
    if status == 408 || status >= 500 { return Delivery::Retry(5); }
    if !(200..300).contains(&status) { return Delivery::Rejected; }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else { return Delivery::Retry(5); };
    if value.get("quota_limited").is_some_and(|v| match v {
        serde_json::Value::Array(values) => !values.is_empty(),
        serde_json::Value::Bool(value) => *value,
        serde_json::Value::Null => false,
        _ => true,
    }) { return Delivery::QuotaLimited; }
    if value.get("status").is_some_and(|v| v == 1 || v == "success") { Delivery::Accepted }
    else { Delivery::Rejected }
}

pub(super) fn send(home: &Path, config: &Config, item: &spool::Stored) -> Delivery {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(3)))
        .max_redirects(0).max_response_header_size(16 * 1024)
        .http_status_as_error(false).build().into();
    send_with(&agent, home, config, item)
}

fn send_with(agent: &ureq::Agent, home: &Path, config: &Config, item: &spool::Stored) -> Delivery {
    if process_revoked() || !item.permitted(home, config) { return Delivery::Revoked; }
    let body = item.payload(config);
    // Last application-level consent check. In-flight HTTP cannot be recalled.
    if process_revoked() || !item.permitted(home, config) { return Delivery::Revoked; }
    let response = agent.post(format!("{}/i/v0/e/", config.host))
        .header("Content-Type", "application/json").send(body.as_slice());
    let Ok(mut response) = response else { return Delivery::Retry(5); };
    let status = response.status().as_u16();
    let retry = response.headers().get("Retry-After").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok());
    let body = response.body_mut().with_config().limit(16 * 1024).read_to_vec();
    match body { Ok(body) => classify(status, &body, retry), Err(_) => Delivery::Retry(5) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn acceptance_requires_valid_acknowledgment() {
        assert_eq!(classify(200, br#"{"status":1}"#, None), Delivery::Accepted);
        assert_eq!(classify(200, br#"{"status":"success"}"#, None), Delivery::Accepted);
        assert_eq!(classify(200, b"broken", None), Delivery::Retry(5));
        assert_eq!(classify(200, br#"{"status":0}"#, None), Delivery::Rejected);
        assert_eq!(classify(200, br#"{"status":1,"quota_limited":["events"]}"#, None), Delivery::QuotaLimited);
        for status in [400, 401, 403, 404, 302] { assert_eq!(classify(status, b"", None), Delivery::Rejected); }
        for status in [408, 500, 503] { assert_eq!(classify(status, b"", None), Delivery::Retry(5)); }
        assert_eq!(classify(429, b"", Some(999999)), Delivery::Retry(3600));
    }
    #[test]
    fn loopback_contract_and_no_redirect_following() {
        use std::net::TcpListener;
        let home = tempfile::tempdir().unwrap();
        set_enabled(home.path(), true).unwrap();
        for (status, body, expected) in [(200, "{\"status\":1}", Delivery::Accepted), (429, "{}", Delivery::Retry(7)), (302, "{}", Delivery::Rejected)] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let config = Config { key: "phc_synthetic_only".into(), host: format!("http://{}", listener.local_addr().unwrap()) };
            let id = identity(home.path()).unwrap();
            let item = spool::Stored::new(&config, &id, Event::TransferFinished, Origin::Cli, Outcome::Failed, ErrorCode::Io, true);
            let expected_id = item.uuid.clone();
            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut data = vec![];
                let mut buffer = [0; 4096];
                loop {
                    let n = socket.read(&mut buffer).unwrap();
                    if n == 0 { break; }
                    data.extend_from_slice(&buffer[..n]);
                    if let Some(index) = data.windows(4).position(|v| v == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&data[..index]).to_lowercase();
                        let len: usize = header.lines().find_map(|l| l.strip_prefix("content-length:")).unwrap().trim().parse().unwrap();
                        if data.len() >= index + 4 + len { break; }
                    }
                }
                let request = String::from_utf8(data).unwrap();
                assert!(request.starts_with("POST /i/v0/e/ HTTP/1.1"));
                assert!(request.contains(&expected_id));
                assert!(request.contains("$exception_list"));
                assert!(!request.contains("SECRET_SENTINEL"));
                write!(socket, "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nRetry-After: 7\r\nLocation: http://127.0.0.1:1/never\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            });
            let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(2))).max_redirects(0).http_status_as_error(false).build().into();
            assert_eq!(send_with(&agent, home.path(), &config, &item), expected);
            server.join().unwrap();
        }
    }
}
