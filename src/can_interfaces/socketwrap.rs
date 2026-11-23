use super::{CanFrameData, CanInterface, Frame};
use socketcan::{
    CanDataFrame, CanFrame, CanSocket, EmbeddedFrame, ExtendedId, Id, Socket, StandardId,
};
use std::time::Instant;

pub struct CanWrapper {
    socket: CanSocket,
}

impl CanInterface for CanWrapper {
    fn new(interface: &str) -> Option<Self> {
        let socket = CanSocket::open(interface)
            .map_err(|e| eprintln!("Failed to open CAN socket {}: {}", interface, e))
            .ok()?;
        Some(Self { socket })
    }

    /// Blocks the current thread until the next data frame is received
    fn parse(&self) -> Result<CanFrameData, Box<dyn std::error::Error>> {
        loop {
            if let CanFrame::Data(f) = self.socket.read_frame()? {
                let (id, extended) = match f.id() {
                    Id::Standard(s) => (u32::from(s.as_raw()), false),
                    Id::Extended(e) => (e.as_raw(), true),
                };
                return Ok(CanFrameData {
                    frame: Frame {
                        id,
                        extended,
                        data: f.data().to_vec(),
                    },
                    time: Instant::now(),
                });
            }
            // remote/error frames carry no signal data: keep waiting
        }
    }

    fn send(&self, frame: &Frame) {
        let id: Option<Id> = if frame.extended {
            ExtendedId::new(frame.id).map(Id::Extended)
        } else {
            u16::try_from(frame.id)
                .ok()
                .and_then(StandardId::new)
                .map(Id::Standard)
        };
        match id.and_then(|id| CanDataFrame::new(id, &frame.data)) {
            Some(f) => {
                if let Err(e) = self.socket.write_frame(&f) {
                    eprintln!("CAN socket send failed: {}", e);
                }
            }
            None => eprintln!("Invalid CAN id/payload for socket send"),
        }
    }
}
