//! Persistent HTTP/1.1 connections for a loopback server a journey's binary talks to — why
//! they are persistent is in AGENTS.md. A binary's requests share the one connection its
//! client keeps alive, and the server closes a connection only once its client has, or asked
//! it to, so a client is never handed back a connection the server already closed.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

/// One request read off a connection, and no byte of the one after it. Only [`read_request`]
/// makes one, so its body is always exactly what its head declares.
pub struct Request {
    head: String,
    body: Vec<u8>,
}

impl Request {
    /// The request line and every header, through the blank line that ends them.
    #[must_use]
    pub fn head(&self) -> &str {
        &self.head
    }

    /// Exactly the `Content-Length` bytes the head declares, or none when it declares none.
    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// The request whole, as the client sent it.
    #[must_use]
    pub fn bytes(&self) -> Vec<u8> {
        [self.head.as_bytes(), &self.body].concat()
    }

    /// The request line, `POST /graphql HTTP/1.1`.
    #[must_use]
    pub fn line(&self) -> &str {
        self.head.lines().next().unwrap_or_default()
    }

    /// The value of the header `name`, matched without regard to case.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.head.lines().skip(1).find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.trim()
                .eq_ignore_ascii_case(name)
                .then_some(value.trim())
        })
    }

    /// Whether the client asked for its connection to end with this answer.
    fn closes(&self) -> bool {
        self.header("connection")
            .is_some_and(|value| value.eq_ignore_ascii_case("close"))
    }
}

/// The next request on `reader`, or `None` when the connection ended — or failed — before one
/// was whole, which is a client going away rather than a request, or when its `Content-Length`
/// is not a length, after which nothing on the connection can be framed. A response is read
/// the same way, its status line standing where a request's line does.
pub fn read_request(reader: &mut impl BufRead) -> Option<Request> {
    let mut head = String::new();
    loop {
        let before = head.len();
        if reader.read_line(&mut head).ok()? == 0 {
            return None;
        }
        if matches!(&head[before..], "\r\n" | "\n") {
            break;
        }
    }
    let mut request = Request {
        head,
        body: Vec::new(),
    };
    if let Some(length) = request.header("content-length") {
        request.body.resize(length.parse::<usize>().ok()?, 0);
        reader.read_exact(&mut request.body).ok()?;
    }
    Some(request)
}

/// `request`, whole as a client sent it, asking the server to close the connection after
/// answering it — for a proxy that forwards one request per connection to a server keeping
/// connections alive, and relays the answer to a client that must not reuse its own.
pub fn closing(request: &[u8]) -> Vec<u8> {
    let line = request
        .windows(2)
        .position(|window| window == b"\r\n")
        .expect("a request line")
        + 2;
    [&request[..line], b"Connection: close\r\n", &request[line..]].concat()
}

/// Serve `listener` on a thread of its own, each connection on another, answering each of its
/// requests in turn with what `answer` returns — whole HTTP/1.1 response bytes, sent without a
/// `Connection: close` so the client keeps the connection — or, for `None`, closing that
/// connection with the request unanswered.
///
/// Returns the count of connections accepted.
pub(crate) fn serve<A>(listener: TcpListener, mut answer: A) -> Arc<AtomicUsize>
where
    A: FnMut(&Request) -> Option<String> + Send + 'static,
{
    serve_with(listener, move |request, stream| {
        let Some(mut reply) = answer(request) else {
            return After::Close;
        };
        if request.closes() {
            // Said, so the client knows not to send another request on it.
            reply = reply.replacen("\r\n", "\r\nConnection: close\r\n", 1);
        }
        match stream.write_all(reply.as_bytes()) {
            Ok(()) => After::KeepOpen,
            Err(_) => After::Close,
        }
    })
}

/// What a connection does once [`serve_with`]'s handler has answered one of its requests.
pub enum After {
    /// Waits for the client's next request.
    KeepOpen,
    /// Closes, the client having been answered — or deliberately not.
    Close,
}

/// [`serve`], for a handler that writes its own answer to the connection — bytes rather than
/// text, a body cut short, or a hold it must keep until the answer is written — and says
/// whether the connection stays open. One the client asked to close is closed either way.
///
/// `answer` runs for one request at a time across every connection, in the order the requests
/// arrive, so a server written for one connection at a time sees exactly what it saw before.
/// Returns the count of connections accepted.
pub fn serve_with<A>(listener: TcpListener, answer: A) -> Arc<AtomicUsize>
where
    A: FnMut(&Request, &mut TcpStream) -> After + Send + 'static,
{
    serve_until(listener, Arc::default(), answer)
}

/// [`serve_with`], until `stop` is set: the next connection accepted after it is, the one a
/// server's owner opens to wake it, ends the listening, and the listener with it.
pub fn serve_until<A>(listener: TcpListener, stop: Arc<AtomicBool>, answer: A) -> Arc<AtomicUsize>
where
    A: FnMut(&Request, &mut TcpStream) -> After + Send + 'static,
{
    let answer = Arc::new(Mutex::new(answer));
    let accepted = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&accepted);
    thread::spawn(move || {
        // An accept that fails is a client that gave up before it was accepted, not a reason to
        // stop listening for every client after it.
        for stream in listener.incoming().flatten() {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            counted.fetch_add(1, Ordering::SeqCst);
            let answer = Arc::clone(&answer);
            thread::spawn(move || {
                let Ok(reading) = stream.try_clone() else {
                    return;
                };
                let mut reader = BufReader::new(reading);
                let mut writer = stream;
                while let Some(request) = read_request(&mut reader) {
                    // A poisoned lock is an answer that panicked, and the panic already says
                    // why; every later request goes on being answered rather than repeating it.
                    let after = (answer.lock().unwrap_or_else(PoisonError::into_inner))(
                        &request,
                        &mut writer,
                    );
                    if matches!(after, After::Close) || request.closes() {
                        return;
                    }
                }
            });
        }
    });
    accepted
}

/// One connection a proxy keeps to the server it forwards to, so that forwarding a client's
/// kept-alive connection does not cost a connection per request on the far side instead.
pub struct Upstream {
    address: SocketAddr,
    connection: Option<BufReader<TcpStream>>,
}

impl Upstream {
    /// A connection to `address`, opened by the first request forwarded.
    #[must_use]
    pub const fn new(address: SocketAddr) -> Self {
        Self {
            address,
            connection: None,
        }
    }

    /// Send `request` and read back the one response it is answered with, whole — or nothing,
    /// when the server could not be reached or closed the connection without an answer, which
    /// the next request then replaces.
    pub fn forward(&mut self, request: &[u8]) -> Vec<u8> {
        let connection = match self.connection.take() {
            Some(connection) => connection,
            None => match TcpStream::connect(self.address) {
                Ok(stream) => BufReader::new(stream),
                Err(_) => return Vec::new(),
            },
        };
        let mut connection = connection;
        let sent = connection.get_mut().write_all(request).is_ok();
        let Some(response) = sent.then(|| read_request(&mut connection)).flatten() else {
            return Vec::new();
        };
        let whole = response.bytes();
        if !response.closes() {
            self.connection = Some(connection);
        }
        whole
    }
}
