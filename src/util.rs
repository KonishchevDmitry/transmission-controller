use std::process::Command;

use common::GenericResult;

pub fn run_command(command: &str, args: &Vec<String>) -> GenericResult<String> {
    let mut command_string = command.to_owned();
    for arg in args {
        command_string.push_str(" ");
        command_string.push_str(&arg);
    }

    let output = try!(Command::new(command).args(args).output()
        .map_err(|e| format!("Failed to execute `{}`: {}", command_string, e)));

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let error = stderr.trim().split('\n').next().unwrap();
        return Err(From::from(format!("`{}` failed with error: {}", command_string, error)));
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
