use peak_can::bus::UsbBus;
use peak_can::socket::usb::UsbCanSocket;
use peak_can::socket::{Baudrate, RecvCan};
use std::thread::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub struct CanWrapper {
    socket: UsbCanSocket,
    last_timestamp: f64,
    last_id: u32,
    last_data: [u8; 8],
}

impl CanWrapper {
    /// Opens a PCAN-USB channel at the given baudrate
    pub fn new(baudrate: String) -> Result<Self, Box<dyn std::error::Error>> {
        let mut baud = Baudrate::Baud1M;
        match baudrate.as_str() {
            "1000000" => {
                baud = Baudrate::Baud1M;
            }
            "800000" => {
                baud = Baudrate::Baud800K;
            }
            "500000" => {
                baud = Baudrate::Baud500K;
            }
            "250000" => {
                baud = Baudrate::Baud250K;
            }
            "125000" => {
                baud = Baudrate::Baud125K;
            }
            "95000" => {
                baud = Baudrate::Baud95K;
            }
            "83000" => {
                baud = Baudrate::Baud83K;
            }
            "50000" => {
                baud = Baudrate::Baud50K;
            }
            "47000" => {
                baud = Baudrate::Baud47K;
            }
            "33000" => {
                baud = Baudrate::Baud33K;
            }
            "20000" => {
                baud = Baudrate::Baud20K;
            }
            "10000" => {
                baud = Baudrate::Baud10K;
            }
            "5000" => {
                baud = Baudrate::Baud5K;
            }
            _ => {
                panic!("Baudrate illegal");
            }
        }

        let socket = UsbCanSocket::open(UsbBus::USB1, baud)
            .map_err(|e| format!("Failed to open PCAN socket: {:?}", e))?;
        Ok(Self {
            socket,
            last_timestamp: 0.0,
            last_id: 0,
            last_data: [0; 8],
        })
    }

    /// Blocks the current thread until the next frame is received
    pub fn parse(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // peak-can's recv() is non-blocking: it returns Err when the receive
        // queue is empty, so poll until a frame arrives.
        let frame = loop {
            match self.socket.recv() {
                Ok((frame, _hw_timestamp)) => break frame,
                Err(_) => sleep(Duration::from_micros(500)),
            }
        };

        // Wall-clock timestamp (same behaviour as the socketcan version)
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?;
        self.last_timestamp = now.as_secs_f64();

        self.last_id = frame.can_id();

        let frame_data = frame.data();
        let mut data = [0u8; 8];
        let len = frame_data.len().min(8);
        data[..len].copy_from_slice(&frame_data[..len]);
        self.last_data = data;

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
