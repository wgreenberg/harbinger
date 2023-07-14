use ::har::Error as HarError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum HarbingerError {
    #[error("io error")]
    IoError(#[from] std::io::Error),
    #[error("unsupported HAR version")]
    UnsupportedHarVersion,
    #[error("HAR error")]
    HarError(#[from] HarError),
    #[error("dump path exists! cowardly bailing")]
    DumpPathExists,
    #[error("Invalid HAR entry: invalid URI {uri}")]
    InvalidHarEntryUri { uri: String },
    #[error("Invalid HAR entry: invalid method {method}")]
    InvalidHarEntryMethod { method: String },
}
