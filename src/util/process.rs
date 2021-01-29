use std::process::Command;

use common::GenericResult;

pub trait RunCommandProvider {
    fn run_command(&self, command: &str, args: &[String]) -> GenericResult<String> {
        run_command(command, args)
    }
}

pub struct RunCommand;
impl RunCommandProvider for RunCommand {}

pub fn run_command(command: &str, args: &[String]) -> GenericResult<String> {
    let mut command_string = s!(command);
    for arg in args {
        command_string.push(' ');
        command_string.push_str(&arg);
    }

    let output = Command::new(command).args(args).output()
        .map_err(|e| format!("Failed to execute `{}`: {}", command_string, e))?;

    if !output.status.success() {
        let stderr = String::from_utf8(output.stderr).map_err(|e| format!(
            "Error during reading `{}` output: {}", command_string, e))?;

        let error = stderr.trim().split('\n').next().unwrap();
        return Err!("`{}` failed with error: {}", command_string, error);
    }

    Ok(String::from_utf8(output.stdout).map_err(|e| format!(
        "Error during reading `{}` output: {}", command_string, e))?)
}

#[cfg(test)]
pub mod tests {
    use common::GenericResult;
    use super::*;

    pub struct RunCommandMock {
        output: String,
    }

    impl RunCommandMock {
        pub fn new(output: &str) -> RunCommandMock {
            RunCommandMock { output: s!(output) }
        }
    }

    impl RunCommandProvider for RunCommandMock {
        fn run_command(&self, _command: &str, _args: &[String]) -> GenericResult<String> {
            Ok(self.output.clone())
        }
    }

    #[test]
    fn test_run_command() {
        assert_eq!(run_command("echo", &[s!("aaa"), s!("bbb\nccc")]).unwrap(), "aaa bbb\nccc\n");
    }

    #[test]
    fn test_run_command_failed() {
        assert_eq!(
            run_command("sh", &[s!("-c"), s!("echo stdout-message && echo stderr-message >&2 && false")]).unwrap_err().to_string(),
            "`sh -c echo stdout-message && echo stderr-message >&2 && false` failed with error: stderr-message"
        );
    }

    #[test]
    fn test_run_command_invalid() {
        assert_eq!(
            run_command("some-invalid-command", &[]).unwrap_err().to_string(),
            "Failed to execute `some-invalid-command`: No such file or directory (os error 2)"
        );
    }
}
