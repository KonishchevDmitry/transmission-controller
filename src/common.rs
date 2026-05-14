// These types are deprecated now. anyhow should be used instead.
pub type EmptyResult = GenericResult<()>;
pub type GenericResult<T> = Result<T, GenericError>;
pub type GenericError = Box<dyn std::error::Error + Send + Sync>;

macro_rules! s {
    ($e:expr) => ($e.to_owned())
}

macro_rules! Err {
    ($($arg:tt)*) => (::std::result::Result::Err(::anyhow::anyhow!($($arg)*).into()))
}