//! A transport observer forwards requests unchanged and records only successful creates.
use super::journal::Journal;
use serde_json::Value;
use std::{
    io::{Read, Write},
    net::TcpListener,
};

pub fn start(upstream: &str, key: &str, journal: Journal) -> String {
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
                    if let Ok(value) = serde_json::from_slice(&body) {
                        journal.observe(&request, &value);
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
