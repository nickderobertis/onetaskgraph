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
    // Whatever the first read said, the connection has nothing after it.
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
