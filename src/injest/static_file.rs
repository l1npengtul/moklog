use base64::DecodeError;
use serde::{Deserialize, Serialize};
use std::fmt::Display;
use std::path::Path;
use tracing::instrument;
use color_eyre::Result;

#[derive(Clone, Debug, Default, PartialOrd, PartialEq, Serialize, Deserialize)]
pub struct StaticFile {
    pub file_hash: u64,
    pub file_name: String,
    pub path: String,
}

#[derive(Clone, Debug, Default, PartialOrd, PartialEq, Serialize, Deserialize)]
pub struct NewFilename {
    pub hash: u64,
    pub new_filename: String,
}

#[derive(Clone, Debug, Default, PartialOrd, PartialEq, Serialize, Deserialize)]
pub struct ParsedFilename {
    pub hash: u64,
    pub raw_filename: String,
}

pub fn hash_file(file: impl AsRef<[u8]>) -> u64 {
    seahash::hash(file)
}

pub fn new_filename(file: impl AsRef<[u8]>, filename: impl AsRef<Path>) -> Option<(u64, String)> {
    let hash = hash_file(file);
    let base64 = base64::encode(hash.to_le_bytes());
    let split = filename.as_ref().split_once(".");
    match split {
        Some((fname, ext)) => Some((hash, format!("{fname}.{base64}.{ext}"))),
        None => None,
    }
}

pub fn parse_filename(filename: impl AsRef<str>) -> Option<(u64, String)> {
    match filename.split_once(".") {
        Some((_, hash_and_ext)) => match hash_and_ext.split_once(".") {
            Some((hash, _)) => match base64::decode(hash) {
                Ok(data) => {
                    let fh = u64::from_le_bytes(data.into());
                    Some((fh, format!()))
                }
                Err(_) => None,
            },
            None => None,
        },
        None => None,
    }
}

pub fn optimize_file(path: impl AsRef<Path>, extension: &str, data: impl AsRef<[u8]>) -> Result<Option<Box<impl AsRef<[u8]>>>> {
    match extension {
        ""
    }
}
