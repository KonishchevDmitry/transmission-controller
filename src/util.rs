use std::process::Command;

use common::GenericResult;

pub fn run_command(command: &str, args: &[String]) -> GenericResult<String> {
    let mut command_string = s!(command);
    for arg in args {
        command_string.push_str(" ");
        command_string.push_str(&arg);
    }

    let output = try!(Command::new(command).args(args).output()
        .map_err(|e| format!("Failed to execute `{}`: {}", command_string, e)));

    if !output.status.success() {
        let stderr = try!(String::from_utf8(output.stderr).map_err(|e| format!(
            "Error during reading `{}` output: {}", command_string, e)));

        let error = stderr.trim().split('\n').next().unwrap();
        return Err!("`{}` failed with error: {}", command_string, error);
    }

    Ok(try!(String::from_utf8(output.stdout).map_err(|e| format!(
        "Error during reading `{}` output: {}", command_string, e))))
}
