use crate::{models::*, SITE_CONTENT, State};
use color_eyre::Result;
use ignore::{DirEntry, Error, Walk};
use sea_orm::EntityTrait;
use std::{path::Path, sync::Arc};
use tokio::fs::{File, read_dir};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use std::async_iter::from_iter;
use std::io::Read;
use tracing::error;
use tracing::log::warn;

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
    let downloads = downloads::Entity::find().all(&state.database).await?;
    let pages = pages::Entity::find().all(&state.database).await?;
    let raw_pages = raw_pages::Entity::find().all(&state.database).await?;
    let staticses = statics::Entity::find().all(&state.database).await?;
    let templateses = templates::Entity::find().all(&state.database).await?;

    let mut pengignore = String::new();
    File::open(Path::new(&format!("{SITE_CONTENT}/pengignore")))
        .await?
        .read_to_string(&mut pengignore)
        .await?;

    // templates
    let templates = {
        for item in Walk::new(format!("{SITE_CONTENT}/templates")) {
            let file = match item {
                Ok(v) => v,
                Err(_) => continue,
            };

            let mut content = String::new();
            File::open(file.path()).await?.read_to_string(&mut content).await?;
            let title = file.file_name().to_string_lossy().to_string();
            let hash =

        }
    }

    Err(())
}

async fn walk_subdirectory(dir: &str) -> Result<Vec<(u64, String)>> {
    let files = vec![];
    for item in Walk::new(format!("{SITE_CONTENT}/{dir}")) {

    }
}
