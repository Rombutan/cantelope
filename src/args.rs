use std::env;

#[derive(Default)]
pub enum CanDataInput {
    #[default]
    File,
    Socket,
    Stdin,
    Remote,
}

#[derive(Default)]
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
    let mut argsi = env::args().skip(1); // skip program name
    let mut args = Args::default();
    args.en_ipm = false;
    args.en_aux = false;
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
                // Make both args.aux_outputs which contains unstructured outputs and args.plots which is structured by plot
                // It's fine if things in args.aux_outputs are duplicated, all it will do is waste a few bytes of memory :()
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

            _ => {
                eprintln!("Unknown argument: {}", arg);
            }
        }
    }

    args.setup_aux_outputs();

    return args;
}
