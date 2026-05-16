use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::time::{Instant, Duration};

use anyhow::{anyhow, Context, Result};
use regex::Regex;

use crate::common::GenericResult;
use crate::util::process::{RunCommandProvider, RunCommand};

pub fn validate_torrent_file_name(torrent_file_name: &str) -> Result<(&Path, PathBuf, &OsStr)> {
    use std::path::Component::*;

    let mut file_root_path = None;
    let mut file_path = PathBuf::new();
    let mut file_name = None;

    for component in Path::new(torrent_file_name).components() {
        match component {
            Normal(component) => {
                if file_root_path.is_none() {
                    file_root_path = Some(Path::new(component));
                }
                file_name = Some(component);
                file_path.push(component);
            },
            Prefix(_) | RootDir | CurDir | ParentDir => {
                return Err!("invalid torrent's file name: {torrent_file_name:?}");
            }
        }
    }

    let (Some(file_root_path), Some(file_name)) = (file_root_path, file_name) else {
        return Err!("invalid torrent's file name: {torrent_file_name:?}");
    };

    Ok((file_root_path, file_path, file_name))
}

pub fn copy_downloaded_file<S: AsRef<Path>, D: AsRef<Path>>(src: S, dst: D) -> Result<()> {
    let mut src_file = open_downloaded_file(src)?;

    let dst = dst.as_ref();
    let mut dst_file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(dst)
        .with_context(|| format!("failed to create {dst:?}"))?;

    io::copy(&mut src_file, &mut dst_file)?;

    Ok(())
}

pub fn check_directory<P: AsRef<Path>>(path: P) -> Result<()> {
    let path = path.as_ref();

    let metadata = match fs::metadata(path) {
        Ok(metadata) => Ok(metadata),
        Err(err) => Err(
            if is_no_such_file_error(&err) {
                anyhow!("{path:?} doesn't exist")
            } else {
                anyhow!("{path:?}: {err}")
            }
        )
    }?;

    if !metadata.is_dir() {
        return Err!("{path:?} is not a directory");
    }

    Ok(())
}

pub fn check_existing_directory<P: AsRef<Path>>(path: P) -> Result<bool> {
    let path = path.as_ref();

    let exists = match fs::metadata(path) {
        Ok(metadata) => {
            if metadata.is_dir() {
                true
            } else {
                return Err!("it already exists and is not a directory");
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
pub fn create_all_dirs_from_base<B: AsRef<Path>, P: AsRef<Path>>(base: B, path: P) -> Result<()> {
    let (base, mut path) = (base.as_ref(), path.as_ref());

    assert!(path.is_relative());

    let mut checked = false;
    let mut deferred_paths = Vec::new();

    while path.components().next().is_some() {
        let full_path = base.join(path);

        if check_existing_directory(&full_path).with_context(|| format!(
            "failed to create {full_path:?} directory"
        ))? {
            checked = true;
            break;
        }

        if let Err(err) = fs::create_dir(&full_path) {
            match err.kind() {
                // The parent directory doesn't exist. Create it first.
                io::ErrorKind::NotFound => {
                    deferred_paths.push(path);

                    if let Some(parent_path) = path.parent() {
                        path = parent_path;
                    } else {
                        break;
                    }
                },

                // We've got a race. Retry the attempt to create the directory.
                io::ErrorKind::AlreadyExists => continue,

                _ => return Err!("failed to create {full_path:?} directory: {err}"),
            }
        } else {
            checked = true;
            break;
        }
    }

    if !checked {
        check_directory(base)?;
    }

    for path in deferred_paths.iter().rev() {
        let full_path = base.join(path);
        fs::create_dir(&full_path).with_context(|| format!(
            "failed to create {full_path:?} directory"))?;
    }

    Ok(())
}

pub fn get_device_usage<P: AsRef<Path>>(path: P) -> GenericResult<(String, u8)> {
    _get_device_usage(path, &RunCommand)
}

fn _get_device_usage<P: AsRef<Path>>(path: P, provider: &dyn RunCommandProvider) -> GenericResult<(String, u8)> {
    let mut path = s!(path.as_ref().to_str().unwrap());

    // df gives a different output for "dir" and "dir/"
    if !path.ends_with('/') {
        path.push('/');
    }

    let output = provider.run_command("df", &[path])?;

    let get_parse_error = || {
        let error = "Got an unexpected output from `df`";
        debug!("{}:\n{}", error, output);
        Err(From::from(error))
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
        s!(captures.name("device").unwrap().as_str()),
        captures.name("use").unwrap().as_str().parse().unwrap(),
    ))
}

// Transmission 4.X has a bug due to which torrents are marked as downloaded before their renaming from *.part files.
fn open_downloaded_file<P: AsRef<Path>>(path: P) -> Result<File> {
    let path = path.as_ref();
    let start_time = Instant::now();
    let mut check_part_file = true;

    loop {
        match File::open(path) {
            Ok(file) => return Ok(file),
            Err(err) => {
                if err.kind() != ErrorKind::NotFound || !check_part_file {
                    return Err!("failed to open {path:?}: {err}");
                }

                let part_path = {
                    let mut part_path = path.as_os_str().to_owned();
                    part_path.push(".part");
                    PathBuf::from(part_path)
                };

                match fs::metadata(&part_path) {
                    Ok(_) => {
                        if start_time.elapsed().as_secs() >= 5 {
                            return Err!("{path:?} hasn't been downloaded ({part_path:?} still exists)");
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    },
                    Err(err) => match err.kind() {
                        ErrorKind::NotFound => {
                            check_part_file = false
                        },
                        _ => return Err!("{part_path:?}: {err}")
                    }
                }
            }
        }
    }
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
    use crate::util::process::tests::RunCommandMock;

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
