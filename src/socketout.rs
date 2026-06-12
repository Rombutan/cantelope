use socketcan::{CanFrame, CanSocket, EmbeddedFrame, ExtendedId, Frame, Id, Socket, StandardId};

pub struct CanWrapper {
    socket: CanSocket,
}

impl CanWrapper {
    /// Opens a new CAN socket in blocking mode.
    pub fn new(interface: &str) -> Result<Self, socketcan::CanError> {
        let socket = CanSocket::open(interface)?;
        Ok(Self { socket })
    }

    /// Sends a CAN frame with either a standard (11-bit)
    /// or extended (29-bit) identifier.
    pub fn send(&mut self, id: u32, data: &[u8]) -> Result<(), socketcan::CanError> {
        let id = if id <= 0x7FF {
            Id::Standard(StandardId::new(id as u16).expect("invalid standard CAN ID"))
        } else {
            Id::Extended(ExtendedId::new(id).expect("invalid extended CAN ID"))
        };

        let frame = CanFrame::new(id, data).expect("invalid payload length");

        self.socket.write_frame(&frame)
    }
}
