// DBC Parsing
use dbc_rs::{Dbc, Message};
use std::fs;

// Arrow IP elements
use arrow::datatypes::{DataType, Field, Schema};
use parquet::arrow::ArrowWriter;
use parquet::file::properties::WriterProperties;
use std::fs::File;

// Literally only for cleaner print
use std::io::{self, Write};

// Custom data storage helpers
pub mod store;
use std::collections::HashMap;
use store::{Column, GenericColumn, MessageSend, TableStore, load_parquet};

// Custom argument parsing
pub mod args;
use std::env;
use std::sync::Arc;

use crate::args::Args;
use crate::args::CanDataInput;

mod can_interfaces;
use can_interfaces::{CanFrameData, CanInterface, EndOfInput, Frame};

// Use ctrl+c as exit signal in socket mode
use std::sync::atomic::{AtomicBool, Ordering};
use std::{process, thread, time};

#[cfg(feature = "plot")]
pub mod plot;

#[cfg(feature = "plot")]
use plot::PlotWindow;

#[cfg(not(feature = "plot"))]
pub type DataPoint = (String, f64, f64); // (signal, x, y)

use rfd::FileDialog;

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

/// One received frame: time in seconds, raw id, payload.
struct RxFrame {
    timestamp: f64,
    id: u32,
    extended: bool,
    data: Vec<u8>,
}

/// Every input (file, SocketCAN, PCAN, TCP, UDP) implements `CanInterface`.
fn open_interface<T: CanInterface + 'static>(iface: &str, what: &str) -> Box<dyn CanInterface> {
    Box::new(T::new(iface).unwrap_or_else(|| panic!("Failed to open {what}")))
}

/// Blocks until the next data frame; None once the input is exhausted (end of file).
/// `epoch` converts the interface's `Instant`s to seconds.
fn next_frame(input: &dyn CanInterface, epoch: time::Instant) -> Option<RxFrame> {
    loop {
        match input.parse() {
            Ok(CanFrameData { frame, time }) => {
                return Some(RxFrame {
                    timestamp: time.duration_since(epoch).as_secs_f64(),
                    id: frame.id,
                    extended: frame.extended,
                    data: frame.data,
                });
            }
            Err(e) if e.is::<EndOfInput>() => return None,
            Err(e) => {
                println!("CAN receive error: {}", e);
                thread::sleep(time::Duration::from_millis(1)); // don't spin on a dead link
            }
        }
    }
}

fn main() {
    std::thread::spawn(|| {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(5));
            let deadlocks = parking_lot::deadlock::check_deadlock();
            if deadlocks.is_empty() {
                continue;
            }
            // Write to a file, not the console, in case the console is what's stuck.
            let mut f = std::fs::File::create("deadlock.txt").unwrap();
            for (i, threads) in deadlocks.iter().enumerate() {
                writeln!(f, "Deadlock #{i}").unwrap();
                for t in threads {
                    writeln!(f, "Thread {:?}\n{:#?}", t.thread_id(), t.backtrace()).unwrap();
                }
            }
        }
    });

    let mut start_args: Args;

    if env::args().len() == 1 {
        println!("Missing args, looking for config file");
        let files = FileDialog::new()
            .add_filter("toml", &["toml"])
            .set_title("Choose Config File")
            .pick_file();
        start_args = toml::from_str(
            &fs::read_to_string(
                files
                    .expect("File picking failed")
                    .as_path()
                    .to_str()
                    .expect("Config path UTF-8 check failed?"),
            )
            .expect("Could not read file"),
        )
        .expect("Could not parse toml config file. Check formatting.");
        start_args.setup_aux_outputs();
    } else {
        start_args = args::process_args();
    }

    let parquet_input = matches!(start_args.candatainput, CanDataInput::Parquet);

    if start_args.dbcfile == "".to_string() && !parquet_input {
        let files = FileDialog::new()
            .add_filter("dbc", &["dbc", "DBC"])
            .set_title("Choose DBC File")
            .pick_file();
        start_args.dbcfile = files
            .expect("DBF File picker failed")
            .as_path()
            .to_str()
            .expect("DBC path UTF-8 check failed?")
            .to_string();
    }

    let args = start_args;

    let dbc_content = if parquet_input {
        String::new()
    } else {
        fs::read_to_string(&args.dbcfile).expect("Could not read DBC file") // Load DBC file contents into string
    };

    let args_en_aux = args.en_aux;

    let args_data_thread = args.clone();

    let store = TableStore::new();

    let data_store = store.clone_ref();
    let handle = std::thread::spawn(move || {
        if matches!(args_data_thread.candatainput, CanDataInput::Parquet) {
            parquet_in(&args_data_thread, data_store);
            return;
        }
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async move {
                data_loop(args_data_thread, &dbc_content, data_store).await;
            });
    });

    #[cfg(feature = "plot")]
    {
        if args_en_aux {
            let plot_store_clone = store;
            println!("Starting plotting window!");
            _ = PlotWindow::run(plot_store_clone, args);
        }
    }

    #[cfg(not(feature = "end_data_on_close"))]
    let _ = handle.join();
}

