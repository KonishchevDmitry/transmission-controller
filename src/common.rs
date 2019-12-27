pub type EmptyResult = GenericResult<()>;
pub type GenericResult<T> = Result<T, GenericError>;
pub type GenericError = Box<dyn std::error::Error + Send + Sync>;

macro_rules! s {
    ($e:expr) => ($e.to_owned())
}

macro_rules! format_to {
    ($($arg:tt)*) => (::std::convert::From::from(format!($($arg)*)))
}

macro_rules! Err {
    ($($arg:tt)*) => (::std::result::Result::Err(format_to!($($arg)*)))
}
