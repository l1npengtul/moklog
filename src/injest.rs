use crate::{models::*, State, SITE_CONTENT};
use chrono::{DateTime, Utc};
use color_eyre::{Report, Result};
use ignore::{Walk, WalkBuilder};
use itertools::Itertools;
use minify_html::Cfg;
use pathdiff::diff_paths;
use sea_orm::EntityTrait;
use seahash::hash;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::{
    fs::{canonicalize, File},
    io::AsyncReadExt,
    process::Command,
};
use tokio_rayon::spawn;
use tracing::{info, log::warn};

const MINIFY_SETTINGS: Cfg = Cfg {
    do_not_minify_doctype: true,
    ensure_spec_compliant_unquoted_attribute_values: true,
    keep_closing_tags: false,
    keep_html_and_head_opening_tags: false,
    keep_spaces_between_attributes: false,
    keep_comments: false,
    minify_css: true,
    minify_js: true,
    remove_bangs: true,
    remove_processing_instructions: true,
};

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

#[derive(Clone, Debug, PartialEq)]
struct Template {
    pub hash: u64,
    pub contents: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct ArticleMeta {
    pub date: DateTime<Utc>,
    pub tags: Vec<String>,
    pub category: String,
    pub author: String,
    pub title: String,
    pub slug: String,
    pub redirects: Vec<String>,
    pub template: Option<String>,
}

pub async fn update_site_content(state: Arc<State>) -> Result<Vec<SiteContentDiffElem>> {
    // explore the whole site
    // first get all the names
    let db_pages = pages::Entity::find().all(&state.database).await?;
    let db_raw_pages = raw_pages::Entity::find().all(&state.database).await?;
    let db_staticses = statics::Entity::find().all(&state.database).await?;
    let db_templateses = templates::Entity::find().all(&state.database).await?;

    let mut pages = Vec::with_capacity(db_pages.len());
    let mut raw_pages = Vec::with_capacity(db_raw_pages.len());
    let mut staticses = Vec::with_capacity(db_staticses.len());
    let mut templateses = Vec::with_capacity(db_templateses.len());

    let mut pengignore = String::new();
    File::open(Path::new(&format!("{SITE_CONTENT}/pengignore")))
        .await?
        .read_to_string(&mut pengignore)
        .await?;

    let unprocessed_templates = walk_subdirectory(format!("{SITE_CONTENT}/templates")).await?;
    let mut processed_templates = HashMap::new();
    let mut items = Vec::new();

    let site_content_dir_path = canonicalize(SITE_CONTENT).await?;

    for unprocessed_template in unprocessed_templates {
        let file_name = match unprocessed_template
            .path
            .file_name()
            .map(|x| x.to_str())
            .flatten()
        {
            Some(name) => name.to_string(),
            None => {
                return Err(Report::msg(format!(
                    "Failed to process template {:?}: Bad File Name",
                    unprocessed_template.path
                )));
            }
        };

        match unprocessed_template
            .path
            .extension()
            .unwrap_or("".as_ref())
            .to_str()
            .unwrap_or("")
        {
            "html" => {
                let mut contents = String::new();
                File::open(unprocessed_template.path)
                    .await?
                    .read_to_string(&mut contents)
                    .await?;
                let hash = spawn(|| hash(contents.as_bytes())).await;
                processed_templates.insert(file_name.clone(), Template { hash, contents });
            }
            "js" => {
                let mut data = Vec::new();
                File::open(unprocessed_template.path)
                    .await?
                    .read_to_end(&mut data)
                    .await?;
                let mut optimized = Vec::with_capacity(data.len());
                spawn(|| {
                    minify_js::minify(data, &mut optimized)
                        .map_err(|x| Report::msg(format!("{x:?}")))
                })
                .await?;
                let hash = spawn(|| hash(optimized.as_slice())).await;
            }
            "sass" => {}
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "wasm" => {}
            _ => continue,
        }
    }

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

    for f in items {
        if f.extension.is_none() {
            warn!("Skipping file {:?}: No file extension", f.path);
            continue;
        }

        let mut read_file = File::open(&f.path).await?;

        match f.extension.unwrap().as_str() {
            "md" => {
                // read string
                let mut file_contents = String::new();
                if let Err(why) = read_file.read_to_string(&mut file_contents).await {
                    warn!("Skipping file {:?}: {}", f.path, why);
                    continue;
                }
                // read header TOML
                let split_twice = file_contents.splitn(2, "+++").collect_vec();
                if split_twice.len() != 4 {
                    warn!("Skipping file {:?}: Bad split", f.path);
                    continue;
                }
                let header = match toml::from_str::<ArticleMeta>(split_twice.get(1).unwrap()) {
                    Ok(meta) => meta,
                    Err(why) => {
                        warn!("Skipping file {:?}: {}", f.path, why);
                        continue;
                    }
                };
                let contents = split_twice.get(3).copied().unwrap_or("");
            }
            "js" | "css" | "html" => {}
            "sass" => {}
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "wasm" => {}
            other_ext => {}
        }
    }
    Err(())
}

async fn hash_file(file: impl AsRef<Path>) -> Result<u64> {
    let mut file_bin = Vec::new();
    File::open(file).await?.read_to_end(&mut file_bin).await?;
    Ok(spawn(move || hash(&file_bin)).await)
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
