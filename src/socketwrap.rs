use socketcan::{CanFrame, CanSocket, EmbeddedFrame, Frame, Socket};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct CanWrapper {
    socket: CanSocket,
    last_timestamp: f64,
    last_id: u32,
    last_data: [u8; 8],
}

impl CanWrapper {
    /// Opens a new CAN socket in blocking mode
    pub fn new(interface: &str) -> Result<Self, socketcan::CanError> {
        let socket = CanSocket::open(interface).unwrap();
        Ok(Self {
            socket,
            last_timestamp: 0.0,
            last_id: 0,
            last_data: [0; 8],
        })
    }

    /// Blocks the current thread until the next packet is received
    pub fn parse(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let frame = self.socket.read_frame()?;

        let now = SystemTime::now().duration_since(UNIX_EPOCH)?;
        self.last_timestamp = now.as_secs_f64();

        if let CanFrame::Data(data_frame) = frame {
            // 1. Correctly extract the ID as a u32
            // .raw_id() from EmbeddedFrame returns the clean integer ID
            self.last_id = data_frame.raw_id();

            // 2. Correctly extract the data bytes
            let mut data = [0u8; 8];
            let frame_data = data_frame.data(); // Returns &[u8]

            // Ensure we don't out-of-bounds if the frame has < 8 bytes
            let len = frame_data.len().min(8);
            data[..len].copy_from_slice(&frame_data[..len]);

            self.last_data = data;
        }

        Ok(())
    }

    pub fn get_timestamp(&self) -> f64 {
        self.last_timestamp
    }

    pub fn get_id(&self) -> u32 {
        self.last_id
    }

    pub fn get_data(&self) -> [u8; 8] {
        self.last_data
    }
}
