// DBC Parsing
use dbc_rs::Dbc;
use std::fs;

// Arrow IP elements
use arrow::datatypes::{DataType, Field, Schema};
use std::sync::Arc;

// Literally only for cleaner print
use std::io::{self, Write};

// Custom data storage helpers
pub mod store;
use store::{Column, GenericColumn};

// Custom argument parsing
pub mod args;

// Custom Candump parsing
use candump::CanDumpParser;

use crate::args::CanDataInput;

// SocketCAN
#[cfg(feature = "socket")]
pub mod socketwrap;

// Use ctrl+c as exit signal in stdin and socket mode
use std::sync::atomic::{AtomicBool, Ordering};

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
    let mut base_row_size = 0; // Just to generate a cool "uncompressed data rate" number
    let mut fields: Vec<Field> = Vec::new(); // vec of column descriptions, later dumped into Arc<Schema>
    let mut columns: Vec<GenericColumn> = Vec::new(); // This vec actually stores the values
    let mut is_filled: Vec<bool> = Vec::new(); // This will keep track of which values have been filled so ones which haven't can be null balanced

    fields.push(Field::new(
        "Time_ms",
        DataType::Float64,
        false, // Time is the only column that must exist in all rows.
    ));

    columns.push(GenericColumn::F64(Column::new())); // Column for time

    is_filled.push(true); // This element of the map won't actually be used, but is needed for indecies to align

    for message in dbc.messages().iter() {
        for signal in message.signals().iter() {
            is_filled.push(false); // If I ever update this to exclude ANY signals which are present in the DBC, I will need to move this into the blocks below
            if signal.length() == 1
                && signal.is_unsigned()
                && signal.factor().is_nearly(1.0)
                && signal.offset().is_nearly(0.0)
            {
                // Definetely a boolean
                base_row_size += 1;
                fields.push(Field::new(signal.name(), DataType::Boolean, true));
                columns.push(GenericColumn::Bool(Column::new()));
            } else if (signal.factor() % 1.0).is_nearly(1.0) {
                // Definetely an integer
                if signal.min() >= f64::from(i8::MIN) && signal.max() <= f64::from(i8::MAX) {
                    // Fits in i8
                    base_row_size += 8;
                    fields.push(Field::new(signal.name(), DataType::Int8, true));
                    columns.push(GenericColumn::I8(Column::new()));
                } else if signal.min() >= f64::from(i16::MIN) && signal.max() <= f64::from(i16::MAX)
                {
                    // Fits in i16
                    base_row_size += 16;
                    fields.push(Field::new(signal.name(), DataType::Int16, true));
                    columns.push(GenericColumn::I16(Column::new()));
                } else if signal.min() >= f64::from(i32::MIN) && signal.max() <= f64::from(i32::MAX)
                {
                    // Fits in i32
                    base_row_size += 32;
                    fields.push(Field::new(signal.name(), DataType::Int32, true));
                    columns.push(GenericColumn::I32(Column::new()));
                } else {
                    // must fits in i64 :shrug
                    base_row_size += 64;
                    fields.push(Field::new(signal.name(), DataType::Int64, true));
                    columns.push(GenericColumn::I64(Column::new()));
                }
            } else {
                // Float
                //                if signal.min() >= f64::from(f16::MIN) && signal.max() <= f64::from(f16::MAX) {   // Fits in f16 (Currently only works in rust-unstable
                //                    print!("f16");
                //                    base_row_size+=16;
                //                    fields.push(Field::new(signal.name(), DataType::Float16, true));
                //                }

                if signal.min() >= f64::from(f32::MIN) && signal.max() <= f64::from(f32::MAX) {
                    base_row_size += 32;
                    fields.push(Field::new(signal.name(), DataType::Float32, true));
                    columns.push(GenericColumn::F32(Column::new()));
                } else {
                    // Must fits in f64 :shrug
                    base_row_size += 64;
                    fields.push(Field::new(signal.name(), DataType::Float64, true));
                    columns.push(GenericColumn::F64(Column::new()));
                }
            }
        }
    }
    println!("\nBasis row size: {} bits", base_row_size);
    let schema = Arc::new(Schema::new(fields));
    // ------

    let mut parser = CanDumpParser::new(&String::default()).unwrap();

    #[cfg(feature = "socket")]
    let mut cansocket: Option<socketwrap::CanWrapper> = None;
    let stdin = io::stdin();

    let time_start;

    match &args.candatainput {
        CanDataInput::File => {
            _ = parser = CanDumpParser::new(&args.input).unwrap();
            parser.parse();
            time_start = parser.get_timestamp();
        }
        CanDataInput::Stdin => {
            let mut nextline = String::new();
            stdin.read_line(&mut nextline).unwrap();
            _ = parser.parse_string(nextline);
            time_start = parser.get_timestamp();
        }
        #[cfg(feature = "socket")]
        CanDataInput::Socket => {
            cansocket = Some(socketwrap::CanWrapper::new(&args.input).unwrap());
            _ = cansocket.as_mut().unwrap().parse();
            time_start = cansocket.as_mut().unwrap().get_timestamp();
        }
        #[cfg(not(feature = "socket"))]
        CanDataInput::Socket => {
            panic!("Socketcan not enabled in this build")
        }
    }

    let exit = Arc::new(AtomicBool::new(false));
    let ex = exit.clone();

    ctrlc::set_handler(move || {
        println!("\nShutdown signal received...");
        ex.store(true, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    let mut num_chunks = 0;
    while !exit.load(Ordering::SeqCst) {
        // Message recieve loop
        let timestamp;
        let id;
        let data;
        match &args.candatainput {
            CanDataInput::File => {
                exit.store(parser.parse(), Ordering::SeqCst);

                timestamp = parser.get_timestamp();
                id = parser.get_id();
                data = parser.get_data();
            }
            CanDataInput::Stdin => {
                let mut nextline = String::new();
                stdin.read_line(&mut nextline).unwrap();
                exit.store(parser.parse_string(nextline), Ordering::SeqCst);

                timestamp = parser.get_timestamp();
                id = parser.get_id();
                data = parser.get_data();
            }
            #[cfg(feature = "socket")]
            CanDataInput::Socket => {
                cansocket.as_mut().unwrap().parse().unwrap();
                timestamp = cansocket.as_mut().unwrap().get_timestamp();
                id = cansocket.as_mut().unwrap().get_id();
                data = cansocket.as_mut().unwrap().get_data();
            }
            #[cfg(not(feature = "socket"))]
            CanDataInput::Socket => {
                panic!("Socketcan not yet supported")
            }
        }
        //if exit {continue;} // Prevents continued execution resulting in duplicated values when file is over, breaks finishing of last row

        let relative_time_rcv = (timestamp - time_start) * 1000.0; // time since start of recording

        match dbc.decode(id, &data, false) {
            Ok(decoded) => {
                for signal in decoded.iter() {
                    let col = &mut columns[schema.index_of(signal.name).unwrap()];
                    if !is_filled[schema.index_of(signal.name).unwrap()] {
                        // Only save the first value from each chunk (as opposed to prev version saving last)
                        match col {
                            GenericColumn::Bool(c) => c.push(Some(signal.value.is_nearly(1.0))),
                            GenericColumn::I8(c) => c.push(Some(signal.value as i8)),
                            GenericColumn::I32(c) => c.push(Some(signal.value as i32)),
                            GenericColumn::I64(c) => c.push(Some(signal.value as i64)),
                            //                            GenericColumn::F16(c) => c.push(Some(f16::from(signal.value))),
                            GenericColumn::F32(c) => c.push(Some(signal.value as f32)),
                            GenericColumn::F64(c) => c.push(Some(signal.value)),
                            _ => {}
                        }
                        is_filled[schema.index_of(signal.name).unwrap()] = true;
                    }
                }
            }
            Err(e) => println!("Signal: {} Data: {:02x?}  Error: {}", id, &data, e),
            //Err(e) => _ = e,
        }
        if relative_time_rcv > (&args.cache_ms * f64::from(num_chunks))
            || exit.load(Ordering::SeqCst)
        {
            let col = &mut columns[schema.index_of("Time_ms").unwrap()];
            is_filled[schema.index_of("Time_ms").unwrap()] = true;
            match col {
                GenericColumn::F64(c) => c.push(Some(relative_time_rcv)),
                _ => {}
            }
            let mut empty_cols = 0;
            for (index, value) in is_filled.iter().enumerate() {
                if !value {
                    columns[index].push_null();
                    empty_cols += 1;
                }
            }
            num_chunks += 1;
            is_filled.fill(false);
            if num_chunks % 250 == 0 {
                print!("\rRow #{} with {} empty fields", num_chunks, empty_cols);
                io::stdout().flush().unwrap();
            }
        }
    }
    println!("");
    let batch = store::finish_record_batch(columns, schema);
    store::write_record_batch_to_parquet(&batch, &args.output).unwrap();
}