/// Parses the DBC, or prints where it failed and exits.
fn parse_dbc(dbc_content: &str) -> Dbc {
    match Dbc::parse(dbc_content) {
        Ok(v) => {
            println!("DBC Succesfully parsed!");
            v
        }
        Err(v) => {
            let line: String;
            match v.line() {
                Some(v) => {
                    line = format!("{}", v);
                }
                None => {
                    line = format!("None");
                }
            }
            println!("DBC Parse failed :(\n Line: {}", line);
            use dbc_rs::Error::*;
            match v {
                UnexpectedEof { line: _ } => {
                    println!("  Unexpected EOF");
                }
                Expected { msg, line: _ } => {
                    println!("  {}", msg);
                }
                InvalidChar { char, line: _ } => {
                    println!("  Invalid character: {}", char);
                }
                MaxStrLength { max, line: _ } => {
                    println!("  String exceeds maximum length of {}", max);
                }
                Nodes { msg, line: _ }
                | Signal { msg, line: _ }
                | Receivers { msg, line: _ }
                | Message { msg, line: _ }
                | Version { msg, line: _ } => {
                    println!("  {}", msg);
                }
                Decoding(msg) | Encoding(msg) | Validation(msg) => {
                    println!("  {}", msg);
                }
                Io(msg) => {
                    println!("  {}", msg);
                }
            }
            std::process::exit(78);
        }
    }
}

/// Messages grouped by the node that sends them (used by plot).
fn build_nodes(dbc: &Dbc) -> HashMap<String, Vec<Message>> {
    let mut nodes: HashMap<String, Vec<Message>> = Default::default();
    for node in dbc.nodes().iter() {
        if nodes.insert(node.to_string(), vec![]).is_some() {
            panic!("Duplicate node: {}", node);
        }
    }
    for message in dbc.messages().iter() {
        nodes
            .entry(message.sender().to_string())
            .or_default()
            .push(message.clone());
    }
    nodes
}

/// Node that holds every signal when loading parquet (there is no DBC to say who sends what).
const PARQUET_NODE: &str = "VECTOR_XXX";

fn is_dbc_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// One placeholder node containing a one-signal message per parquet column, so the signal
/// tables still have something to show. The messages come from a throwaway in-memory DBC
/// (not the user's DBC file); they can't be sent or decoded.
fn placeholder_nodes(signal_names: &[String]) -> HashMap<String, Vec<Message>> {
    let mut text = format!("VERSION \"\"\n\nNS_ :\n\nBS_:\n\nBU_: {PARQUET_NODE}\n\n");
    let mut id: u32 = 0;
    for name in signal_names.iter().filter(|n| is_dbc_identifier(n)) {
        id += 1;
        text.push_str(&format!(
            "BO_ {} {name}: 1 {PARQUET_NODE}\n SG_ {name} : 0|1@1+ (1,0) [0|1] \"\" Vector__XXX\n\n",
            id | 0x8000_0000 // extended, so there's no 2047 message limit
        ));
    }

    match Dbc::parse(&text) {
        Ok(dbc) => build_nodes(&dbc),
        Err(e) => {
            println!("Could not build placeholder signal list: {:?}", e);
            HashMap::from([(PARQUET_NODE.to_string(), vec![])])
        }
    }
}

