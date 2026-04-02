use serde::{Deserialize, Serialize};
use std::{env, fs};

use rfd::FileDialog;

#[derive(Default, Serialize, Deserialize)]
pub enum CanDataInput {
    #[default]
    File,
    Socket,
    Stdin,
    Remote,
}

#[derive(Default, Serialize, Deserialize)]
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

            "--input" | "-i" => {
                let value = argsi.next().expect("--input requires a value");
                args.input = value;
            }

            "--candump" | "-f" => {
                args.candatainput = CanDataInput::File;
            }

            #[cfg(feature = "socket")]
            "--socket" | "-s" => {
                args.candatainput = CanDataInput::Socket;
            }

            #[cfg(not(feature = "socket"))]
            "--socket" | "-s" => {
                panic!("Socketcan feature disabled!")
            }

            "--stdin" | "-t" => {
                args.candatainput = CanDataInput::Stdin;
            }

            "--remote" | "-r" => {
                args.candatainput = CanDataInput::Remote;
            }

            "--cache-ms" | "-c" => {
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
            "--emit-config" => {
                emit_config = true;

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
                        .save_file();
                    emit_config_path = files.unwrap().as_path().to_str().unwrap().to_string();
                }
            }

            _ => {
                eprintln!("Unknown argument: {}", arg);
            }
        }
    }

    args.setup_aux_outputs();

    if emit_config {
        fs::write(emit_config_path, toml::to_string(&args).unwrap()).unwrap();
    }

    return args;
}
