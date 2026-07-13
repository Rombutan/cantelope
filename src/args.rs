use serde::{Deserialize, Serialize};
use std::{env, fs};

use rfd::FileDialog;

#[derive(Default, Serialize, Deserialize, Clone)]
pub enum CanDataInput {
    #[default]
    File,
    Socket,
    Stdin,
    TcpRemote,
    UdpRemote,
}

#[derive(Default, Serialize, Deserialize, Clone)]
pub struct Args {
    pub dbcfile: String,
    pub input: String,
    pub output: String,
    pub candatainput: CanDataInput,
    pub cache_ms: f64,
    pub aux_outputs: Vec<String>,
    pub plots: Vec<Vec<String>>,
    pub regrens: Vec<(String, bool, f64)>,
    pub regrens_raw: String,
    pub en_ipm: bool,
    pub en_aux: bool,
    pub python: String,
    pub socketout: String,
}

impl Args {
    pub fn setup_aux_outputs(&mut self) {
        for plot in &self.plots {
            for output in plot {
                if !self.aux_outputs.contains(&output) {
                    self.aux_outputs.push(output.clone());
                    self.en_aux = true;
                }
            }
        }

        if !self.regrens_raw.is_empty() {
            self.regrens =
                parse_regrens(self.regrens_raw.clone()).expect("Could not parse regrens_raw");
        }

        for regren in &self.regrens {
            if !self.aux_outputs.contains(&regren.0) {
                self.aux_outputs.push(regren.0.clone());
                self.en_aux = true;
            }
        }
    }
}

pub fn parse_regrens(raw_val: String) -> Result<Vec<(String, bool, f64)>, String> {
    let mut regrens: Vec<(String, bool, f64)> = [].to_vec();
    for item in raw_val.split(',') {
        // Find the operator and its type
        let (op_idx, is_greater) = if let Some(idx) = item.find(']') {
            (idx, true) // 1 for >
        } else if let Some(idx) = item.find('[') {
            (idx, false) // 0 for <
        } else {
            return Err("Incorrect Format".to_owned());
        };

        // Split based on the found index
        let name_str = &item[..op_idx];
        let threshold_str = &item[op_idx + 1..];

        let name = name_str.to_string();
        let threshold: f64 = threshold_str
            .parse()
            .map_err(|e: std::num::ParseFloatError| e.to_string())?;

        // Tuple is (String, bool, f64)
        // is_greater (bool) will be true (1) for > and false (0) for <
        regrens.push((name.clone(), is_greater, threshold));
    }

    Ok(regrens)
}

const HELP_MSG: &str = "cantelope [options]
Options:                        If no options, use file picker for config.
    --dbc | d file.dbc          Specifies dbc file for decoding.
                                If unprovided, use gui file picker.
  Input Options:                If none given, use candump and file picker.
    [--candump | -f file.log]   Specifies candump mode and file to use.
                                Expects format:
                                `(time in seconds) interface 0xid#0xdata`
                                which is the default of `candump -L`.
    [--stdin | -t]              Sets STDIN mode with candump format
    [--tcp ip:port]             Sets TCP client mode and server address.
    [--socket | -s can0]        Uses a socketcan interface for input,
                                only available if compiled with option.
  Python:
    [--python file.py]          Will instantiate and execute a class from
                                that file on each row, and make that output
                                available to plotting subsystem.
  Output Options:
    [--output | -o f.parquet]   Enables output to provided parquet file.
    [--cache_ms | -c n]         Sets minimum time between rows. n is integer.
    [--socketout socket]        Enabled immediate output of all recieved frames
                                on provided socketcan interface. No windows.

  Plotting / GUI Options:
    [--plot.. | -p.. SIG1,SIG2] Enables plotting and adds a plot to the window
                                You can add as many plots as you can fit and
                                as many signals to a plot as you want.
    [--regrens | -rg SIG]13.5,..]
                                Enables regren row. You can add as many signals
                                to regren as you like. The inequality uses `[`
                                and `]` in place of `<` and `>` to avoid shell
                                issues. Comma seperate eahc inequality.
  Config Options:
    [--emit-config [file.toml]] Will create a `.toml` file with the full
                                configuration provided. If you don't provide
                                a file, the gui file picker will be called.
    [--config [file.toml]]      Will set all settings to the provided (or chose)
                                config file, but continue parsing arguments,
                                so you can ovveride any values in the config
                                by adding arguments after this. This also sets
                                the config file path, so --emit-config after this
                                with no argument WILL OVERWRITE the config file
                                you passed here.
    ";

