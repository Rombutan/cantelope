use bytemuck::{Pod, Zeroable};
use std::io::Read;
use std::net::TcpStream;

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
    stream: TcpStream,
    // Private variables to hold the "last parsed" state
    timestamp: f64,
    id: u32,
    data: [u8; 8],
}

impl TcpWrapper {
    /// Connects to the TCP Relay server
    pub fn new(addr: &str) -> Self {
        let stream = TcpStream::connect(addr).expect("Failed to connect to CAN relay server");

        Self {
            stream,
            timestamp: 0.0,
            id: 0,
            data: [0; 8],
        }
    }

    /// Reads the next 24-byte frame from the network and updates internal state
    pub fn parse(&mut self) -> Result<(), std::io::Error> {
        let mut buffer = [0u8; std::mem::size_of::<CanFrame>()];

        // Read exactly 24 bytes
        self.stream.read_exact(&mut buffer)?;

        // Cast bytes back into our struct
        let frame: CanFrame = *bytemuck::from_bytes(&buffer);

        // Update private variables
        self.timestamp = frame.timestamp;
        self.id = frame.id;
        self.data = frame.data;

        Ok(())
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
