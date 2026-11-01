//! A transport observer forwards requests unchanged and records only successful creates.
//!
//! Started [`guarded`], it also holds the lane to its run's [`Namespace`]: a request naming
//! another run's record is answered with that refusal and never reaches Linear.
use super::journal::Journal;
use super::namespace::Namespace;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::Arc,
};

#[allow(
    dead_code,
    reason = "this module is compiled into both lanes' test binaries, and the asset lane's only observer is the guarded one"
)]
pub fn start(upstream: &str, key: &str, journal: Journal) -> String {
    serve(upstream, key, journal, None)
}

/// [`start`], refusing every request outside `namespace`.
pub fn guarded(upstream: &str, key: &str, journal: Journal, namespace: Arc<Namespace>) -> String {
    serve(upstream, key, journal, Some(namespace))
}

fn serve(upstream: &str, key: &str, journal: Journal, namespace: Option<Arc<Namespace>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback observer");
    let endpoint = format!("http://{}/graphql", listener.local_addr().unwrap());
    let upstream = upstream.to_owned();
    let key = key.to_owned();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let client = reqwest::Client::new();
        'connections: for mut stream in listener.incoming().flatten() {
            let mut bytes = Vec::new();
            let mut buf = [0; 8192];
            let request: Value = loop {
                let n = match stream.read(&mut buf) {
                    Ok(0) | Err(_) => continue 'connections,
                    Ok(n) => n,
                };
                bytes.extend_from_slice(&buf[..n]);
                assert!(bytes.len() < 2_000_000, "bounded live request");
                if let Some(split) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&bytes[..split]).to_ascii_lowercase();
                    let size: usize = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if bytes.len() >= split + 4 + size {
                        break serde_json::from_slice(&bytes[split + 4..split + 4 + size]).unwrap();
                    }
                }
            };
            if let Some(refusal) = namespace
                .as_ref()
                .and_then(|namespace| namespace.admit(&request).err())
            {
                let body = json!({"errors":[{"message":refusal}]}).to_string();
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 Refused\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                continue;
            }
            let response = runtime.block_on(
                client
                    .post(&upstream)
                    .header("Authorization", &key)
                    .json(&request)
                    .send(),
            );
            match response {
                Ok(response) => {
                    let status = response.status().as_u16();
                    let body = runtime.block_on(response.bytes()).unwrap();
                    if let Ok(value) = serde_json::from_slice::<Value>(&body) {
                        journal.observe(&request, &value);
                        if let Some(namespace) = &namespace {
                            namespace.learn(&value);
                        }
                    }
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status} Observed\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(&body);
                }
                Err(_) => {
                    let _ = stream.write_all(b"HTTP/1.1 502 Upstream unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                }
            }
        }
    });
    endpoint
}
