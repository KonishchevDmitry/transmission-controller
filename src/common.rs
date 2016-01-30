pub type EmptyResult = GenericResult<()>;
pub type GenericError = Box<::std::error::Error + Send + Sync>;
pub type GenericResult<T> = Result<T, GenericError>;

macro_rules! s {
    ($e:expr) => ($e.to_owned())
}

macro_rules! format_to {
    ($($arg:tt)*) => (::std::convert::From::from(format!($($arg)*)))
}

macro_rules! Err {
    ($($arg:tt)*) => (::std::result::Result::Err(format_to!($($arg)*)))
}
