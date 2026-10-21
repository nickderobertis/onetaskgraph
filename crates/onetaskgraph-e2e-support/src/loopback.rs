//! Persistent HTTP/1.1 connections for a loopback server a journey's binary talks to — why
//! they are persistent is in AGENTS.md. A binary's requests share the one connection its
//! client keeps alive, and the server closes a connection only once its client has, or asked
//! it to, so a client is never handed back a connection the server already closed.

use std::io::{self, BufRead, BufReader, Write};
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
    next_request(reader).ok().flatten()
}

/// [`read_request`], telling the connection ending where a request would begin — `Ok(None)`,
/// a client done with it — from a read that failed: the socket's own error, a connection that
/// ended part way through a request as `UnexpectedEof`, or a `Content-Length` that is not a
/// length as `InvalidData`.
///
/// # Errors
///
/// Whatever stopped a request being read whole.
pub fn next_request(reader: &mut impl BufRead) -> io::Result<Option<Request>> {
    let mut head = String::new();
    loop {
        let before = head.len();
        if reader.read_line(&mut head)? == 0 {
            if head.is_empty() {
                return Ok(None);
            }
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the connection ended inside a request's head",
            ));
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
        let length = length.parse::<usize>().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("a Content-Length of {length:?} is not a length"),
            )
        })?;
        request.body.resize(length, 0);
        reader.read_exact(&mut request.body)?;
    }
    Ok(Some(request))
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
/// Counts each connection accepted on `accepted`, which the caller may already have handed out.
pub(crate) fn serve<A>(listener: TcpListener, accepted: Arc<AtomicUsize>, mut answer: A)
where
    A: FnMut(&Request) -> Option<String> + Send + 'static,
{
    serve_counting(
        listener,
        Arc::default(),
        accepted,
        move |request, stream| {
            let Some(mut reply) = answer(request) else {
                return After::Close;
            };
            if request.closes() {
                // Said, so the client knows not to send another request on it.
                reply = reply.replacen("\r\n", "\r\nConnection: close\r\n", 1);
            }
            match stream.write_all(reply.as_bytes()) {
                Ok(()) => After::KeepOpen,
                Err(error) => {
                    unheard("an answer could not be written", stream, &error);
                    After::Close
                }
            }
        },
    );
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
    let accepted = Arc::default();
    serve_counting(listener, stop, Arc::clone(&accepted), answer);
    accepted
}

/// [`serve_until`], counting each connection accepted on `counted`.
fn serve_counting<A>(
    listener: TcpListener,
    stop: Arc<AtomicBool>,
    counted: Arc<AtomicUsize>,
    answer: A,
) where
    A: FnMut(&Request, &mut TcpStream) -> After + Send + 'static,
{
    let answer = Arc::new(Mutex::new(answer));
    thread::spawn(move || {
        // An accept that fails is a client that gave up before it was accepted, not a reason to
        // stop listening for every client after it — but it is said, since it is also what a
        // host short of sockets looks like from here.
        for stream in listener.incoming() {
            let stream = match stream {
                Ok(stream) => stream,
                Err(error) => {
                    eprintln!(
                        "loopback server on {}: an accept failed: {error}",
                        listener
                            .local_addr()
                            .map_or_else(|_| "?".to_owned(), |at| at.to_string())
                    );
                    continue;
                }
            };
            if stop.load(Ordering::SeqCst) {
                break;
            }
            counted.fetch_add(1, Ordering::SeqCst);
            let answer = Arc::clone(&answer);
            thread::spawn(move || {
                let reading = match stream.try_clone() {
                    Ok(reading) => reading,
                    Err(error) => {
                        unheard("a connection could not be read", &stream, &error);
                        return;
                    }
                };
                let mut reader = BufReader::new(reading);
                let mut writer = stream;
                loop {
                    let request = match next_request(&mut reader) {
                        Ok(Some(request)) => request,
                        Ok(None) => return,
                        Err(error) => {
                            unheard("a request could not be read", &writer, &error);
                            return;
                        }
                    };
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
}

/// Say on standard error what went wrong on a connection the server was serving — which the
/// test harness shows beside the failure of the test that started the server, and nowhere else.
fn unheard(what: &str, stream: &TcpStream, error: &io::Error) {
    let end = |at: io::Result<SocketAddr>| at.map_or_else(|_| "?".to_owned(), |at| at.to_string());
    eprintln!(
        "loopback server on {}: {what} on the connection from {}: {error}",
        end(stream.local_addr()),
        end(stream.peer_addr())
    );
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
        let failed = |what: &str, error: &io::Error| {
            eprintln!("loopback proxy to {}: {what}: {error}", self.address);
            Vec::new()
        };
        let mut connection = match self.connection.take() {
            Some(connection) => connection,
            None => match TcpStream::connect(self.address) {
                Ok(stream) => BufReader::new(stream),
                Err(error) => return failed("could not connect", &error),
            },
        };
        if let Err(error) = connection.get_mut().write_all(request) {
            return failed("could not send a request", &error);
        }
        let response = match next_request(&mut connection) {
            Ok(Some(response)) => response,
            // Closed unanswered, which a server may do on purpose.
            Ok(None) => return Vec::new(),
            Err(error) => return failed("could not read an answer", &error),
        };
        let whole = response.bytes();
        if !response.closes() {
            self.connection = Some(connection);
        }
        whole
    }
}
