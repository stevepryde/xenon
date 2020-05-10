use thiserror::Error;

pub type XenonResult<T> = Result<T, XenonError>;

#[derive(Error, Debug)]
pub enum XenonError {}
