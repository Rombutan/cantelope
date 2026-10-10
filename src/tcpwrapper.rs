use bytemuck::{Pod, Zeroable};
use std::io::{self, ErrorKind, Read};
use std::net::{TcpStream, ToSocketAddrs};
use std::thread::sleep;
use std::time::Duration;

const RX_TIMEOUT: Duration = Duration::from_secs(1);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
const RETRY_DELAY: Duration = Duration::from_millis(500);

// This must match the Relay's struct exactly
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct CanFrame {
    timestamp: f64,
    id: u32,
    _pad: u32,
    data: [u8; 8],
}

pub struct TcpWrapper {
    addr: String,
    stream: TcpStream,
    reconnects: u64,
    // Private variables to hold the "last parsed" state
    timestamp: f64,
    id: u32,
    data: [u8; 8],
}

impl TcpWrapper {
    /// Connects to the TCP Relay server (panics if the first connect fails,
    /// same as before; later drops are retried automatically).
    pub fn new(addr: &str) -> Self {
        let stream = Self::connect(addr).expect("Failed to connect to CAN relay server");

        Self {
            addr: addr.to_string(),
            stream,
            reconnects: 0,
            timestamp: 0.0,
            id: 0,
            data: [0; 8],
        }
    }

    /// One connection attempt with a bounded connect time and the RX timeout applied.
    fn connect(addr: &str) -> io::Result<TcpStream> {
        let mut last_err = io::Error::new(ErrorKind::InvalidInput, "no addresses resolved");
        for sock_addr in addr.to_socket_addrs()? {
            match TcpStream::connect_timeout(&sock_addr, CONNECT_TIMEOUT) {
                Ok(stream) => {
                    stream.set_read_timeout(Some(RX_TIMEOUT))?;
                    return Ok(stream);
                }
                Err(e) => last_err = e,
            }
        }
        Err(last_err)
    }

    /// Replaces the stream, retrying until it succeeds.
    fn reconnect(&mut self) {
        loop {
            match Self::connect(&self.addr) {
                Ok(stream) => {
                    self.stream = stream; // old stream (and any partial frame) is dropped
                    self.reconnects += 1;
                    return;
                }
                Err(_) => sleep(RETRY_DELAY),
            }
        }
    }

    /// Reads the next 24-byte frame and updates internal state.
    /// If nothing arrives for 1s (or the link drops), reconnects and keeps waiting.
    pub fn parse(&mut self) -> Result<Vec<u8>, io::Error> {
        let mut buffer = [0u8; std::mem::size_of::<CanFrame>()];

        loop {
            match self.stream.read_exact(&mut buffer) {
                Ok(()) => break,
                Err(e) => match e.kind() {
                    // Timeout (WouldBlock on Unix, TimedOut on Windows), peer closed,
                    // or a connection-level error: framing is unknown, so start fresh.
                    ErrorKind::WouldBlock
                    | ErrorKind::TimedOut
                    | ErrorKind::UnexpectedEof
                    | ErrorKind::ConnectionReset
                    | ErrorKind::ConnectionAborted
                    | ErrorKind::BrokenPipe
                    | ErrorKind::NotConnected => self.reconnect(),
                    ErrorKind::Interrupted => continue,
                    _ => return Err(e),
                },
            }
        }

        // The u8 buffer isn't guaranteed 8-byte aligned, so use the unaligned read.
        let frame: CanFrame = bytemuck::pod_read_unaligned(&buffer);

        self.timestamp = frame.timestamp;
        self.id = frame.id;
        self.data = frame.data;

        Ok(Vec::from(buffer))
    }

    pub fn reconnect_count(&self) -> u64 {
        self.reconnects
    }

    pub fn get_timestamp(&self) -> f64 {
        self.timestamp
    }

    pub fn get_id(&self) -> u32 {
        self.id
    }

    pub fn get_data(&self) -> [u8; 8] {
        self.data
    }
}
