use super::{CanFrameData, CanInterface, Frame};
use bytemuck::{Pod, Zeroable};
use std::io::{self, ErrorKind, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const RX_TIMEOUT: Duration = Duration::from_secs(1);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
const RETRY_DELAY: Duration = Duration::from_millis(500);

// SocketCAN's extended-frame flag and 29-bit id mask
const EFF_FLAG: u32 = 0x8000_0000;
const EFF_MASK: u32 = 0x1FFF_FFFF;

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
    stream: Mutex<TcpStream>,
    reconnects: AtomicU64,
}

impl TcpWrapper {
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

    /// Replaces the stream in place, retrying until it succeeds.
    fn reconnect(&self, stream: &mut TcpStream) {
        loop {
            match Self::connect(&self.addr) {
                Ok(new_stream) => {
                    *stream = new_stream; // old stream (and any partial frame) is dropped
                    self.reconnects.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                Err(_) => sleep(RETRY_DELAY),
            }
        }
    }

    pub fn reconnect_count(&self) -> u64 {
        self.reconnects.load(Ordering::Relaxed)
    }
}

impl CanInterface for TcpWrapper {
    /// For TCP, the `interface` string is the relay address, e.g. "192.168.1.10:5000".
    fn new(interface: &str) -> Option<Self> {
        let stream = Self::connect(interface).ok()?;
        Some(Self {
            addr: interface.to_string(),
            stream: Mutex::new(stream),
            reconnects: AtomicU64::new(0),
        })
    }

    /// Reads the next 24-byte frame.
    /// If nothing arrives for 1s (or the link drops), reconnects and keeps waiting.
    fn parse(&self) -> Result<CanFrameData, Box<dyn std::error::Error>> {
        let mut buffer = [0u8; std::mem::size_of::<CanFrame>()];

        {
            let mut stream = self.stream.lock().map_err(|_| "TCP stream lock poisoned")?;
            loop {
                match stream.read_exact(&mut buffer) {
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
                        | ErrorKind::NotConnected => self.reconnect(&mut stream),
                        ErrorKind::Interrupted => continue,
                        _ => return Err(e.into()),
                    },
                }
            }
        } // lock released here so send() isn't blocked during conversion

        let time = Instant::now();

        // The u8 buffer isn't guaranteed 8-byte aligned, so use the unaligned read.
        let raw: CanFrame = bytemuck::pod_read_unaligned(&buffer);

        Ok(CanFrameData {
            frame: Frame {
                id: raw.id & EFF_MASK,
                extended: raw.id & EFF_FLAG != 0 || (raw.id & EFF_MASK) > 0x7FF,
                data: raw.data.to_vec(),
            },
            time,
        })
    }

    /// Sends a frame to the relay in the same 24-byte wire format.
    fn send(&self, frame: &Frame) {
        let id = if frame.extended {
            (frame.id & EFF_MASK) | EFF_FLAG
        } else {
            frame.id
        };

        let mut data = [0u8; 8];
        let len = frame.data.len().min(8);
        data[..len].copy_from_slice(&frame.data[..len]);

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);

        let out = CanFrame {
            timestamp,
            id,
            _pad: 0,
            data,
        };

        match self.stream.lock() {
            Ok(mut stream) => {
                if let Err(e) = stream.write_all(bytemuck::bytes_of(&out)) {
                    eprintln!("TCP send failed: {}", e);
                }
            }
            Err(_) => eprintln!("TCP stream lock poisoned"),
        }
    }
}
