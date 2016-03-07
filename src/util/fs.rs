use std::fs;
use std::io;
use std::path::Path;

use libc;
use regex::Regex;

use common::{EmptyResult, GenericResult};
use util::process::{RunCommandProvider, RunCommand};

pub fn copy_file<S: AsRef<Path>, D: AsRef<Path>>(src: S, dst: D) -> EmptyResult {
    let dst = dst.as_ref();
    let mut src_file = try!(fs::File::open(&src).map_err(|e| format!(
        "Failed to open '{}': {}", src.as_ref().display(), e)));

    // TODO: use O_CREAT & O_EXCL
    try!(match fs::metadata(&dst) {
        Ok(_) => Err(format!("'{}' already exists", dst.display())),
        Err(err) => match err.kind() {
            io::ErrorKind::NotFound => Ok(()),
            _ => Err(format!("Failed to create '{}': {}", dst.display(), err))
        }
    });

    let mut dst_file = try!(fs::File::create(&dst).map_err(|e| format!(
        "Failed to create '{}': {}", dst.display(), e)));

    try!(io::copy(&mut src_file, &mut dst_file));

    Ok(())
}

pub fn check_directory<P: AsRef<Path>>(path: P) -> EmptyResult {
    let path = path.as_ref();

    let metadata = try!(match fs::metadata(&path) {
        Ok(metadata) => Ok(metadata),
        Err(err) => Err(
            if is_no_such_file_error(&err) {
                format!("'{}' doesn't exist", path.display())
            } else {
                format!("'{}': {}", path.display(), err)
            }
        )
    });

    if !metadata.is_dir() {
        return Err!("'{}' is not a directory", path.display());
    }

    Ok(())
}

pub fn check_existing_directory<P: AsRef<Path>>(path: P) -> GenericResult<bool> {
    let path = path.as_ref();

    let exists = match fs::metadata(&path) {
        Ok(metadata) => {
            if metadata.is_dir() {
                true
            } else {
                return Err!("It already exists and is not a directory");
            }
        },
        Err(err) => {
            if err.kind() == io::ErrorKind::NotFound {
                false
            } else {
                return Err(From::from(err));
            }
        }
    };

    Ok(exists)
}

/// Creates all directories represented by `path` in `base` directory.
///
/// Uses optimistic scenario optimized for the case when the directories already exist. If `path`
/// is empty, only checks that `base` directory exists.
pub fn create_all_dirs_from_base<B: AsRef<Path>, P: AsRef<Path>>(base: B, path: P) -> EmptyResult {
    let (base, mut path) = (base.as_ref(), path.as_ref());

    assert!(path.is_relative());

    let mut checked = false;
    let mut deferred_paths = Vec::new();

    while path.components().next().is_some() {
        let full_path = base.join(&path);

        if try!(check_existing_directory(&full_path).map_err(|e| format!(
            "Failed to create '{}' directory: {}", full_path.display(), e))
        ) {
            checked = true;
            break;
        }

        if let Err(err) = fs::create_dir(&full_path) {
            match err.kind() {
                // The parent directory doesn't exist. Create it first.
                io::ErrorKind::NotFound => {
                    deferred_paths.push(path.clone());

                    if let Some(parent_path) = path.parent() {
                        path = parent_path;
                    } else {
                        break;
                    }
                },

                // We've got a race. Retry the attempt to create the directory.
                io::ErrorKind::AlreadyExists => continue,

                _ => return Err!("Failed to create '{}' directory: {}", full_path.display(), err),
            }
        } else {
            checked = true;
            break;
        }
    }

    if !checked {
        try!(check_directory(&base));
    }

    for path in deferred_paths.iter().rev() {
        let full_path = base.join(&path);
        try!(fs::create_dir(&full_path).map_err(|e| format!(
            "Failed to create '{}' directory: {}", full_path.display(), e)));
    }

    Ok(())
}

pub fn get_device_usage<P: AsRef<Path>>(path: P) -> GenericResult<(String, u8)> {
    _get_device_usage(path, &RunCommand)
}

fn _get_device_usage<P: AsRef<Path>>(path: P, provider: &RunCommandProvider) -> GenericResult<(String, u8)> {
    let mut path = s!(path.as_ref().to_str().unwrap());

    // df gives a different output for "dir" and "dir/"
    if !path.ends_with('/') {
        path.push('/');
    }

    let output = try!(provider.run_command("df", &[path]));

    let get_parse_error = || {
        let error = "Got an unexpected output from `df`";
        debug!("{}:\n{}", error, output);
        return Err(From::from(error))
    };

    let lines: Vec<&str> = output.trim().split('\n').collect();
    if lines.len() != 2 {
        return get_parse_error()
    }

    let output_re = Regex::new(r"(?x)^
        \s*(?P<device>.*?)    # Device
        (?:\s+\d+){3}         # Blocks, Used, Available
        \s+(?P<use>\d{1,2})%  # Use%
    ").unwrap();

    let captures = match output_re.captures(lines[1]) {
        Some(captures) => captures,
        None => return get_parse_error(),
    };

    Ok((
        s!(captures.name("device").unwrap()),
        captures.name("use").unwrap().parse().unwrap(),
    ))
}

fn is_no_such_file_error(error: &io::Error) -> bool {
    if let Some(errno) = error.raw_os_error() {
        if errno == libc::ENOTDIR || errno == libc::ENOENT {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use util::process::tests::RunCommandMock;

    #[test]
    fn test_get_device_usage() {
        assert_eq!(
            super::_get_device_usage("/some/path", &RunCommandMock::new("\
                Filesystem     1K-blocks     Used Available Use% Mounted on\n\
                /dev/sdb1      153836548 55183692  98636472  36% /mnt/var_data\n\
            ")).unwrap(),
            (s!("/dev/sdb1"), 36)
        );
    }

    #[test]
    fn test_get_device_usage_no_data() {
        assert_eq!(
            super::_get_device_usage("/some/path", &RunCommandMock::new("\
                Filesystem     1K-blocks     Used Available Use% Mounted on\n\
            ")).unwrap_err().to_string(),
            "Got an unexpected output from `df`"
        );
    }

    #[test]
    fn test_get_device_usage_few_devices() {
        assert_eq!(
            super::_get_device_usage("/some/path", &RunCommandMock::new("\
                Filesystem     1K-blocks      Used Available Use% Mounted on\n\
                /dev/sda1       30830592  16071884  13169564  55% /\n\
                /dev/sdb1      153836548  48887416 104932748  32% /mnt/var_data\n\
            ")).unwrap_err().to_string(),
            "Got an unexpected output from `df`"
        );
    }
}
