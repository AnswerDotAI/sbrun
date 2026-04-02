use std::{env, ffi::OsString, process};

use sbrun::{CliCommand, help_text, parse_cli, run};

fn main() {
    let program = env::args_os()
        .next()
        .unwrap_or_else(|| OsString::from("sbrun"));
    let program = program.to_string_lossy();
    match parse_cli(env::args_os()) {
        Ok(CliCommand::Help) => {
            print!("{}", help_text(&program));
        }
        Ok(CliCommand::Version) => {
            println!("sbrun {}", env!("CARGO_PKG_VERSION"));
        }
        Ok(CliCommand::Run { target, options }) => match run(target, options) {
            Ok(_) => unreachable!(),
            Err(err) => {
                eprintln!("sbrun: {err}");
                process::exit(111);
            }
        },
        Err(err) => {
            eprintln!("sbrun: {err}");
            eprintln!();
            eprint!("{}", help_text(&program));
            process::exit(111);
        }
    }
}
