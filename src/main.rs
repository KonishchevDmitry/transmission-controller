extern crate rustc_serialize;

#[macro_use]
mod common;
mod config;

use std::process;
use std::io::Write;

use common::GenericResult;

fn daemon() -> GenericResult<i32> {
    let path = "settings.json";

    let config = try!(config::read_config(path).map_err(
        |e| format!("Error while reading '{}' configuration file: {}.", path, e)));

    println!("{:?}", config);

    Ok(0)
}

fn main() {
    let exit_code = match daemon() {
        Ok(code) => code,
        Err(err) => {
            let _ = writeln!(&mut std::io::stderr(), "{}", err);
            1
        }
    };

    process::exit(exit_code);
}
