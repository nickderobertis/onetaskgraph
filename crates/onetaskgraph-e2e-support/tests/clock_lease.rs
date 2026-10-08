//! The opt-in lease at the coordinator's real socket boundary.
use onetaskgraph_e2e_support::clock::SimulatedClock;
use std::{
    io::{BufRead, BufReader, Write},
    net::TcpStream,
    time::Duration,
};

struct Client {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}
impl Client {
    fn attach(clock: &SimulatedClock, id: usize) -> Option<Self> {
        let (_, endpoint) = clock.client_env(id).remove(0);
        let address = endpoint.rsplit_once('/').unwrap().0;
        let writer = TcpStream::connect(address).unwrap();
        writer
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let reader = BufReader::new(writer.try_clone().unwrap());
        let mut client = Self { reader, writer };
        client.send(&format!("attach {id}"));
        (client.read() == "attached").then_some(client)
    }
    fn send(&mut self, line: &str) {
        writeln!(self.writer, "{line}").unwrap();
    }
    fn read(&mut self) -> String {
        let mut line = String::new();
        assert!(self.reader.read_line(&mut line).unwrap() > 0);
        line.trim().to_owned()
    }
}
#[test]
fn a_lease_holds_time_between_invocations_then_releases_a_finished_participant() {
    let clock = SimulatedClock::start(2);
    let lease = clock.lease(0);
    let first = Client::attach(&clock, 0).unwrap();
    let mut other = Client::attach(&clock, 1).unwrap();
    other.send("sleep 1 10000000000");
    other.send("now 2");
    assert_eq!(other.read(), "now 2 0");
    drop(first);
    lease.wait_for_detach();
    let next =
        Client::attach(&clock, 0).expect("the lease acknowledged the old connection's detach");
    assert_eq!(
        clock.now(),
        Duration::ZERO,
        "time advanced while the bundle driver was between invocations"
    );
    drop(next);
    drop(lease);
    assert_eq!(other.read(), "wake 1");
    assert_eq!(clock.now(), Duration::from_secs(10));
}
