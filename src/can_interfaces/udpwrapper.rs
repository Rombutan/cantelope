use super::{CanFrameData, CanInterface, Frame};
use bytemuck::{Pod, Zeroable};
use socket2::{Domain, Protocol, Socket, Type};
use std::io::{self, ErrorKind};
use std::mem::size_of;
use std::net::{SocketAddr, UdpSocket};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const LOCAL_ADDR: &str = "0.0.0.0:1534";
const RX_TIMEOUT: Duration = Duration::from_secs(1);
const RETRY_DELAY: Duration = Duration::from_millis(500);

// SocketCAN's extended-frame flag and 29-bit id mask
const EFF_FLAG: u32 = 0x8000_0000;
const EFF_MASK: u32 = 0x1FFF_FFFF;

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct CanFrame {
    timestamp: f64,
    id: u32,
    _pad: u32,
    data: [u8; 8],
}

const FRAME_SIZE: usize = size_of::<CanFrame>();

pub struct UdpWrapper {
    socket: UdpSocket,
}

impl UdpWrapper {
    /// Binds the local port, connects to the relay and sends the hello message.
    fn open(addr: &str) -> io::Result<UdpSocket> {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
        #[cfg(not(windows))]
        {
            socket.set_reuse_address(true)?;
            socket.set_reuse_port(true)?;
        }
        let local: SocketAddr = LOCAL_ADDR
            .parse()
            .map_err(|e| io::Error::new(ErrorKind::InvalidInput, e))?;
        socket.bind(&local.into())?;

        let socket: UdpSocket = socket.into();
        socket.set_read_timeout(Some(RX_TIMEOUT))?;
        socket.connect(addr)?;
        socket.send(b"Please connecto bro")?;
        Ok(socket)
    }
}

impl CanInterface for UdpWrapper {
    /// For UDP, the `interface` string is the relay address, e.g. "192.168.1.10:5000".
    fn new(interface: &str) -> Option<Self> {
        let socket = Self::open(interface)
            .map_err(|e| eprintln!("UDP setup failed: {}", e))
            .ok()?;
        Some(Self { socket })
    }

    /// Blocks until a valid 24-byte frame arrives, pinging the relay every
    /// second of silence so it knows we're still listening.
    fn parse(&self) -> Result<CanFrameData, Box<dyn std::error::Error>> {
        // One byte larger than a frame, so oversized datagrams show up as a
        // wrong length instead of being silently truncated to 24 bytes.
        let mut buffer = [0u8; FRAME_SIZE + 1];

        loop {
            match self.socket.recv(&mut buffer) {
                Ok(FRAME_SIZE) => break,
                Ok(n) => eprintln!("Ignoring {}-byte datagram (expected {})", n, FRAME_SIZE),
                Err(e) => match e.kind() {
                    // Timeout (WouldBlock on Unix, TimedOut on Windows)
                    ErrorKind::WouldBlock | ErrorKind::TimedOut => {
                        if let Err(e) = self.socket.send(b"timeout-ping") {
                            eprintln!("Failed to ping relay: {}", e);
                        }
                    }
                    // The relay isn't listening yet (ICMP port unreachable): wait and retry
                    ErrorKind::ConnectionRefused | ErrorKind::ConnectionReset => sleep(RETRY_DELAY),
                    ErrorKind::Interrupted => continue,
                    _ => return Err(e.into()),
                },
            }
        }

        let time = Instant::now();

        // The u8 buffer isn't guaranteed 8-byte aligned, so use the unaligned read.
        let raw: CanFrame = bytemuck::pod_read_unaligned(&buffer[..FRAME_SIZE]);

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

        if let Err(e) = self.socket.send(bytemuck::bytes_of(&out)) {
            eprintln!("UDP send failed: {}", e);
        }
    }
}
