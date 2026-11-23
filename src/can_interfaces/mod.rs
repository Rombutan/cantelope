#[cfg(feature = "socket")]
pub mod socketwrap;

#[cfg(feature = "peak-can")]
pub mod peakwrap;

pub mod filewrap;
pub mod tcpwrapper;
pub mod udpwrapper;

use std::time::Instant;

/// A classic CAN data frame, independent of any platform-specific CAN crate
/// (socketcan only builds on Linux, but PCAN/TCP/UDP/file inputs run anywhere).
#[derive(Clone, Debug)]
pub struct Frame {
    /// Raw id without any extended-frame flag bit.
    pub id: u32,
    pub extended: bool,
    pub data: Vec<u8>,
}

pub struct CanFrameData {
    pub frame: Frame,
    pub time: Instant,
}

/// Returned by `parse()` when a finite source (a file) has no more frames.
#[derive(Debug)]
pub struct EndOfInput;

impl std::fmt::Display for EndOfInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "end of input")
    }
}

impl std::error::Error for EndOfInput {}

pub trait CanInterface {
    fn new(interface: &str) -> Option<Self>
    where
        Self: Sized;
    /// Blocks until the next frame arrives.
    fn parse(&self) -> Result<CanFrameData, Box<dyn std::error::Error>>;
    fn send(&self, frame: &Frame);
}