/// Parquet input: loads a whole parquet file into the store instead of running the data loop.
/// No DBC is read; every signal is filed under one placeholder node.
fn parquet_in(args: &args::Args, store: TableStore) {
    let file = File::open(&args.input).unwrap_or_else(|e| {
        println!("Could not open parquet file {}: {}", args.input, e);
        std::process::exit(74);
    });

    let schema = {
        let mut write_cols = store.write_columns();
        load_parquet(file, &mut write_cols).unwrap_or_else(|e| {
            println!("Could not read parquet file {}: {}", args.input, e);
            std::process::exit(74);
        })
    };

    {
        let mut write_signalmap = store.signalmap.write();
        write_signalmap.clear();
        for (index, field) in schema.fields().iter().enumerate() {
            write_signalmap.insert(field.name().to_string(), index);
        }
    }

    // Column 0 is the time column, everything after it is a signal.
    let signal_names: Vec<String> = schema
        .fields()
        .iter()
        .skip(1)
        .map(|f| f.name().to_string())
        .collect();
    *store.nodes.write() = placeholder_nodes(&signal_names);

    let rows = store.read_columns().first().map_or(0, |c| c.len());
    println!(
        "Loaded {} rows and {} signals from parquet",
        rows,
        signal_names.len()
    );

    store.set_ready();
}

