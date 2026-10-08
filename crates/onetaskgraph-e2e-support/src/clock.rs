//! A simulated clock for journeys whose plugins pace and back off: the coordinator every
//! spawned binary attaches to, and the hold a loopback endpoint puts on an answer.
//!
//! A journey starts one [`SimulatedClock`] for the number of binary processes it will spawn,
//! hands each process the environment [`SimulatedClock::client_env`] answers for it, and has
//! each loopback endpoint the processes talk to take a [`SimulatedRequest`] for every request
//! it receives. Virtual time then advances only while every attached process is waiting —
//! never while one is computing — so a minute of pacing costs no real time and a journey's
//! timing figures are the same on every run. The client half, which the binary runs, is
//! `onetaskgraph_core::process_clock`; the wire between the two is stated in its module.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use tokio::sync::oneshot;

/// A coordinator of virtual time for a fixed number of binary processes.
///
/// Its origin is zero. **Virtual time advances only when every attached client is waiting**
/// — sleeping on its clock, or blocked on a request whose handler is in
/// [`SimulatedRequest::delay`] — **and no handler holds a request outside a delay**; it then
/// moves to the earliest pending wake-up, wakes everything due at that instant, and stops. It
/// never advances while any client or any handler is computing, so receiving a 500 KB body
/// costs no virtual time, and neither does hashing a 12 MB copy. It does not advance at all
/// until `clients` processes have attached. A client counts as waiting while it has any wait
/// pending on the clock, and stops counting once it detaches — its process exiting — so a
/// client that has finished never holds the others back. Nothing here sleeps in real time.
///
/// **Telling clients apart at a loopback endpoint** is the endpoint's: give each client a
/// listener of its own — hand client `n` the address of the `n`th — and have each listener's
/// handler call [`request`](Self::request) with that `n`. That is how every journey here does
/// it, and it needs nothing from the request itself.
#[derive(Clone)]
pub struct SimulatedClock {
    state: Arc<Mutex<State>>,
    address: SocketAddr,
}

/// An opt-in logical participant across multiple binary invocations.
///
/// While its participant has no attached subprocess, the driver is computing and virtual
/// time cannot advance. Dropping the lease means the logical participant has finished.
/// Existing callers without a lease keep the coordinator's original detach behaviour.
pub struct SimulatedLease {
    state: Arc<Mutex<State>>,
    client: usize,
}

impl SimulatedLease {
    /// `wait_for_detach(&self) -> ()`: after waiting for one subprocess to exit, wait for
    /// its clock connection's detach to be acknowledged before starting the next.
    /// This synchronises connection teardown and advances no virtual time.
    pub fn wait_for_detach(&self) {
        let mut state = self.state.lock().expect("the coordinator is not poisoned");
        while state.clients.contains_key(&self.client) {
            let disconnected = Arc::clone(&state.disconnected);
            state = disconnected
                .wait(state)
                .expect("the coordinator is not poisoned");
        }
    }
}

impl Drop for SimulatedLease {
    fn drop(&mut self) {
        let mut state = self.state.lock().expect("the coordinator is not poisoned");
        state.leases.remove(&self.client);
        state.advance();
    }
}

/// A request a loopback endpoint's handler is answering, taken for each request it receives.
///
/// While it is held outside [`delay`](Self::delay) the handler is computing, and virtual time
/// does not advance. Dropping it — once the answer has been written — is the handler being
/// done with the request.
pub struct SimulatedRequest {
    state: Arc<Mutex<State>>,
    client: usize,
}

/// Everything the coordinator knows, behind one lock.
#[derive(Default)]
struct State {
    now: Duration,
    expected: usize,
    /// Every client that has ever attached.
    attached: BTreeSet<usize>,
    /// The clients attached now, each with its pending waits and its open connection.
    clients: BTreeMap<usize, Client>,
    /// Logical participants whose driver may be between subprocess invocations.
    leases: BTreeSet<usize>,
    disconnected: Arc<Condvar>,
    /// Requests held by a handler outside a delay.
    computing: usize,
    /// Pending delays, by deadline then a sequence number, with whose request each holds.
    delays: BTreeMap<(Duration, u64), (usize, oneshot::Sender<()>)>,
    delay_sequence: u64,
}

