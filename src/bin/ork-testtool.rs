//! A tiny program for the process tests (T07). Built only with `--features testtool`.
//!
//! - `sleep SECONDS`: wait, then exit 0
//! - `print LINES`: print `line 1` .. `line N`
//! - `exit CODE`: exit with CODE
//! - `env NAME`: print the variable's value (nothing when unset)

use std::process::ExitCode;
use std::time::Duration;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [command, value] = args.as_slice() else {
        eprintln!("usage: ork-testtool sleep SECONDS | print LINES | exit CODE | env NAME");
        return ExitCode::from(2);
    };

    match command.as_str() {
        "sleep" => {
            std::thread::sleep(Duration::from_secs_f64(value.parse().unwrap_or(0.0)));
            ExitCode::SUCCESS
        }
        "print" => {
            for i in 1..=value.parse::<u32>().unwrap_or(0) {
                println!("line {i}");
            }
            ExitCode::SUCCESS
        }
        "exit" => ExitCode::from(value.parse::<u8>().unwrap_or(1)),
        "env" => {
            print!("{}", std::env::var(value).unwrap_or_default());
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("unknown command: {command}");
            ExitCode::from(2)
        }
    }
}
