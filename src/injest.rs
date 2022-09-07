use crate::{models::*, State, SITE_CONTENT};
use color_eyre::Result;
use ignore::{DirEntry, Error, Walk, WalkBuilder};
use sea_orm::EntityTrait;
use std::collections::HashMap;
use std::fs::FileType;
use std::{path::Path, sync::Arc};
use tokio::fs::File;
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

pub async fn update_site_content(state: Arc<State>) -> Result<Vec<SiteContentDiffElem>> {
    // explore the whole site
    // first get all the names
    let db_downloads = downloads::Entity::find().all(&state.database).await?;
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
    for pgcat in walker_with_ignores(SITE_CONTENT) {
        let page = match pgcat {
            Ok(v) => v,
            Err(why) => {
                warn!("Skipping file: {}", why);
                continue;
            }
        };
    }

    Err(())
}

#[derive(Clone, Debug, PartialEq)]
struct FileStruct {
    pub name: String,
    pub file_type: Option<FileType>,
    pub content: String,
    pub hash: u64,
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

        let mut content = String::new();
        File::open(file.path())
            .await?
            .read_to_string(&mut content)
            .await?;

        let (content, hash) = match spawn_blocking(move || {
            let ccc = content;
            let hash = seahash::hash(ccc.as_bytes());
            (ccc, hash)
        })
        .await
        {
            Ok(v) => v,
            Err(why) => {
                warn!("Skipping file: {}", why);
                continue;
            }
        };

        info!("Processed File: {:?}", file.path());
        files.push(FileStruct {
            name: file.file_name().to_string_lossy().to_string(),
            file_type: file.file_type(),
            content,
            hash,
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
