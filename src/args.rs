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
    pub en_ipm: bool,
    pub en_aux: bool,
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
                args.aux_outputs.extend(list.clone());
                args.plots.push(list);
                args.en_aux = true;
            }

            _ => {
                eprintln!("Unknown argument: {}", arg);
            }
        }
    }

    return args;
}
