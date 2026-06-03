// DBC Parsing
use dbc_rs::Dbc;
use std::fs;

// Arrow IP elements
use arrow::datatypes::{DataType, Field, Schema};

// Literally only for cleaner print
use std::io::{self, Write};

// Custom data storage helpers
pub mod store;
use store::{Column, GenericColumn};

// Custom argument parsing
pub mod args;
use std::env;
use std::sync::{Arc, RwLock};

// Custom Candump parsing
use candump::CanDumpParser;

// Custom TCP Can interface
pub mod tcpwrapper;
pub mod udpwrapper;

use crate::args::Args;
use crate::args::CanDataInput;

// Pyo3
use pyo3::{
    prelude::*,
    types::{PyAnyMethods, PyModule, PyTuple},
};
use std::ffi::CString;

// SocketCAN
#[cfg(feature = "socket")]
pub mod socketwrap;

// Use ctrl+c as exit signal in stdin and socket mode
use std::sync::atomic::{AtomicBool, Ordering};
use std::{process, thread, time};

// Allows data loop to send decoded values back to main thread
use std::sync::mpsc;
use std::sync::mpsc::SyncSender;

#[cfg(feature = "plot")]
pub mod plot;

#[cfg(feature = "plot")]
use plot::DataPoint;

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

enum InputSource {
    File(CanDumpParser),
    #[cfg(feature = "socket")]
    Can(socketwrap::CanWrapper),
    Tcp(tcpwrapper::TcpWrapper),
    Udp(udpwrapper::UdpWrapper),
    Stdin((CanDumpParser, io::Stdin)),
}

fn main() {
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

    let args = Arc::new(RwLock::new(start_args)); // Load arguments into a struct

    if args.read().unwrap().dbcfile == "".to_string() {
        let files = FileDialog::new()
            .add_filter("dbc", &["dbc", "DBC"])
            .set_title("Choose DBC File")
            .pick_file();
        args.write().unwrap().dbcfile = files
            .expect("DBF File picker failed")
            .as_path()
            .to_str()
            .expect("DBC path UTF-8 check failed?")
            .to_string();
    }

    let dbc_content =
        fs::read_to_string(&args.read().unwrap().dbcfile).expect("Could not read DBC file"); // Load DBC file contents into string

    let (tx, rx) = mpsc::sync_channel::<DataPoint>(100); // For transfers from the data loop thread to main

    let args_en_aux = args.read().unwrap().en_aux;

    let args_data_thread = Arc::clone(&args);

    let handle = std::thread::spawn(move || {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async move {
                data_loop(args_data_thread, &dbc_content, tx).await;
            });
    });

    #[cfg(feature = "plot")]
    if args_en_aux {
        println!("Starting plotting window!");
        _ = PlotWindow::run(rx, args);
    }

    #[cfg(not(feature = "end_data_on_close"))]
    let _ = handle.join();
}