/// One attached client.
struct Client {
    writer: TcpStream,
    /// Its waits on the clock, by its own sequence number, each with its deadline.
    sleeps: BTreeMap<u64, Duration>,
    /// Its requests held in a delay.
    delayed: usize,
}

impl Client {
    fn waiting(&self) -> bool {
        !self.sleeps.is_empty() || self.delayed > 0
    }

    fn send(&mut self, line: &str) {
        // A client that has gone is detached by its own reader; nothing to do here.
        let _ = writeln!(self.writer, "{line}").and_then(|()| self.writer.flush());
    }
}

impl State {
    /// Advance to the earliest pending wake-up when nothing is computing, and wake what is
    /// due there.
    fn advance(&mut self) {
        if self.attached.len() < self.expected
            || self.computing > 0
            || self
                .leases
                .iter()
                .any(|client| !self.clients.contains_key(client))
        {
            return;
        }
        if !self.clients.values().all(Client::waiting) {
            return;
        }
        let earliest_sleep = self
            .clients
            .values()
            .flat_map(|client| client.sleeps.values().copied())
            .min();
        let earliest_delay = self.delays.keys().next().map(|(deadline, _)| *deadline);
        let Some(earliest) = [earliest_sleep, earliest_delay].into_iter().flatten().min() else {
            return;
        };
        self.now = self.now.max(earliest);
        let now = self.now;
        for client in self.clients.values_mut() {
            let due: Vec<u64> = client
                .sleeps
                .iter()
                .filter(|(_, deadline)| **deadline <= now)
                .map(|(seq, _)| *seq)
                .collect();
            for seq in due {
                client.sleeps.remove(&seq);
                client.send(&format!("wake {seq}"));
            }
        }
        let due: Vec<(Duration, u64)> = self
            .delays
            .keys()
            .take_while(|(deadline, _)| *deadline <= now)
            .copied()
            .collect();
        for key in due {
            if let Some((client, waker)) = self.delays.remove(&key) {
                if let Some(held) = self.clients.get_mut(&client) {
                    held.delayed -= 1;
                }
                self.computing += 1;
                let _ = waker.send(());
            }
        }
    }
}

impl SimulatedClock {
    /// Virtual time starts at zero, and does not advance until `clients` binary
    /// processes have attached.
    pub fn start(clients: usize) -> SimulatedClock {
        // llmlint: ignore[no_panics_on_recoverable_errors] The signature is the shared one the
        // plugin nodes build against, `start(clients) -> SimulatedClock`, and its only callers
        // are tests: a host that cannot bind a loopback port has no test this could still run,
        // and failing it with the reason is the recoverable path a test harness has.
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback listener");
        let address = listener.local_addr().expect("the listener's address");
        let state = Arc::new(Mutex::new(State {
            expected: clients,
            ..State::default()
        }));
        let accepting = Arc::clone(&state);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let serving = Arc::clone(&accepting);
                std::thread::spawn(move || serve(&serving, stream));
            }
        });
        SimulatedClock { state, address }
    }

    /// The environment one spawned binary needs to run on this clock as client
    /// `client` (0-based).
    pub fn client_env(&self, client: usize) -> Vec<(String, String)> {
        vec![(
            onetaskgraph_plugin_api::SIMULATED_CLOCK_VARIABLE.to_owned(),
            format!("{}/{client}", self.address),
        )]
    }

    /// Current virtual time.
    pub fn now(&self) -> std::time::Duration {
        self.lock().now
    }

    /// Taken by a loopback endpoint's handler for each request it receives from
    /// `client`, and dropped when the answer has been written.
    pub fn request(&self, client: usize) -> SimulatedRequest {
        self.lock().computing += 1;
        SimulatedRequest {
            state: Arc::clone(&self.state),
            client,
        }
    }

    /// `lease(client: usize) -> SimulatedLease`: keep one logical participant present
    /// across its subprocess transitions. Take it before the first invocation and drop it
    /// after the last; the driver between invocations then counts as computing. The client
    /// must be one `start` nominated and may hold only one lease at a time.
    ///
    /// # Panics
    /// A client outside this coordinator's nomination or one already leased is a test bug.
    pub fn lease(&self, client: usize) -> SimulatedLease {
        let mut state = self.lock();
        assert!(
            client < state.expected,
            "a lease must name a nominated client"
        );
        assert!(state.leases.insert(client), "a client holds only one lease");
        SimulatedLease {
            state: Arc::clone(&self.state),
            client,
        }
    }

    /// Every client that has attached so far, whether or not it is still attached.
    pub fn attached(&self) -> Vec<usize> {
        self.lock().attached.iter().copied().collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().expect("the coordinator is not poisoned")
    }
}

