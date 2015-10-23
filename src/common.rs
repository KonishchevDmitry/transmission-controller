pub type GenericResult<T> = Result<T, Box<::std::error::Error + Send + Sync>>;

macro_rules! s {
    ($e:expr) => ($e.to_owned())
}

// FIXME: use everywhere?
macro_rules! Err {
    ($($arg:tt)*) => (Err(From::from(format!($($arg)*))))
}
