use crate::{models::*, State, SITE_CONTENT};
use bytes::Bytes;
use color_eyre::Result;
use ignore::{DirEntry, Error, Walk, WalkBuilder};
use pathdiff::diff_paths;
use sea_orm::EntityTrait;
use std::collections::HashMap;
use std::fs::FileType;
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

enum FileToProcess {
    Raw(PathBuf),
    Process(PathBuf),
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

    let mut pages = HashMap::new();
    let mut series = HashMap::new();
    let mut static_fies = HashMap::new();

    let site_content_dir_path = canonicalize(SITE_CONTENT).await?;

    for item in walker_with_ignores(SITE_CONTENT) {
        let file = match item {
            Ok(f) => f,
            Err(why) => {
                warn!("Skipping file <unknown>: {}", why);
                continue;
            }
        };

        // get category of file
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

        // match the file extension
        match 
        // we only process markdown
    }

    Err(())
}

#[derive(Clone, Debug, PartialEq)]
struct FileStruct {
    pub name: String,
    pub path: PathBuf,
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