impl SimulatedRequest {
    /// Hold this request's answer until `duration` of virtual time has passed.
    ///
    /// Dropped before it completes — a handler giving up on the hold — it takes its wake-up
    /// back and the request is computing again, so a hold nobody waits on any more neither
    /// advances virtual time nor counts its client as waiting.
    pub async fn delay(&self, duration: std::time::Duration) {
        let (waker, woken) = oneshot::channel();
        let key = {
            let mut state = self.state.lock().expect("the coordinator is not poisoned");
            let deadline = state.now + duration;
            let sequence = state.delay_sequence;
            state.delay_sequence += 1;
            state
                .delays
                .insert((deadline, sequence), (self.client, waker));
            state.computing -= 1;
            if let Some(client) = state.clients.get_mut(&self.client) {
                client.delayed += 1;
            }
            state.advance();
            (deadline, sequence)
        };
        let held = Held {
            state: &self.state,
            client: self.client,
            key,
        };
        // The coordinator moved this request back to computing as it fired the waker.
        let _ = woken.await;
        std::mem::forget(held);
    }
}

/// A hold not yet woken, which puts its request back to computing if it is dropped first.
struct Held<'a> {
    state: &'a Arc<Mutex<State>>,
    client: usize,
    key: (Duration, u64),
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        let mut state = self.state.lock().expect("the coordinator is not poisoned");
        // Already fired, the coordinator moved it back itself; otherwise this does.
        if state.delays.remove(&self.key).is_some() {
            if let Some(client) = state.clients.get_mut(&self.client) {
                client.delayed -= 1;
            }
            state.computing += 1;
        }
    }
}

impl Drop for SimulatedRequest {
    fn drop(&mut self) {
        let mut state = self.state.lock().expect("the coordinator is not poisoned");
        state.computing -= 1;
        state.advance();
    }
}

/// Serve one client connection until it closes.
fn serve(state: &Arc<Mutex<State>>, stream: TcpStream) {
    let _ = stream.set_nodelay(true);
    let Ok(writer) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let Some(client) = line
        .trim_end()
        .strip_prefix("attach ")
        .and_then(|client| client.parse::<usize>().ok())
    else {
        return;
    };
    {
        let mut state = state.lock().expect("the coordinator is not poisoned");
        let mut attached = Client {
            writer,
            sleeps: BTreeMap::new(),
            delayed: 0,
        };
        // A client number this coordinator was not started for, or one already attached, would
        // count toward the clients it waits for without being one of them: refused, and the
        // binary that sent it stops naming the answer.
        if client >= state.expected || state.clients.contains_key(&client) {
            attached.send("refused");
            return;
        }
        attached.send("attached");
        state.attached.insert(client);
        state.clients.insert(client, attached);
        state.advance();
    }
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let words: Vec<&str> = line.split_whitespace().collect();
        let mut state = state.lock().expect("the coordinator is not poisoned");
        let now = state.now;
        let Some(held) = state.clients.get_mut(&client) else {
            break;
        };
        match words.as_slice() {
            ["now", seq] => {
                if let Ok(seq) = seq.parse::<u64>() {
                    held.send(&format!("now {seq} {}", now.as_nanos()));
                }
            }
            ["sleep", seq, nanos] => {
                if let (Ok(seq), Ok(nanos)) = (seq.parse(), nanos.parse::<u64>()) {
                    held.sleeps.insert(seq, now + Duration::from_nanos(nanos));
                }
            }
            ["cancel", seq] => {
                if let Ok(seq) = seq.parse() {
                    held.sleeps.remove(&seq);
                }
            }
            _ => {}
        }
        state.advance();
    }
    let mut state = state.lock().expect("the coordinator is not poisoned");
    state.clients.remove(&client);
    state.disconnected.notify_all();
    state.advance();
}
