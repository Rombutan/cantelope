use std::fs::File;
use std::io::{BufRead, BufReader};

pub struct CanDumpParser {
    reader: BufReader<File>,
    current_timestamp: f64,
    current_id: u32,
    current_data: [u8; 8],
}

impl CanDumpParser {
    pub fn new(file_name: &str) -> std::io::Result<Self> {
        let file = File::open(file_name)?;
        Ok(Self {
            reader: BufReader::new(file),
            current_timestamp: 0.0,
            current_id: 0,
            current_data: [0u8; 8],
        })
    }

    /// Parse the next valid line.
    /// Returns true if EOF is reached or last line is malformed.
    pub fn parse(&mut self) -> bool {
        let mut line = String::new();

        loop {
            line.clear();
            let bytes_read = self.reader.read_line(&mut line).unwrap_or(0);
            if bytes_read == 0 {
                // EOF
                return true;
            }

            if let Some((ts, id, data)) = Self::parse_line(&line) {
                self.current_timestamp = ts;
                self.current_id = id;
                self.current_data = data;
                return false;
            }
            // Malformed line: skip to next
        }
    }

    fn parse_line(line: &str) -> Option<(f64, u32, [u8; 8])> {
        let line = line.trim();
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 {
            return None;
        }

        let ts_str = parts[0].trim_matches(|c| c == '(' || c == ')');
        let timestamp: f64 = ts_str.parse().ok()?;

        let id_data: Vec<&str> = parts[2].split('#').collect();
        if id_data.len() != 2 {
            return None;
        }

        let id = u32::from_str_radix(id_data[0], 16).ok()?;

        // Parse data, allow shorter than 8 bytes
        let data_str = id_data[1];
        let mut data = [0u8; 8];
        for i in 0..(data_str.len() / 2).min(8) {
            data[i] = u8::from_str_radix(&data_str[i * 2..i * 2 + 2], 16).ok()?;
        }

        Some((timestamp, id, data))
    }

    pub fn get_timestamp(&self) -> f64 {
        self.current_timestamp
    }

    pub fn get_id(&self) -> u32 {
        self.current_id
    }

    pub fn get_data(&self) -> [u8; 8] {
        self.current_data
    }
}

