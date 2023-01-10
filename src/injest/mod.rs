use color_eyre::Result;
use relative_path::RelativePath;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub mod build;
pub mod generate;
pub mod processor;
pub mod static_file;
pub mod stylesheet;
pub mod templates;

pub fn path_relativizie(base: impl AsRef<Path>, item: impl AsRef<Path>) -> Result<String> {
    let base = RelativePath::new(base.as_ref());
    let item = RelativePath::new(item);
    let new = item.strip_prefix(base)?;
    Ok(new.to_string())
}
