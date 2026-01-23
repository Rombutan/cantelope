// DBC Parsing
use dbc_rs::Dbc;
use std::fs;

// Custom argument parsing
pub mod args; 

fn main() {
    let args = args::process_args(); // Load arguments into a struct 

    let dbc_content = fs::read_to_string(&args.dbcfile).unwrap(); // Load DBC file contents into string
    let dbc = Dbc::parse(&dbc_content).unwrap(); // Parse DBC
}