async fn data_loop(args: Arc<RwLock<args::Args>>, dbc_content: &String, tx: SyncSender<DataPoint>) {
    let dbc;
    match Dbc::parse(&dbc_content) {
        Ok(v) => {
            dbc = v;
            println!("DBC Succesfully parsed!");
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
    } // Parse DBC

    // ------- CREATE SCHEMA
    let mut base_row_size = 0; // Just to generate a cool "uncompressed data rate" number
    let mut fields: Vec<Field> = Vec::new(); // vec of column descriptions, later dumped into Arc<Schema>
    let mut columns: Vec<GenericColumn> = Vec::new(); // This vec actually stores the values
    let mut is_filled: Vec<bool> = Vec::new(); // This will keep track of which values have been filled so ones which haven't can be null balanced

    let mut column_names = vec!["Time_ms"];

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
            column_names.push(signal.name());
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

    Python::initialize();
    let mut python_string: Vec<String> = vec![];
    let mut python_object: Option<Py<PyAny>> = None;
    let python = &args.read().unwrap().python.clone();
    if python.contains(".py") {
        let python_code = fs::read_to_string(python).expect("Failed to read Python file from disk");

        // Convert the dynamic code string into a C-compatible string slice
        let py_code_c = CString::new(python_code).unwrap();
        let pyfile = CString::new(python.as_str()).unwrap();
        let pymod = CString::new(python.replace(".py", "")).unwrap();

        Python::attach(|py| {
            // Compile the dynamic string into an in-memory module
            let module = PyModule::from_code(py, &py_code_c, &pyfile, &pymod)?;

            // Find the class inside the module
            match module.getattr("CantelopeExtension") {
                Ok(class_obj) => {
                    let factory_result =
                        class_obj.call_method1("create_and_initialize", (column_names,))?;
                    let tuple_result = factory_result.downcast::<PyTuple>()?;

                    let instance = tuple_result.get_item(0)?;
                    python_object = Some(instance.unbind());

                    python_string = tuple_result.get_item(1).unwrap().extract()?;
                }
                Err(e) => {
                    // This will print the exact Python stack trace/error type (e.g., AttributeError)
                    eprintln!("Failed to get CantelopeExtension: {:?}", e);

                    // Let's also print everything that IS available in the module to debug
                    if let Ok(dir) = module.dir() {
                        println!("Available attributes in module: {:?}", dir);
                    }
                }
            }
            Ok::<(), PyErr>(())
        })
        .unwrap();
    }

    let schema = Arc::new(Schema::new(fields));

    let mut input: InputSource;
    let inputname = &args.read().unwrap().candatainput.clone();
    match inputname {
        CanDataInput::File => {
            input = InputSource::File(CanDumpParser::new(&args.read().unwrap().input).unwrap());
        }
        CanDataInput::Stdin => {
            input =
                InputSource::Stdin((CanDumpParser::new(&String::default()).unwrap(), io::stdin()));
        }
        #[cfg(feature = "socket")]
        CanDataInput::Socket => {
            input = InputSource::Can(
                socketwrap::CanWrapper::new(&args.read().unwrap().input)
                    .expect("Failed to open CAN Socket?\n"),
            );
        }
        #[cfg(not(feature = "socket"))]
        CanDataInput::Socket => {
            panic!("Socketcan not enabled in this build")
        }
        CanDataInput::TcpRemote => {
            input = InputSource::Tcp(tcpwrapper::TcpWrapper::new(&args.read().unwrap().input));
        }
        CanDataInput::UdpRemote => {
            let addr = args.read().unwrap().input.clone();
            input = InputSource::Udp(udpwrapper::UdpWrapper::new(&addr).await);
        }
    }

    let time_start;
    match input {
        InputSource::File(ref mut filestream) => {
            filestream.parse();
            time_start = filestream.get_timestamp();
        }
        InputSource::Stdin((ref mut parser, ref mut inputstream)) => {
            let mut nextline = String::new();
            match inputstream.read_line(&mut nextline) {
                Ok(_n) => {}
                Err(msg) => {
                    println!("While trying to read from stdin: {}", msg);
                    std::process::exit(74);
                }
            }
            _ = parser.parse_string(nextline);
            time_start = parser.get_timestamp();
        }
        #[cfg(feature = "socket")]
        InputSource::Can(ref mut socketwrapper) => {
            socketwrapper.parse().unwrap();
            time_start = socketwrapper.get_timestamp();
        }
        InputSource::Tcp(ref mut networkwrapper) => {
            networkwrapper.parse().unwrap();
            time_start = networkwrapper.get_timestamp();
        }
        InputSource::Udp(ref mut networkwrapper) => {
            networkwrapper.parse().await.unwrap();
            time_start = networkwrapper.get_timestamp();
        }
    }

    let exit = Arc::new(AtomicBool::new(false));
    let ex = exit.clone();

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
    while !exit.load(Ordering::SeqCst) {
        let mut python_inputs: Vec<Option<f64>> = vec![None; python_string.len()];

        // Message recieve loop
        let timestamp;
        let id;
        let data;

        match input {
            InputSource::File(ref mut filestream) => {
                filestream.parse();
                timestamp = filestream.get_timestamp();
                id = filestream.get_id();
                data = filestream.get_data();
            }
            InputSource::Stdin((ref mut parser, ref mut inputstream)) => {
                let mut nextline = String::new();
                match inputstream.read_line(&mut nextline) {
                    Ok(_n) => {}
                    Err(msg) => {
                        println!("While trying to read from stdin: {}", msg);
                        std::process::exit(74);
                    }
                }
                exit.store(parser.parse_string(nextline), Ordering::SeqCst);
                timestamp = parser.get_timestamp();
                id = parser.get_id();
                data = parser.get_data();
            }
            #[cfg(feature = "socket")]
            InputSource::Can(ref mut socketwrapper) => {
                socketwrapper.parse().unwrap();
                timestamp = socketwrapper.get_timestamp();
                id = socketwrapper.get_id();
                data = socketwrapper.get_data();
            }
            InputSource::Tcp(ref mut networkwrapper) => {
                networkwrapper.parse().unwrap();
                timestamp = networkwrapper.get_timestamp();
                id = networkwrapper.get_id();
                data = networkwrapper.get_data();
            }
            InputSource::Udp(ref mut networkwrapper) => {
                networkwrapper.parse().await.unwrap();
                timestamp = networkwrapper.get_timestamp();
                id = networkwrapper.get_id();
                data = networkwrapper.get_data();
            }
        }

        let relative_time_rcv = (timestamp - time_start) * 1000.0; // time since start of recording

        match dbc.decode(id, &data, false) {
            Ok(decoded) => {
                for signal in decoded.iter() {
                    let col = &mut columns[schema.index_of(signal.name).unwrap()];
                    if !is_filled[schema.index_of(signal.name).unwrap()] {
                        // Only save the first value from each chunk (as opposed to prev version saving last)
                        if args.read().unwrap().en_ipm {
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

                        if python_string.iter().any(|s| s == &signal.name) {
                            python_inputs[(python_string
                                .iter()
                                .position(|x| x == &signal.name.to_string()))
                            .unwrap()] = Some(signal.value);
                        }

                        if args
                            .read()
                            .unwrap()
                            .aux_outputs
                            .iter()
                            .any(|s| s == &signal.name)
                        {
                            let _ = tx.try_send((
                                signal.name.to_string(),
                                relative_time_rcv,
                                signal.value,
                            ));
                        }
                    }
                }
            }
            Err(e) => println!("Signal: {} Data: {:02x?}  Error: {}", id, &data, e),
            //Err(e) => _ = e,
        }

        if relative_time_rcv > (&args.read().unwrap().cache_ms * f64::from(num_chunks))
            || exit.load(Ordering::SeqCst)
        {
            if let Some(ref py_obj) = python_object {
                let mut python_outputs: Vec<(String, f64)> = vec![];
                Python::attach(|py| {
                    // Re-bind the object to the current GIL context
                    let bound_obj = py_obj.bind(py);

                    // Execute the method on the instance
                    let py_result = bound_obj.call_method1("process_numbers", (python_inputs,))?;

                    // Extract the result back into native Rust types
                    python_outputs = py_result.extract()?;

                    Ok::<(), PyErr>(())
                })
                .unwrap();

                for value in python_outputs {
                    if args
                        .read()
                        .unwrap()
                        .aux_outputs
                        .iter()
                        .any(|s| s == &value.0)
                    {
                        let _ = tx.try_send((value.0.to_string(), relative_time_rcv, value.1));
                    }
                }
            }

            num_chunks += 1;

            if args.read().unwrap().en_ipm {
                let col = &mut columns[schema.index_of("Time_ms").unwrap()];
                is_filled[schema.index_of("Time_ms").unwrap()] = true;
                match col {
                    GenericColumn::F64(c) => c.push(Some(relative_time_rcv)),
                    _ => {}
                }

                for (index, value) in is_filled.iter().enumerate() {
                    if !value {
                        columns[index].push_null();
                    }
                }

                is_filled.fill(false);
            }
            if num_chunks % 250 == 0 {
                print!("\rRow #{}", num_chunks);
                io::stdout().flush().unwrap();
            }
        }
    }
    println!("");
    if args.read().unwrap().en_ipm {
        let batch = store::finish_record_batch(columns, schema);
        store::write_record_batch_to_parquet(&batch, &args.read().unwrap().output).unwrap();
        println!("Finished writting out!");
    }
}
