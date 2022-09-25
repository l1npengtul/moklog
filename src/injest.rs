use crate::{models::*, State, SITE_CONTENT};
use chrono::{DateTime, Utc};
use color_eyre::{Report, Result};
use ignore::{Walk, WalkBuilder};
use itertools::Itertools;
use lightningcss::{
    printer::PrinterOptions,
    stylesheet::{ParserOptions, StyleSheet},
};
use markdown_toc::Heading;
use minify_html::Cfg;
use pathdiff::diff_paths;
use pulldown_cmark::{html::push_html, Options, Parser};
use rsass::compile_scss;
use sea_orm::EntityTrait;
use seahash::hash;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};
use tantivy::{
    schema::{Schema, STORED, TEXT},
    Index,
};
use tera::{Context, Tera};
use tokio::{
    fs::{canonicalize, remove_dir_all, DirBuilder, File},
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
    pub title: String,
    pub date: DateTime<Utc>,
    pub tags: Vec<String>,
    pub category: String,
    pub author: String,
    pub slug: String,
    pub redirects: Vec<String>,
    pub template: Option<String>,
    pub generate_toc: bool,
}

#[derive(Copy, Clone, Debug, Ord, PartialOrd, Eq, PartialEq)]
pub enum CompiledFileType {
    Html,
    Js,
    Css,
    Scss,
    RawBinary,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DataType {
    Direct,
    Binary(Vec<u8>),
    String(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProcessedFile {
    pub path: PathBuf,
    pub ftype: CompiledFileType,
    pub hash: u64,
    pub data: DataType,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProcessedArticle {}

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

    let mut unprocessed_templates = walk_subdirectory(format!("{SITE_CONTENT}/templates")).await?;
    let mut processed_templates = HashSet::new();
    let mut items = Vec::new();

    let mut color_scheme = None;

    let site_content_dir_path = canonicalize(SITE_CONTENT).await?;

    let mut site_content_dir_path_template = canonicalize(SITE_CONTENT).await?;
    site_content_dir_path_template.push("templates");

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

        let mut relative_file_path =
            match diff_paths(&unprocessed_template.path, &site_content_dir_path_template) {
                Some(p) => p,
                None => {
                    warn!(
                        "Skipping file {:?}: Failed to construct pathdiff",
                        &unprocessed_template.path
                    );
                    continue;
                }
            };

        let extension = relative_file_path
            .extension()
            .map(|x| x.to_str())
            .flatten()
            .unwrap_or("");

        match extension {
            "" => {}
        }

        let processed_file = process_file(relative_file_path).await?;

        match processed_file.ftype {
            CompiledFileType::Html => {
                let path_as_str = relative_file_path.to_string_lossy().to_string();
                if !processed_templates.insert(path_as_str) {
                    warn!("Overwriting template {}: Already Exists", path_as_str);
                }
            }
            _ => {
                staticses.push(processed_file);
            }
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
    }

    let mut templater = Tera::default();
    let mut processed_articles = Vec::new();

    let mut schema = Schema::builder();
    let title = schema.add_text_field("title", TEXT | STORED);
    let author = schema.add_text_field("author", TEXT | STORED);
    let category = schema.add_text_field("category", TEXT | STORED);
    let tags = schema.add_json_field("tags", STORED);
    let date = schema.add_date_field("tags", STORED);
    let body = schema.add_text_field("body", TEXT);
    let schema = schema.build();

    let indx_dir = state.config.index_dir.clone();
    let _ = remove_dir_all(&indx_dir).await;
    DirBuilder::new().recursive(true).create(&indx_dir).await?;
    let mut indexer = spawn(move || Index::create_in_dir(indx_dir, schema)).await?;

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

                // get md contents
                let contents = split_twice.get(3).copied().unwrap_or("");
                let mut fence = None;
                let mut toc_cfg = markdown_toc::Config::default();
                toc_cfg.bullet = "-".to_string();
                // generate table of contents
                let table_of_contents = contents
                    .lines()
                    .filter(|line| match fence {
                        Some(tag) => {
                            if line.starts_with(tag) {
                                fence = None;
                            }
                            false
                        }
                        None => {
                            if line.starts_with("```") {
                                fence = Some("```");
                                false
                            } else if line.starts_with("~~~") {
                                fence = Some("~~~");
                                false
                            } else {
                                true
                            }
                        }
                    })
                    .map(Heading::from_str)
                    .filter_map(Result::ok)
                    .filter_map(|heading| heading.format(&toc_cfg))
                    .join("\n");

                // render to HTML
                let mut options = Options::empty();
                options.insert(Options::ENABLE_FOOTNOTES);
                options.insert(Options::ENABLE_HEADING_ATTRIBUTES);
                options.insert(Options::ENABLE_STRIKETHROUGH);
                options.insert(Options::ENABLE_SMART_PUNCTUATION);
                options.insert(Options::ENABLE_TABLES);

                let mut page_contents_rendered = spawn(|| {
                    let md_contents = Parser::new_ext(contents, options);
                    let mut contents = String::new();
                    push_html(&mut contents, md_contents);
                    contents
                })
                .await;

                let mut table_of_contents_rendered = spawn(|| {
                    let md_toc = Parser::new_ext(&table_of_contents, options);
                    let mut contents = String::new();
                    push_html(&mut contents, md_toc);
                    contents
                })
                .await;

                // set content
                let mut tera_context = Context::new();
                tera_context.insert("page.title", &header.title);
                tera_context.insert("page.date", &header.date);
                tera_context.insert("page.author", &header.author);
                tera_context.insert("page.category", &header.category);
                tera_context.insert("page.tags", &header.tags);
                tera_context.insert("page.redirects", &header.redirects);
                tera_context.insert("page.slug", &header.slug);
                tera_context.insert("page.do_generate_toc", &header.generate_toc);
                tera_context.insert("page.table_of_contents", &table_of_contents_rendered);
                tera_context.insert("page.content", &page_contents_rendered);

                // get template
                let template_key = match header.template {
                    Some(t) => t,
                    None => {
                        // get the category
                        let mut intemideraty = match f.category {
                            Some(c) => c,
                            None => match f.path.file_name() {
                                Some(os) => os.to_string_lossy().to_string(),
                                None => {
                                    warn!("Skipping File {:?}: Could not find template.", f.path);
                                    continue;
                                }
                            },
                        };
                        intemideraty += ".html";
                        intemideraty
                    }
                };
                let template = match processed_templates.get(&template_key) {
                    None => {
                        warn!("Skipping File {:?}: Could not find template.", f.path);
                        continue;
                    }
                    Some(t) => t,
                };

                if let Err(why) = templater.add_template_file(template, Some(&template_key)) {
                    warn!(
                        "Skipping File {:?}: Could add template {} due to {}.",
                        f.path, template, why
                    );
                    continue;
                }

                let rendered = match templater.render(&template_key, &tera_context) {
                    Ok(r) => r,
                    Err(why) => {
                        warn!("Skipping File {:?}: {}", f.path, why);
                        continue;
                    }
                };
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

async fn process_file(file: impl AsRef<Path>) -> Result<ProcessedFile> {
    let path = file.as_ref().to_path_buf();
    let extension = path.extension().map(|x| x.to_str()).flatten().unwrap_or("");

    let processed = match extension {
        "html" => {
            let mut contents = String::new();
            File::open(&path)
                .await?
                .read_to_string(&mut contents)
                .await?;
            let hclone = contents.clone();
            let hash = spawn(move || hash(hclone.as_bytes())).await;

            ProcessedFile {
                path,
                ftype: CompiledFileType::Html,
                hash,
                data: DataType::String(contents),
            }
        }
        "js" => {
            let mut data = Vec::new();
            File::open(&path).await?.read_to_end(&mut data).await?;
            let compiled = spawn(|| -> Result<Vec<u8>> {
                let mut optimized = Vec::with_capacity(data.len());
                minify_js::minify(data, &mut optimized)
                    .map_err(|x| Report::msg(format!("{x:?}")))?;
                Ok(optimized)
            })
            .await?;
            let hclone = compiled.clone();
            let hash = spawn(move || hash(hclone.as_slice())).await;

            ProcessedFile {
                path,
                ftype: CompiledFileType::Js,
                hash,
                data: DataType::Binary(compiled),
            }
        }
        "css" => {
            let mut contents = String::new();
            File::open(&path)
                .await?
                .read_to_string(&mut contents)
                .await?;
            let compiled = spawn(move || -> Result<String> {
                let compiler = StyleSheet::parse(contents.as_str(), ParserOptions::default())
                    .map_err(|why| Report::msg(why.to_string()))?;
                let result = compiler.to_css(PrinterOptions::default())?;
                Ok(result.code)
            })
            .await?;
            let hclone = compiled.clone();
            let hash = spawn(move || hash(hclone.as_bytes())).await;

            ProcessedFile {
                path,
                ftype: CompiledFileType::Css,
                hash,
                data: DataType::String(compiled),
            }
        }
        "sass" => {
            let mut contents = Vec::new();
            File::open(&path).await?.read_to_end(&mut contents).await?;
            let compiled = spawn(move || -> Result<String> {
                let compiled_css = String::from_utf8(compile_scss(&contents, Default::default())?)?;
                let compiler = StyleSheet::parse(compiled_css.as_str(), ParserOptions::default())
                    .map_err(|why| Report::msg(why.to_string()))?;
                let result = compiler.to_css(PrinterOptions::default())?;
                Ok(result.code)
            })
            .await?;
            let hclone = compiled.clone();
            let hash = spawn(move || hash(hclone.as_bytes())).await;

            ProcessedFile {
                path,
                ftype: CompiledFileType::Css,
                hash,
                data: DataType::String(compiled),
            }
        }
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "wasm" => {
            let mut data = Vec::new();
            File::open(&path).await?.read_to_end(&mut data).await?;
            let hclone = data.clone();
            let hash = spawn(move || hash(hclone.as_slice())).await;
            ProcessedFile {
                path,
                ftype: CompiledFileType::RawBinary,
                hash,
                data: DataType::Binary(data),
            }
        }
        _ => return Err(Report::msg("unknown file format")), // TODO
    };
    Ok(processed)
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
