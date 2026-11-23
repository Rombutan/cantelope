use super::{CanFrameData, CanInterface, EndOfInput, Frame};
use candump::CanDumpParser;
use std::sync::Mutex;
use std::time::{Duration, Instant};

struct State {
    parser: CanDumpParser,
    /// Wall-clock anchor: the first frame in the file maps to this instant.
    base: Instant,
    first_timestamp: Option<f64>,
    at_end: bool,
}

/// Replays a candump log through the `CanInterface` trait.
/// Recorded timestamps are mapped onto `Instant`s relative to when the file was opened.
pub struct FileWrapper {
    state: Mutex<State>,
}

impl CanInterface for FileWrapper {
    /// For files, the `interface` string is the path to the candump log.
    fn new(interface: &str) -> Option<Self> {
        Some(Self {
            state: Mutex::new(State {
                parser: CanDumpParser::new(interface).unwrap(),
                base: Instant::now(),
                first_timestamp: None,
                at_end: false,
            }),
        })
    }

    /// Returns the next logged frame, then `EndOfInput` once the file is exhausted.
    fn parse(&self) -> Result<CanFrameData, Box<dyn std::error::Error>> {
        let mut s = self.state.lock().map_err(|_| "file parser lock poisoned")?;
        if s.at_end {
            return Err(Box::new(EndOfInput));
        }

        // The parser reports end-of-file together with the final frame, so deliver
        // that frame now and report the end on the next call.
        if s.parser.parse() {
            s.at_end = true;
        }

        let timestamp = s.parser.get_timestamp();
        let first = *s.first_timestamp.get_or_insert(timestamp);
        let offset = (timestamp - first).max(0.0);
        let time = s.base
            + if offset.is_finite() {
                Duration::from_secs_f64(offset)
            } else {
                Duration::ZERO
            };

        let id = s.parser.get_id();
        let data = s.parser.get_data().to_vec();

        Ok(CanFrameData {
            frame: Frame {
                id,
                extended: id > 0x7FF,
                data,
            },
            time,
        })
    }

    fn send(&self, _frame: &Frame) {
        eprintln!("You can't send data into a file!");
    }
}
