pub type GenericResult<T> = Result<T, Box<::std::error::Error + Send + Sync>>;

macro_rules! s {
    ($e:expr) => ($e.to_string())
}
