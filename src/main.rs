#[macro_use] extern crate log;
extern crate env_logger;
#[macro_use] extern crate futures;
#[macro_use] extern crate tokio_core;

// pub mod for now until the entire API is used internally
pub mod pool;

use std::env;
use std::io::{self, Read, Write};
use std::net::SocketAddr;

use futures::{Future, Poll};
use futures::stream::Stream;
use tokio_core::reactor::Core;
use tokio_core::net::{TcpListener, TcpStream};
use pool::Pool;


#[derive(Debug)]
enum ConnectionState {
    ClientReading,
    ClientWriting,
    ServerReading,
    ServerWriting,
}

#[must_use = "Must use Pipe"]
struct Pipe {
    client_addr: SocketAddr,
    server_addr: SocketAddr,
    client: TcpStream,
    server: TcpStream,
    state: ConnectionState,

    /// The buffer from the client to send to the server
    send_buf: Vec<u8>,

    /// The buffer from the server to send to the client
    recv_buf: Vec<u8>,
}

impl Future for Pipe {
    type Item = ();
    type Error = io::Error;

    fn poll(&mut self) -> Poll<(), io::Error> {
        trace!("Polling...");

        loop {

            match self.state {
                ConnectionState::ClientReading => {
                    loop {
                        trace!("Reading from {}", self.client_addr);

                        // TODO should really be a VecDequeue in case we read more than once
                        let bytes = try_nb!(self.client.read_to_end(&mut self.send_buf));
                        trace!("Read {} bytes from {}", bytes, self.client_addr);

                        if bytes == 0 {
                            self.state = ConnectionState::ServerWriting;
                            trace!("State switched to {:?}", self.state);
                            break;
                        }
                    }
                }

                ConnectionState::ServerWriting => {
                    trace!("Writing to {}", self.server_addr);
                    try_nb!(self.server.write_all(&mut self.send_buf));
                    trace!("Wrote {} bytes to {}", self.send_buf.len(), self.server_addr);

                    self.server.shutdown(::std::net::Shutdown::Write).expect("Failed to shutdown writes for server socket");
                    self.state = ConnectionState::ServerReading;
                    trace!("State switched to {:?}", self.state);
                }

                ConnectionState::ServerReading => {
                    loop {
                        trace!("Reading from {}", self.server_addr);

                        let bytes = try_nb!(self.server.read_to_end(&mut self.recv_buf));
                        trace!("Read {} bytes from {}", bytes, self.server_addr);

                        if bytes == 0 {
                            self.state = ConnectionState::ClientWriting;
                            trace!("State switched to {:?}", self.state);
                            break;
                        }
                    }
                }

                ConnectionState::ClientWriting => {
                    trace!("Writing to {}", self.client_addr);
                    try_nb!(self.client.write_all(&mut self.recv_buf));
                    trace!("Wrote {} bytes to {}", self.recv_buf.len(), self.client_addr);

                    self.state = ConnectionState::ClientReading;
                    trace!("State switched to {:?}", self.state);
                }
            }
        }
    }
}

fn main() {
    env_logger::init().unwrap();

    let addr = env::args().nth(1).unwrap_or("127.0.0.1:8080".to_string());
    let addr = addr.parse::<SocketAddr>().unwrap();

    let backend = env::args().nth(2).unwrap_or("127.0.0.1:12345".to_string());
    let mut pool = Pool::new(vec![backend]).unwrap();

    // Create the event loop that will drive this server
    let mut lp = Core::new().unwrap();
    let handle = lp.handle();
    let h2 = handle.clone();

    let s = TcpListener::bind(&addr, &handle.clone());

    // Create a TCP listener which will listen for incoming connections
    let listener = lp.run(futures::done(s)).unwrap();

    info!("Listening on: {}", addr);

    let proxy = listener.incoming().for_each(|(sock, addr)| {
        debug!("Incoming connection on {}", addr);

        let backend = pool.get().unwrap();

        // TODO turn this into a pool managed by raft
        let pipe = TcpStream::connect(&backend, &h2).and_then(move |server| {

            Box::new(Pipe {
                client_addr: addr,
                server_addr: backend,
                client: sock,
                server: server,
                state: ConnectionState::ClientReading,
                send_buf: Vec::new(),
                recv_buf: Vec::new(),
            })

        }).and_then(|_| {
            debug!("Finished proxying");
            futures::finished(())
        }).map_err(|e| {
            error!("Error trying proxy - {}", e);
            ()
        });

        // spawn expects Item=Async(()), Error=()
        handle.spawn(pipe);

        Ok(())
    });

    lp.run(proxy).unwrap();
}