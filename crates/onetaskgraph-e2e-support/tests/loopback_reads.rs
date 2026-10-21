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

/// An answer saying `Connection: close` is relayed whole, and the next request opens a
/// connection of its own.
#[test]
fn an_upstream_answering_close_is_replaced_for_the_next_request() {
    use onetaskgraph_e2e_support::loopback::{After, Upstream, serve_with};
    let listener = TcpListener::bind("127.0.0.1:0").expect("a listener");
    let address = listener.local_addr().expect("an address");
    let accepted = serve_with(listener, |_, stream| {
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 2\r\n\r\nok")
            .expect("answered");
        After::Close
    });
    let request = b"POST /graphql HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}";
    let mut upstream = Upstream::new(address);
    for _ in 0..2 {
        assert!(upstream.forward(request).ends_with(b"\r\n\r\nok"));
    }
    assert_eq!(accepted.load(std::sync::atomic::Ordering::SeqCst), 2);
}

/// A handler that panicked answering one request leaves the server answering the next.
#[test]
fn a_server_goes_on_answering_after_its_handler_panicked() {
    use onetaskgraph_e2e_support::loopback::{After, Upstream, serve_with};
    let listener = TcpListener::bind("127.0.0.1:0").expect("a listener");
    let address = listener.local_addr().expect("an address");
    let mut answered = 0;
    serve_with(listener, move |_, stream| {
        answered += 1;
        assert!(answered > 1, "the first request's handler panics");
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
            .expect("answered");
        After::KeepOpen
    });
    let request = b"POST /graphql HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}";
    assert!(
        Upstream::new(address).forward(request).is_empty(),
        "the panicking one"
    );
    assert!(
        Upstream::new(address)
            .forward(request)
            .ends_with(b"\r\n\r\nok")
    );
}

/// A request held on one connection until a request arrives on a second is answered, with
/// each connection kept to a proxy connection of its own to the server behind.
#[test]
fn connections_served_each_are_answered_side_by_side_over_their_own_upstreams() {
    use std::sync::{Arc, Barrier};
    use std::time::Duration;

    use onetaskgraph_e2e_support::loopback::{After, Upstream, serve_each, serve_with};
    let server = TcpListener::bind("127.0.0.1:0").expect("a listener");
    let address = server.local_addr().expect("an address");
    let behind = serve_with(server, |_, stream| {
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
            .expect("answered");
        After::KeepOpen
    });
    let proxy = TcpListener::bind("127.0.0.1:0").expect("a listener");
    let front = proxy.local_addr().expect("an address");
    let together = Arc::new(Barrier::new(2));
    let accepted = serve_each(proxy, move || {
        let (together, mut upstream) = (Arc::clone(&together), Upstream::new(address));
        move |sent: &onetaskgraph_e2e_support::loopback::Request, stream: &mut TcpStream| {
            let answer = upstream.forward(&sent.bytes());
            // Released only once the other connection's request is here too.
            together.wait();
            stream.write_all(&answer).expect("relayed");
            After::KeepOpen
        }
    });
    let clients: Vec<_> = (0..2)
        .map(|_| {
            std::thread::spawn(move || {
                let mut client = TcpStream::connect(front).expect("a client");
                client
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .expect("a timeout");
                let mut reader = BufReader::new(client.try_clone().expect("a reader"));
                for _ in 0..2 {
                    client
                        .write_all(b"POST /graphql HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}")
                        .expect("sent");
                    let answer = next_request(&mut reader)
                        .expect("an answer before the timeout")
                        .expect("answered");
                    assert_eq!(answer.body(), b"ok");
                }
            })
        })
        .collect();
    for client in clients {
        client.join().expect("both clients answered twice");
    }
    let count = |connections: &std::sync::atomic::AtomicUsize| {
        connections.load(std::sync::atomic::Ordering::SeqCst)
    };
    assert_eq!(count(&accepted), 2, "each client kept its one connection");
    assert_eq!(
        count(&behind),
        2,
        "each proxy connection kept its one upstream"
    );
}

/// A connection whose client asked for it to close ends after the answer, though the handler
/// would have kept it open.
#[test]
fn a_client_asking_to_close_ends_a_connection_its_handler_keeps_open() {
    use std::io::Read;

    use onetaskgraph_e2e_support::loopback::{After, serve_with};
    let listener = TcpListener::bind("127.0.0.1:0").expect("a listener");
    let address = listener.local_addr().expect("an address");
    serve_with(listener, |_, stream| {
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
            .expect("answered");
        After::KeepOpen
    });
    let mut client = TcpStream::connect(address).expect("a client");
    client
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .expect("a timeout");
    client
        .write_all(b"POST /graphql HTTP/1.1\r\nConnection: close\r\nContent-Length: 2\r\n\r\n{}")
        .expect("sent");
    let mut answer = Vec::new();
    client
        .read_to_end(&mut answer)
        .expect("answered, and then the connection ends");
    assert!(answer.ends_with(b"\r\n\r\nok"), "{answer:?}");
}

#[test]
fn a_head_that_is_not_http_is_refused_by_name() {
    for line in [
        "hello there",
        "hello HTTP/garbage",
        "POST /graphql HTTP/2",
        "HTTP/1.1 2xx OK",
    ] {
        let error = after_sending(format!("{line}\r\n\r\n").as_bytes()).expect_err(line);
        assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
        assert!(
            error
                .to_string()
                .contains("is neither an HTTP/1.x request line nor a status line"),
            "{error}"
        );
    }
    for header in ["no colon here", "bad name: value", ": nameless"] {
        let sent = format!("POST /graphql HTTP/1.1\r\n{header}\r\n\r\n");
        let error = after_sending(sent.as_bytes()).expect_err(header);
        assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
        assert!(
            error.to_string().contains(&format!("{header:?}")),
            "{error}"
        );
    }
    let answered = after_sending(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
        .expect("a status line is read as a response");
    assert_eq!(answered.as_deref(), Some("HTTP/1.1 200 OK"));
}

#[test]
fn content_lengths_that_disagree_are_refused() {
    let error = after_sending(
        b"POST /graphql HTTP/1.1\r\nContent-Length: 2\r\nContent-Length: 10\r\n\r\n{}",
    )
    .expect_err("ambiguous framing");
    assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
    assert!(error.to_string().contains("disagree"), "{error}");
    let agreed = after_sending(
        b"POST /graphql HTTP/1.1\r\nContent-Length: 2\r\nContent-Length: 2\r\n\r\n{}",
    )
    .expect("repeated lengths that agree frame one body");
    assert_eq!(agreed.as_deref(), Some("POST /graphql HTTP/1.1"));
}

#[test]
fn a_chunked_body_is_refused_rather_than_read_as_the_next_request() {
    let error = after_sending(
        b"POST /graphql HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\n\r\n",
    )
    .expect_err("unframeable");
    assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
    assert!(error.to_string().contains("\"chunked\""), "{error}");
}