async fn data_loop(args: args::Args, dbc_content: &String, store: TableStore) {
    let dbc = parse_dbc(dbc_content);

    // ------- CREATE SCHEMA
    let mut base_row_size = 0; // Just to generate a cool "uncompressed data rate" number
    let mut fields: Vec<Field> = Vec::new(); // vec of column descriptions, later dumped into Arc<Schema>
    let mut is_filled: Vec<bool> = Vec::new(); // This will keep track of which values have been filled so ones which haven't can be null balanced

    let mut write_signalmap = store.signalmap.write();

    let mut column_names = vec!["Time_ms"];

    fields.push(Field::new(
        "Time_ms",
        DataType::Float64,
        false, // Time is the let mut index_counter: usize = 0;only column that must exist in all rows.
    ));
    write_signalmap.insert("Time_ms".to_string(), 0);

    store
        .write_columns()
        .push(GenericColumn::F64(Column::new())); // Column for time

    is_filled.push(true); // This element of the map won't actually be used, but is needed for indecies to align. Corresponds to Time_ms

    let mut nodes: HashMap<String, Vec<Message>> = Default::default(); // This will store messages in association with the node they are from, for use by plot. Store and schema are still flat.
    for node in dbc.nodes().iter() {
        println!("{}", node);
        if nodes.insert(node.to_string(), vec![]).is_some() {
            panic!("Duplicate node: {}", node);
        }
    }

    let mut index_counter: usize = 0;
    for message in dbc.messages().iter() {
        nodes
            .entry(message.sender().to_string())
            .or_default()
            .push(message.clone());
        for signal in message.signals().iter() {
            index_counter += 1;
            write_signalmap.insert(signal.name().to_string(), index_counter);
            is_filled.push(false); // If I ever update this to exclude ANY signals which are present in the DBC, I will need to move this into the blocks below
            column_names.push(signal.name());
            if signal.length() == 1
                && signal.is_unsigned()
                && signal.factor().is_nearly(1.0)
                && signal.offset().is_nearly(0.0)
            {
                // Definetely a boolean
                base_row_size += 1;
                fields.push(Field::new(signal.name(), DataType::Boolean, true));
                store
                    .write_columns()
                    .push(GenericColumn::Bool(Column::new()));
            } else if (signal.factor() % 1.0).is_nearly(1.0) {
                // Definetely an integer
                if signal.min() >= f64::from(i8::MIN) && signal.max() <= f64::from(i8::MAX) {
                    // Fits in i8
                    base_row_size += 8;
                    fields.push(Field::new(signal.name(), DataType::Int8, true));
                    store.write_columns().push(GenericColumn::I8(Column::new()));
                } else if signal.min() >= f64::from(i16::MIN) && signal.max() <= f64::from(i16::MAX)
                {
                    // Fits in i16
                    base_row_size += 16;
                    fields.push(Field::new(signal.name(), DataType::Int16, true));
                    store
                        .write_columns()
                        .push(GenericColumn::I16(Column::new()));
                } else if signal.min() >= f64::from(i32::MIN) && signal.max() <= f64::from(i32::MAX)
                {
                    // Fits in i32
                    base_row_size += 32;
                    fields.push(Field::new(signal.name(), DataType::Int32, true));
                    store
                        .write_columns()
                        .push(GenericColumn::I32(Column::new()));
                } else {
                    // must fits in i64 :shrug
                    base_row_size += 64;
                    fields.push(Field::new(signal.name(), DataType::Int64, true));
                    store
                        .write_columns()
                        .push(GenericColumn::I64(Column::new()));
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
                    store
                        .write_columns()
                        .push(GenericColumn::F32(Column::new()));
                } else {
                    // Must fits in f64 :shrug
                    base_row_size += 64;
                    fields.push(Field::new(signal.name(), DataType::Float64, true));
                    store
                        .write_columns()
                        .push(GenericColumn::F64(Column::new()));
                }
            }
        }
    }

    let mut _nodes = store.nodes.write();
    *_nodes = nodes;
    drop(_nodes);

    drop(write_signalmap);
    println!("\nBasis row size: {} bits", base_row_size);

    let schema = Arc::new(Schema::new(fields));

    println!("Schema created");

    // Interface frames carry an `Instant`; measure them from here (before opening the input).
    let epoch = time::Instant::now();

    let input = match &args.candatainput {
        CanDataInput::Parquet => unreachable!("parquet input is handled by parquet_in"),
        CanDataInput::File => {
            open_interface::<can_interfaces::filewrap::FileWrapper>(&args.input, "input file")
        }
        #[cfg(feature = "socket")]
        CanDataInput::Socket => {
            open_interface::<can_interfaces::socketwrap::CanWrapper>(&args.input, "CAN socket")
        }
        #[cfg(not(feature = "socket"))]
        CanDataInput::Socket => panic!("Socketcan not enabled in this build"),
        #[cfg(feature = "peak-can")]
        CanDataInput::Peak => open_interface::<can_interfaces::peakwrap::CanWrapper>(
            &args.peakbaudrate.to_string(),
            "PCAN interface",
        ),
        #[cfg(not(feature = "peak-can"))]
        CanDataInput::Peak => panic!("Peak-Can not enabled in this build"),
        CanDataInput::TcpRemote => {
            open_interface::<can_interfaces::tcpwrapper::TcpWrapper>(&args.input, "TCP relay")
        }
        CanDataInput::UdpRemote => {
            open_interface::<can_interfaces::udpwrapper::UdpWrapper>(&args.input, "UDP relay")
        }
    };

    println!("Input created!");

    #[cfg(feature = "socket")]
    let outsocket = if args.socketout.is_empty() {
        None
    } else {
        Some(open_interface::<can_interfaces::socketwrap::CanWrapper>(
            &args.socketout,
            "output CAN socket",
        ))
    };

    let exit = Arc::new(AtomicBool::new(false));
    let ex = exit.clone();

    let time_start = next_frame(&*input, epoch).map_or(0.0, |f| f.timestamp);

    #[cfg(feature = "not_wasm")]
    ctrlc::set_handler(move || {
        println!("\nShutdown signal received...");
        ex.store(true, Ordering::SeqCst);
        thread::sleep(time::Duration::from_millis(200));
        // Possible hazard with very large data blocks while writting output to
        // parquet, or when input frequency is very low. these compound as well.
        process::exit(1);
    })
    .expect("Error setting Ctrl-C handler");

    let mut num_chunks = 1;
    let mut last_time_ms: f64 = 0.0;
    while !exit.load(Ordering::SeqCst) {
        let mut offset: f64 = 0.0;

        // Message recieve loop
        let rx = next_frame(&*input, epoch);
        if rx.is_none() {
            exit.store(true, Ordering::SeqCst); // end of input: still flush the last row below
        }
        if let Some(RxFrame {
            timestamp,
            id,
            extended,
            data,
        }) = rx
        {
            #[cfg(feature = "socket")]
            {
                if let Some(socket) = &outsocket {
                    socket.send(&Frame {
                        id,
                        extended,
                        data: data.clone(),
                    });
                }
            }

            last_time_ms = (timestamp - time_start) * 1000.0; // time since start of recording

            match dbc.decode(id, &data, false) {
                Ok(decoded) => {
                    let mut write_col = store.write_columns();
                    for signal in decoded.iter() {
                        let index = store
                            .signalmap
                            .read()
                            .get(signal.name)
                            .expect("Schema, map, or column vector poisoned")
                            .clone();
                        if is_filled.len() <= index {
                            println!("Signal out of range: {} at index: {}", signal.name, index);
                        }
                        if !is_filled[index] {
                            match &mut write_col[index] {
                                GenericColumn::Bool(c) => c.push(Some(signal.value.is_nearly(1.0))),
                                GenericColumn::I8(c) => c.push(Some(signal.value as i8)),
                                GenericColumn::I32(c) => c.push(Some(signal.value as i32)),
                                GenericColumn::I64(c) => c.push(Some(signal.value as i64)),
                                //                            GenericColumn::F16(c) => c.push(Some(f16::from(signal.value))),
                                GenericColumn::F32(c) => c.push(Some(signal.value as f32)),
                                GenericColumn::F64(c) => c.push(Some(signal.value)),
                                _ => {}
                            }
                            is_filled[index] = true;
                        }
                    }
                }
                Err(e) => println!("Signal: {} Data: {:02x?}  Error: {}", id, &data, e),
                //Err(e) => _ = e,
            }
        }
        let relative_time_rcv = last_time_ms;

        if relative_time_rcv > (&args.cache_ms * f64::from(num_chunks) + offset)
            || exit.load(Ordering::SeqCst)
        {
            if ((&args.cache_ms * f64::from(num_chunks) + offset) - relative_time_rcv)
                > 2.0 * &args.cache_ms
            {
                offset += (&args.cache_ms * f64::from(num_chunks) + offset) - relative_time_rcv;
            }

            num_chunks += 1;

            let mut write_cols = store.write_columns();
            is_filled[0] = true;
            match &mut write_cols[0] {
                GenericColumn::F64(c) => c.push(Some(relative_time_rcv)),
                _ => {}
            }

            for (index, value) in is_filled.iter().enumerate() {
                if !value {
                    write_cols[index].push_null();
                }
            }

            store.set_ready(); // Don't let plot start until there's a row.

            is_filled.fill(false);

            if num_chunks % 250 == 0 {
                print!("\rRow #{}", num_chunks);
                io::stdout().flush().unwrap();
            }
        }
        {
            let mut sends = store.sends.write();
            sends.retain_mut(|send_msg| {
                let values: Vec<(&str, f64)> = send_msg
                    .content
                    .iter()
                    .map(|(name, value)| (&**name, *value))
                    .collect();
                let id = dbc.messages().find(&send_msg.name).unwrap().id();
                let extended = dbc.messages().find(&send_msg.name).unwrap().is_extended();
                if let Ok(payload) = dbc.encode(id, &values, extended) {
                    input.send(&Frame {
                        id,
                        extended,
                        data: payload.to_vec(),
                    });
                } else {
                    println!("some kinda encoding error");
                }

                if send_msg.repeat <= 1 {
                    false // last repeat done: drop it from the vec
                } else {
                    send_msg.repeat -= 1;
                    true // keep it for the next round
                }
            });
        }
    }
    println!("");

    if args.en_ipm {
        print!("Writing Out Stored Data");
        let props = WriterProperties::builder()
            .set_compression(parquet::basic::Compression::SNAPPY)
            .build();
        let mut writer = ArrowWriter::try_new(
            File::create(&args.output).unwrap(),
            schema.clone(),
            Some(props),
        )
        .unwrap();

        while let Some(batch) = store.next_record_batch(1000, schema.clone()) {
            writer.write(&batch).unwrap();
            //store.purge_converted(); // only after the writes succeeded
            print!(".");
        }
        println!("");
        println!("Done!");
        writer.close().unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_nodes_from_fs_dbc() {
        let dbc = Dbc::parse(&std::fs::read_to_string("fs.dbc").unwrap()).unwrap();
        let names: Vec<String> = dbc
            .messages()
            .iter()
            .flat_map(|m| {
                m.signals()
                    .iter()
                    .map(|s| s.name().to_string())
                    .collect::<Vec<_>>()
            })
            .collect();
        let nodes = placeholder_nodes(&names);
        let msgs = nodes.get(PARQUET_NODE).expect("placeholder node");
        println!("signals {} messages {}", names.len(), msgs.len());
        assert_eq!(nodes.len(), 1);
        assert_eq!(msgs.len(), names.len());
    }
}
