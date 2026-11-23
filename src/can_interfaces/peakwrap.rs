use super::{CanFrameData, CanInterface, Frame};
use peak_can::bus::UsbBus;
use peak_can::socket::usb::UsbCanSocket;
use peak_can::socket::{Baudrate, CanFrame, MessageType, RecvCan, SendCan};
use std::thread::sleep;
use std::time::{Duration, Instant};

pub struct CanWrapper {
    socket: UsbCanSocket,
}

impl CanInterface for CanWrapper {
    /// For PCAN, the `interface` string is the baudrate, e.g. "500000".
    fn new(interface: &str) -> Option<Self> {
        let baud = match interface {
            "1000000" => Baudrate::Baud1M,
            "800000" => Baudrate::Baud800K,
            "500000" => Baudrate::Baud500K,
            "250000" => Baudrate::Baud250K,
            "125000" => Baudrate::Baud125K,
            "95000" => Baudrate::Baud95K,
            "83000" => Baudrate::Baud83K,
            "50000" => Baudrate::Baud50K,
            "47000" => Baudrate::Baud47K,
            "33000" => Baudrate::Baud33K,
            "20000" => Baudrate::Baud20K,
            "10000" => Baudrate::Baud10K,
            "5000" => Baudrate::Baud5K,
            _ => return None,
        };

        let socket = UsbCanSocket::open(UsbBus::USB1, baud).ok()?;
        Some(Self { socket })
    }

    /// Blocks until the next frame is received.
    fn parse(&self) -> Result<CanFrameData, Box<dyn std::error::Error>> {
        // peak-can's recv() is non-blocking, so poll until a frame arrives.
        let frame = loop {
            match self.socket.recv() {
                Ok((frame, _hw_timestamp)) => break frame,
                Err(_) => sleep(Duration::from_micros(500)),
            }
        };
        let time = Instant::now();

        Ok(CanFrameData {
            frame: Frame {
                id: frame.can_id(),
                extended: frame.is_extended_frame(),
                data: frame.data().to_vec(),
            },
            time,
        })
    }

    fn send(&self, frame: &Frame) {
        let msg_type = if frame.extended {
            MessageType::Extended
        } else {
            MessageType::Standard
        };

        match CanFrame::new(frame.id, msg_type, &frame.data) {
            Ok(pcan_frame) => {
                if let Err(e) = self.socket.send(pcan_frame) {
                    eprintln!("PCAN send failed: {:?}", e);
                }
            }
            Err(e) => eprintln!("Failed to build PCAN frame: {:?}", e),
        }
    }
}
