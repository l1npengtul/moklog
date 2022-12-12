use base64::DecodeError;
use serde::{Deserialize, Serialize};
use tracing::instrument;

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

pub fn new_filename(file: impl AsRef<[u8]>, filename: String) -> (u64, String) {
    let hash = hash_file(file);
    let base64 = base64::encode(hash.to_le_bytes());
    (hash, format!("{base64}_{filename}"))
}

pub fn parse_filename(filename: String) -> Option<(u64, String)> {
    match filename.split_once("_") {
        Some((hash, filename)) => match base64::decode(hash) {
            Ok(data) => {
                let fh = u64::from_le_bytes(data.into());
                Some((fh, filename.to_string()))
            }
            Err(_) => return None,
        },
        None => None,
    }
}
