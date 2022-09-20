use crate::{models::*, State, SITE_CONTENT};
use bytes::Bytes;
use color_eyre::{Report, Result};
use ignore::{DirEntry, Error, Walk, WalkBuilder};
use pathdiff::diff_paths;
use sea_orm::EntityTrait;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::FileType;
use std::io::Read;
use std::path::PathBuf;
use std::{path::Path, sync::Arc};
use tantivy::HasLen;
use tokio::fs::{canonicalize, File};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::task::spawn_blocking;
use tracing::log::warn;
use tracing::{error, info};

pub async fn pull_git(state: Arc<State>) -> Result<()> {
    if Path::new(SITE_CONTENT).is_dir() {
        Command::new("git").arg("pull").spawn()?.wait().await?;
    } else {
        Command::new("git")
            .arg("clone")
            .arg("-b")
            .arg(state.config.branch())
            .arg(state.config.git())
            .arg(SITE_CONTENT)
            .arg("--recursive")
            .spawn()?
            .wait()
            .await?;
    }

    Ok(())
}

pub enum SiteContentDiffElem {
    Removed(u64),
    Added(u64),
}

#[derive(Clone, Debug, PartialEq)]
struct RegisteredFile {
    pub path: PathBuf,
    pub extension: Option<String>,
    pub category: Option<String>,
    pub subcategory: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
struct FileStruct {
    pub name: String,
    pub path: PathBuf,
}

pub async fn update_site_content(state: Arc<State>) -> Result<Vec<SiteContentDiffElem>> {
    // explore the whole site
    // first get all the names
    let db_pages = pages::Entity::find().all(&state.database).await?;
    let db_raw_pages = raw_pages::Entity::find().all(&state.database).await?;
    let db_staticses = statics::Entity::find().all(&state.database).await?;
    let db_templateses = templates::Entity::find().all(&state.database).await?;

    let mut pengignore = String::new();
    File::open(Path::new(&format!("{SITE_CONTENT}/pengignore")))
        .await?
        .read_to_string(&mut pengignore)
        .await?;

    let mut templates = walk_subdirectory(format!("{SITE_CONTENT}/templates")).await?;
    let mut processed_templates = Vec::new();
    let mut items = Vec::new();

    let site_content_dir_path = canonicalize(SITE_CONTENT).await?;

    for item in walker_with_ignores(SITE_CONTENT) {
        let file = match item {
            Ok(f) => f,
            Err(why) => {
                warn!("Skipping file <unknown>: {}", why);
                continue;
            }
        };

        let extension = if let Some(e) = file.path().extension() {
            match e.to_str() {
                Some(s) => s.to_string(),
                None => {
                    warn!("Skipping file {:?}: Bad file extension", file.path());
                    continue;
                }
            }
        } else {
            warn!("Skipping file {:?}: No file extension", file.path());
            continue;
        };

        let mut relative_dir_path = match diff_paths(file.path(), &site_content_dir_path) {
            Some(p) => p,
            None => {
                warn!(
                    "Skipping file {:?}: Failed to construct pathdiff",
                    file.path()
                );
                continue;
            }
        };

        let file_path_depth = relative_dir_path.iter().count() - 1;

        let hash = match hash_file(file.path()).await {
            Ok(h) => h,
            Err(why) => {
                warn!(
                    "Skipping file {:?}: Failed to construct file hash: {why:?}",
                    file.path()
                );
                continue;
            }
        };

        let category = relative_dir_path
            .iter()
            .nth(0)
            .map(|ostr| ostr.to_str())
            .flatten()
            .map(ToString::to_string);
        let subcategory = relative_dir_path
            .iter()
            .nth(1)
            .map(|ostr| ostr.to_str())
            .flatten()
            .map(ToString::to_string);
        let extension = file
            .into_path()
            .extension()
            .map(|ostr| ostr.to_str())
            .flatten()
            .map(ToString::to_string);

        items.push(RegisteredFile {
            path: relative_dir_path,
            extension,
            category,
            subcategory,
        });

        // match extension.as_str() {
        //     "md" => {
        //         // get the category
        //         // 1 => just an article
        //         // 2 => a series :pog:
        //         // more => owo wtf is this???????
        //
        //         if file_path_depth == 1 {
        //
        //         } else if file_path_depth == 2 {
        //         }
        //     }
        //     "html" => {}
        //     "css" | "js" => {}
        //     "sass" => {}
        //     "png" | "jpg" | "jpeg" | "gif" => {}
        //     ext => {
        //         // TODO: Custom file handler plugins here
        //         warn!(
        //             "Skipping file {:?}: Unknown file extension {ext}",
        //             file.path()
        //         );
        //         continue;
        //     }
        // }
    }

    items.into_iter().for_each(|f| {
        if f.extension.is_none() {
            return;
        }

        let mut read_file = match std::fs::File::open(&f.path) {
            Ok(file) => file,
            Err(why) => {
                warn!("Skipping file {:?}: {:?}", &f.path, why);
                return;
            }
        };

        match f.extension.unwrap().as_str() {
            "md" => {
                // read string
                let mut file_contents = String::new();
                if let Err(why) = read_file.read_to_string(&mut file_contents) {
                    warn!("Skipping file {:?}: {:?}", f.path, why);
                    return;
                }
                // read header TOML
                let split_twice = file_contents.splitn(2, "+++");
            }
            "js" | "css" | "html" => {}
            "sass" => {}
            "png" | "jpg" | "jpeg" | "gif" | "webp" => {}
            other_ext => {}
        }
    });

    Err(())
}

async fn hash_file(file: impl AsRef<Path>) -> Result<u64> {
    let mut file_bin = Vec::new();
    File::open(file).await?.read_to_end(&mut file_bin).await?;
    match spawn_blocking(move || seahash::hash(&file_bin)).await {
        Ok(u) => Ok(u),
        Err(why) => Err(Report::new(why)),
    }
}

async fn walk_subdirectory(dir: impl AsRef<Path>) -> Result<Vec<FileStruct>> {
    let mut files = vec![];
    for item in Walk::new(dir) {
        let file = match item {
            Ok(f) => f,
            Err(why) => {
                warn!("Skipping file: {}", why);
                continue;
            }
        };

        info!("Processed File: {:?}", file.path());
        files.push(FileStruct {
            name: file.file_name().to_string_lossy().to_string(),
            path: file.into_path(),
        })
    }
    Ok(files)
}

fn walker_with_ignores(path: impl AsRef<Path>) -> Walk {
    WalkBuilder::new(path)
        .add_custom_ignore_filename("templates")
        .add_custom_ignore_filename("styles")
        .add_custom_ignore_filename("misc")
        .add_custom_ignore_filename("static")
        .add_custom_ignore_filename("downloads")
        .add_custom_ignore_filename("error")
        .build()
}
