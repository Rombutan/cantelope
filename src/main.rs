// DBC Parsing
use dbc_rs::Dbc;
use std::fs;

// Arrow IP elements
use arrow::datatypes::{Schema, Field, DataType};
use std::sync::Arc;

// Arrow parquet elements

// Custom argument parsing
pub mod args; 

// Custom Candump parsing
pub mod candumpparse;


// Used for type decisions only
trait FloatExt {
    fn is_nearly(&self, target: f64) -> bool;
}

impl FloatExt for f64 {
    fn is_nearly(&self, target: f64) -> bool {
        // Use a slightly larger margin than f64::EPSILON 
        // if you expect multiple cumulative calculations.
        (self - target).abs() < f64::EPSILON
    }
}

fn main() {
    let args = args::process_args(); // Load arguments into a struct 

    let dbc_content = fs::read_to_string(&args.dbcfile).unwrap(); // Load DBC file contents into string
    let dbc = Dbc::parse(&dbc_content).unwrap(); // Parse DBC

    // ------- CREATE SCHEMA
    let mut fields: Vec<Field> = Vec::new();
    
    fields.push(Field::new(
        "Time_ms",
        DataType::Float64,
        false, // Time is the only column that must exist in all rows.
    ));

    for message in dbc.messages().iter(){
        for signal in message.signals().iter(){
            println!();
            print!("{} : ", &signal.name());
            if(signal.length() == 1 
                && signal.is_unsigned() 
                && signal.factor().is_nearly(1.0) 
                && signal.offset().is_nearly(0.0)){
                print!("Boolean");
                continue;
            }
        }
    }
    let schema = Arc::new(Schema::new(fields));
    // ------


    let mut parser = candumpparse::CanDumpParser::new(&args.input).unwrap();

    let mut exit = false;
    while !exit { // Message recieve loop
        exit = parser.parse();
        if exit {continue;} // Prevents continued execution resulting in duplicated values when file is over

        println!(
            "Timestamp: {}, ID: {:X}, Data: {:?}",
            parser.get_timestamp(),
            parser.get_id(),
            parser.get_data()
        );
        match dbc.decode(parser.get_id(), &parser.get_data(), false) {
            Ok(decoded) => {
                println!("  Decoded {} signals:", decoded.len());
                for signal in decoded.iter() {
                    let unit_str = signal.unit.map(|u| format!(" {}", u)).unwrap_or_default();
                    println!("    {}: {}{}", signal.name, signal.value, unit_str);
                }
            }
            Err(e) => println!("  Error: {}", e),
        }
        
    }
}
