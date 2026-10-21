//! How a loopback server tells a client done with its connection from a request that could
//! not be read, over real loopback sockets.

use std::io::{BufReader, ErrorKind, Write};
use std::net::{Shutdown, TcpListener, TcpStream};

use onetaskgraph_e2e_support::loopback::next_request;

/// What the server side reads after a client sends `sent` and stops writing.
fn after_sending(sent: &[u8]) -> std::io::Result<Option<String>> {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a listener");
    let mut client =
        TcpStream::connect(listener.local_addr().expect("an address")).expect("a client");
    client.write_all(sent).expect("sent");
    client.shutdown(Shutdown::Write).expect("done writing");
    let (served, _) = listener.accept().expect("accepted");
    let mut reader = BufReader::new(served);
    let first = next_request(&mut reader)?.map(|request| request.line().to_owned());
    assert!(matches!(next_request(&mut reader), Ok(None) | Err(_)));
    Ok(first)
}

#[test]
fn a_client_closing_between_requests_is_the_connection_ending() {
    assert_eq!(after_sending(b"").expect("no error"), None);
}

#[test]
fn a_whole_request_is_read() {
    let read =
        after_sending(b"POST /graphql HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}").expect("a request");
    assert_eq!(read.as_deref(), Some("POST /graphql HTTP/1.1"));
}

#[test]
fn a_connection_ending_inside_a_head_is_an_error_not_an_end() {
    let error = after_sending(b"POST /graphql HTTP/1.1\r\nHost: x\r\n").expect_err("cut short");
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof, "{error}");
}

#[test]
fn a_connection_ending_inside_a_body_is_an_error_not_an_end() {
    let error = after_sending(b"POST /graphql HTTP/1.1\r\nContent-Length: 10\r\n\r\n{}")
        .expect_err("cut short");
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof, "{error}");
}

#[test]
fn a_length_that_is_not_one_is_refused_by_name() {
    let error = after_sending(b"POST /graphql HTTP/1.1\r\nContent-Length: ten\r\n\r\n")
        .expect_err("unframeable");
    assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
    assert!(error.to_string().contains("\"ten\""), "{error}");
}

/// A proxy's connection to its server, closed under it unanswered, is replaced by the next
/// request it forwards, which is answered.
#[test]
fn an_upstream_closed_unanswered_is_replaced_by_the_next_request() {
    use onetaskgraph_e2e_support::loopback::{After, Upstream, serve_with};
    let listener = TcpListener::bind("127.0.0.1:0").expect("a listener");
    let address = listener.local_addr().expect("an address");
    let mut answered = 0;
    let accepted = serve_with(listener, move |_, stream| {
        answered += 1;
        if answered == 1 {
            return After::Close;
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
            .expect("answered");
        After::KeepOpen
    });
    let request = b"POST /graphql HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}";
    let mut upstream = Upstream::new(address);
    assert!(upstream.forward(request).is_empty(), "closed unanswered");
    let answer = upstream.forward(request);
    assert!(
        answer.ends_with(b"\r\n\r\nok"),
        "{}",
        String::from_utf8_lossy(&answer)
    );
    assert_eq!(
        upstream.forward(request),
        answer,
        "the replacement is kept for the next"
    );
    assert_eq!(accepted.load(std::sync::atomic::Ordering::SeqCst), 2);
}

/// The first report naming `needle`, waited for a few seconds: a server reports from the thread
/// serving the connection, after the client has gone.
fn report_naming(needle: &str) -> String {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Some(line) = onetaskgraph_e2e_support::loopback::reported()
            .into_iter()
            .find(|line| line.contains(needle))
        {
            return line;
        }
        assert!(
            std::time::Instant::now() < until,
            "nothing reported naming {needle}: {:?}",
            onetaskgraph_e2e_support::loopback::reported()
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// A server that could not read a request says which connection, and why.
#[test]
fn a_server_reports_a_request_it_could_not_read_naming_the_connection() {
    use onetaskgraph_e2e_support::loopback::{After, serve_with};
    let listener = TcpListener::bind("127.0.0.1:0").expect("a listener");
    let address = listener.local_addr().expect("an address");
    serve_with(listener, |_, _| After::Close);
    let mut client = TcpStream::connect(address).expect("a client");
    client
        .write_all(b"POST /graphql HTTP/1.1\r\nHost: x\r\n")
        .expect("half a head");
    client.shutdown(Shutdown::Write).expect("done writing");
    let from = client.local_addr().expect("the client's end").to_string();
    let said = report_naming(&format!("on the connection from {from}"));
    assert!(
        said.starts_with(&format!(
            "loopback server on {address}: a request could not be read"
        )) && said.contains("the connection ended inside a request's head"),
        "{said}"
    );
}

/// A proxy that cannot reach its server, or reads an answer cut short, says so and answers
/// nothing, and the request after it is forwarded afresh.
#[test]
fn a_proxy_reports_what_it_could_not_forward_and_recovers() {
    use onetaskgraph_e2e_support::loopback::{After, Upstream, serve_with};
    let request = b"POST /graphql HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}";

    let gone = TcpListener::bind("127.0.0.1:0")
        .expect("a listener")
        .local_addr()
        .expect("an address");
    assert!(Upstream::new(gone).forward(request).is_empty());
    let said = report_naming(&format!("loopback proxy to {gone}: could not connect"));
    assert!(said.contains("(os error "), "{said}");

    let listener = TcpListener::bind("127.0.0.1:0").expect("a listener");
    let address = listener.local_addr().expect("an address");
    let mut answered = 0;
    serve_with(listener, move |_, stream| {
        answered += 1;
        if answered == 1 {
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nok");
            return After::Close;
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
            .expect("answered");
        After::KeepOpen
    });
    let mut upstream = Upstream::new(address);
    assert!(upstream.forward(request).is_empty(), "an answer cut short");
    report_naming(&format!(
        "loopback proxy to {address}: could not read an answer"
    ));
    assert!(upstream.forward(request).ends_with(b"\r\n\r\nok"));
}
