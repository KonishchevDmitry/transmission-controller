use std::fs;
use std::io;
use std::path::Path;

use regex::Regex;

use common::{EmptyResult, GenericResult};
use util;

pub fn copy_file<S: AsRef<Path>, D: AsRef<Path>>(src: &S, dst: &D) -> EmptyResult {
    let mut src_file = try!(fs::File::open(src).map_err(|e| format!(
        "Failed to open '{}': {}", src.as_ref().display(), e)));

    // TODO: use O_CREAT & O_EXCL
    try!(match fs::metadata(dst) {
        Ok(_) => Err(format!("'{}' already exists", dst.as_ref().display())),
        Err(err) => match err.kind() {
            io::ErrorKind::NotFound => Ok(()),
            _ => Err(format!("Failed to create '{}': {}", dst.as_ref().display(), err))
        }
    });

    let mut dst_file = try!(fs::File::create(dst).map_err(|e| format!(
        "Failed to create '{}': {}", dst.as_ref().display(), e)));

    try!(io::copy(&mut src_file, &mut dst_file));

    Ok(())
}

pub fn get_device_usage<P: AsRef<Path>>(path: &P) -> GenericResult<(String, u8)> {
    let mut path = s!(path.as_ref().to_str().unwrap());

    // df gives a different output for "dir" and "dir/"
    if !path.ends_with('/') {
        path.push('/');
    }

    let output = try!(util::run_command("df", &[path]));

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
