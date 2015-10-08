#[macro_use]
extern crate hyper;
extern crate rustc_serialize;
extern crate mime;

#[macro_use]
mod common;
mod config;
mod json;
mod transmissionrpc;

use std::process;
use std::io::Write;

use common::GenericResult;
use config::ConfigReadingError;

fn daemon() -> GenericResult<i32> {
    let path = "settings.json";

    let config = try!(config::read_config(path).map_err(
        |e| match e {
            ConfigReadingError::ValidationError(_) => {
                format!("Validation of '{}' configuration file failed: {}.", path, e)
            },
            _ => format!("Error while reading '{}' configuration file: {}.", path, e),
        }));

    let mut rpc_url: String;
    rpc_url = format!("http://{host}:{port}{path}",
        host=config.rpc_bind_address, port=config.rpc_port, path=config.rpc_url);

    if !rpc_url.ends_with("/") {
        rpc_url.push_str("/");
    }

    rpc_url.push_str("rpc");

    let mut client = transmissionrpc::TransmissionClient::new(&rpc_url);
    if config.rpc_authentication_required {
        client.set_authentication(&config.rpc_username, &config.rpc_plain_password.as_ref().unwrap());
    }

    println!("{} {:?}", rpc_url, config);
    client.get_torrents();

    Ok(0)
}

fn main() {
    let exit_code = match daemon() {
        Ok(code) => code,
        Err(err) => {
            let _ = writeln!(&mut std::io::stderr(), "Error: {}", err);
            1
        }
    };

    process::exit(exit_code);
}