pub fn process_args() -> Args {
    let mut argsi = env::args().skip(1).peekable(); // skip program name
    let mut args = Args::default();
    args.en_ipm = false;
    args.en_aux = false;

    let mut emit_config = false;
    let mut emit_config_path: String = "".to_string();

    while let Some(arg) = argsi.next() {
        match arg.as_str() {
            "--dbc" | "-d" => {
                let value = argsi.next().expect("--dbc requires a value");
                args.dbcfile = value;
            }

            "--candump" | "-f" => {
                args.candatainput = CanDataInput::File;
                args.input = argsi
                    .next()
                    .expect("--tcp requires a value")
                    .parse()
                    .unwrap()
            }

            #[cfg(feature = "socket")]
            "--socket" | "-s" => {
                args.candatainput = CanDataInput::Socket;
                args.input = argsi
                    .next()
                    .expect("--tcp requires a value")
                    .parse()
                    .unwrap()
            }

            #[cfg(not(feature = "socket"))]
            "--socket" => {
                panic!("Socketcan feature disabled!")
            }

            #[cfg(feature = "socket")]
            "--socketout" => {
                args.socketout = argsi
                    .next()
                    .expect("--tcp requires a value")
                    .parse()
                    .unwrap()
            }

            #[cfg(not(feature = "socket"))]
            "--socketout" | "-s" => {
                panic!("Socketcan feature disabled!")
            }

            "--stdin" | "-t" => {
                args.candatainput = CanDataInput::Stdin;
            }

            "--tcp" => {
                args.candatainput = CanDataInput::TcpRemote;
                args.input = argsi
                    .next()
                    .expect("--tcp requires a value")
                    .parse()
                    .unwrap()
            }

            "--udp" => {
                args.candatainput = CanDataInput::UdpRemote;
                args.input = argsi
                    .next()
                    .expect("--tcp requires a value")
                    .parse()
                    .unwrap()
            }

            "--cache_ms" | "-c" => {
                args.cache_ms = argsi
                    .next()
                    .expect("--cache-ms requires a value")
                    .parse()
                    .unwrap();
            }

            "--output" | "-o" => {
                args.output = argsi
                    .next()
                    .expect("--output requires a value")
                    .parse()
                    .unwrap();
                args.en_ipm = true;
            }

            "--python" => {
                args.python = argsi
                    .next()
                    .expect("--output requires a value")
                    .parse()
                    .unwrap();
            }

            "--plot" | "-p" => {
                let raw_val = argsi.next().expect("--plot requires a value");
                let list: Vec<String> = raw_val.split(',').map(|s| s.to_string()).collect();
                args.plots.push(list);
            }

            "--regrens" | "-rg" => {
                let raw_val = argsi
                    .next()
                    .expect("--regre requires at least one field and threshold");

                args.regrens_raw = raw_val.clone();
                args.regrens = parse_regrens(raw_val).unwrap();
            }
            "--config" => {
                let mut need_dialogue = true;

                if let Some(next) = argsi.peek() {
                    if !next.starts_with('-') {
                        let value = argsi.next().unwrap();
                        emit_config_path = value;
                        need_dialogue = false;
                    }
                }

                if need_dialogue {
                    let files = FileDialog::new()
                        .add_filter("toml", &["toml"])
                        .set_title("Choose Config File")
                        .set_file_name("Cantelope_Config.toml")
                        .pick_file();
                    emit_config_path = files
                        .as_ref()
                        .unwrap()
                        .as_path()
                        .to_str()
                        .unwrap()
                        .to_string();

                    let mut start_args: Args = toml::from_str(
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
                    args = start_args;
                } else {
                    let mut start_args: Args = toml::from_str(
                        &fs::read_to_string(&emit_config_path).expect("Could not read file."),
                    )
                    .expect("Could not parse toml config file. Check formatting.");
                    start_args.setup_aux_outputs();
                    args = start_args;
                }
            }
            "--emit-config" => {
                emit_config = true;

                let mut need_dialogue = true;

                if let Some(next) = argsi.peek() {
                    if !next.starts_with('-') {
                        let value = argsi.next().unwrap();
                        emit_config_path = value;
                        need_dialogue = false;
                    }
                } else if emit_config_path.len() > 0 {
                    need_dialogue = false;
                }

                if need_dialogue {
                    let files = FileDialog::new()
                        .add_filter("toml", &["toml"])
                        .set_title("Choose Config File")
                        .save_file();
                    emit_config_path = files.unwrap().as_path().to_str().unwrap().to_string();
                }
            }

            "--help" | "-h" => {
                println!("{}", HELP_MSG);
                std::process::exit(0);
            }

            _ => {
                eprintln!("Unknown argument: {} \n{}", arg, HELP_MSG);
            }
        }
    }

    args.setup_aux_outputs();

    if emit_config {
        fs::write(emit_config_path, toml::to_string(&args).unwrap()).unwrap();
    }

    return args;
}
