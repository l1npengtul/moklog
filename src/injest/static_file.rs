use base64::DecodeError;
use serde::{Deserialize, Serialize};
use std::fmt::Display;
use std::path::{Path, PathBuf};
use tracing::instrument;
use color_eyre::Result;
use memmap2::Mmap;
use crate::injest::path_relativizie;

#[derive(Clone, Debug, Default, PartialOrd, PartialEq, Serialize, Deserialize)]
pub struct StaticFile {
    pub file_name: String,
    pub path: PathBuf,
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
    let file_name = filename.as_ref().file_name()?.to_str()?;
    let split = file_name.split_once(".");
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

pub fn process_static_file(file: impl AsRef<Path>) -> Option<(u64, StaticFile)> {
    let file = file.as_ref();
    if file.metadata()?.len() != 0 {
        let data = unsafe { Mmap::map(file.path())? };
        let mut filename = file.into_path();
        let last = filename.file_name().unwrap().to_str().unwrap_or_default();
        if let Some((hash, newfname)) = new_filename(data.as_ref(), last) {
            let filename = filename.with_file_name(newfname);
            let new_filename = path_relativizie(file, filename)?;
            Some((
                hash,
                StaticFile {
                    file_name: new_filename,
                    path: file.into_path(),
                })
            )
        } else {
            None
        }
    } else {
        None
    }
}
