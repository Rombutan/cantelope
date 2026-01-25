use std::env;

#[derive(Default)]
pub enum CanDataInput{
    #[default]
    File,
    Socket,
    Stdin,
}

#[derive(Default)]
pub struct Args{
    pub dbcfile: String,
    pub input: String,
    pub output: String,
    pub candatainput: CanDataInput,
    pub cache_ms: f64,
}


pub fn process_args() -> Args {
    let mut argsi = env::args().skip(1); // skip program name
    let mut args = Args::default();

    while let Some(arg) = argsi.next() {
        match arg.as_str() {
            "--dbc" | "-d" => {
                let value = argsi
                    .next()
                    .expect("--dbc requires a value");
                args.dbcfile = value;
            }

            "--input" | "-i" => {
                let value = argsi
                    .next()
                    .expect("--input requires a value");
                args.input = value;
            }

            "--candump" | "-f" => {
                args.candatainput = CanDataInput::File;
            }

            
            "--cache-ms" | "-c" => {
                args.cache_ms = argsi.next().expect("--cache-ms requires a value").parse().unwrap();
            }

            "--output" | "-o" => {
                args.output = argsi.next().expect("--output requires a value").parse().unwrap();
            }

            _ => {
                eprintln!("Unknown argument: {}", arg);
            }
        }
    }
    return args;
}

