use std::env;

#[derive(Default)]
pub enum CanDataInput {
    #[default]
    File,
    Socket,
    Stdin,
}

#[derive(Default)]
pub struct Args {
    pub dbcfile: String,
    pub input: String,
    pub output: String,
    pub candatainput: CanDataInput,
    pub cache_ms: f64,
    pub aux_outputs: Vec<String>,
    pub aux_ms: f64,
    pub aux_num: u32,
    pub en_ipm: bool,
}

pub fn process_args() -> Args {
    let mut argsi = env::args().skip(1); // skip program name
    let mut args = Args::default();
    args.en_ipm = false;
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
                args.aux_outputs.push(
                    argsi
                        .next()
                        .expect("--aux requires a value")
                        .parse()
                        .unwrap(),
                );
            }

            _ => {
                eprintln!("Unknown argument: {}", arg);
            }
        }
    }

    if (args.aux_ms != f64::default()) ^ (args.aux_outputs.len() > 0) {
        // If Aux timing has been set XOR aux outputs have been added
        eprintln!("Auxiliary outputs and time range MUST be set together");
    }

    if (args.aux_ms * 5.0) < args.cache_ms {
        eprintln!("Dont set auxiliary time range to less than 5 times the cache timing");
    }

    args.aux_num = (args.aux_ms / args.cache_ms) as u32;

    return args;
}
