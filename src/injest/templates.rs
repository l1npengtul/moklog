use crate::injest::stylesheet::{compile_sass, optimize_css};
use crate::injest::StaticFile;
use color_eyre::{Report, Result};
use ignore::{Walk, WalkBuilder};
use itertools::Itertools;
use memmap2::Mmap;
use minify_js::TopLevelMode;
use rhai::Dynamic;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use syntect::highlighting::{Theme, ThemeSet};
use tera::Tera;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tracing_subscriber::fmt::format;

pub struct SiteTheme {
    pub metadata: SiteThemeMetadata,
    pub syntect_colors: ThemeSet,
    pub tera_templates: Tera,
    pub shortcode: HashMap<String, String>,
    pub functions: HashMap<String, String>,
    pub filters: HashMap<String, String>,
    pub styles: HashMap<String, String>,
    pub js_scripts: HashMap<String, String>,
    pub files: HashSet<String, StaticFile>,
}

#[derive(Serialize, Deserialize)]
pub struct SiteThemeMetadata {
    pub authors: Vec<String>,
    pub name: String,
    pub link: String,
    pub version: Version,
}

pub async fn build_site_theme() -> Result<SiteTheme> {
    const TEMPLATE_DIR: &str = ".gumilgi_site/theme";

    macro_rules! make_template_walker {
        ($path:expr) => {{
            WalkBuilder::new($path)
                .ignore(true)
                .add_custom_ignore_filename(".gmignore")
                .build()
        }};
    }

    // template metadata

    let mut template_metadata = String::new();
    File::open(TEMPLATE_DIR)
        .await?
        .read_to_string(&mut template_metadata)
        .await?;
    let metadata = toml::from_str::<SiteThemeMetadata>(&template_metadata)?;

    // syntax highlighting

    let mut syntect_colors = ThemeSet::default();
    syntect_colors.add_from_folder(format!("{TEMPLATE_DIR}/highlighting"))?;

    // add tera templates

    let mut tera_templates = Tera::default();
    let mut template_files = vec![];
    for template_entry in make_template_walker!(format!("{TEMPLATE_DIR}/templates")) {
        let template_entry = template_entry?;
        let file_extension = template_entry
            .path()
            .extension()
            .unwrap_or_default()
            .to_str()
            .unwrap_or_default();
        if file_extension != "html" || file_extension != "tera" {
            continue;
        }
        template_files.push(template_entry.into_path());
    }
    tera_templates.add_template_files(template_files.into_iter())?;

    // compile scss, css

    let mut styles = HashMap::new();
    for style_entry in make_template_walker!(format!("{TEMPLATE_DIR}/stylesheets")) {
        let style_entry = style_entry?;
        let file_extension = style_entry
            .path()
            .extension()
            .unwrap_or_default()
            .to_str()
            .unwrap_or_default();
        let file_length = style_entry.metadata()?.len() as usize;
        if file_length == 0 {
            continue;
        }
        let file_name = style_entry
            .file_name()
            .to_str()
            .ok_or(Report::new("bad file name"))?
            .to_string();
        if file_extension == "css" {
            let memmap = unsafe { Mmap::map(style_entry.path())? }.to_str()?;
            let optimized = optimize_css(memmap).await?;
            styles.insert(file_name, optimized);
        } else if file_extension == "scss" {
            let memmap = unsafe { Mmap::map(style_entry.path())? };
            let compiled = compile_sass(memmap.as_ref()).await?;
            let optimized = optimize_css(&compiled).await?;
            styles.insert(file_name, optimized);
        }
    }

    // minify JS

    let mut js_scripts = HashMap::new();
    for script_entry in make_template_walker!(format!("{TEMPLATE_DIR}/scripts")) {
        let script_entry = script_entry?;
        let file_extension = script_entry
            .path()
            .extension()
            .unwrap_or_default()
            .to_str()
            .unwrap_or_default();
        let file_length = script_entry.metadata()?.len() as usize;
        if file_length == 0 {
            continue;
        }
        let file_name = script_entry
            .file_name()
            .to_str()
            .ok_or(Report::new("bad file name"))?
            .to_string();
        if file_extension == "js" {
            let mut load = Vec::with_capacity(file_length);
            File::open(script_entry.path())
                .await?
                .read_to_end(&mut load)
                .await?;
            let mut out = Vec::new();
            minify_js::minify(TopLevelMode::Global, load, &mut out)?;
            js_scripts.insert(file_name, String::from_utf8(out)?);
        }
    }

    // load shortcodes

    Ok(SiteTheme {
        syntect_colors,
        tera_templates,
        shortcode: Default::default(),
        functions: Default::default(),
        filters: Default::default(),
        metadata,
        styles,
        js_scripts,
    })
}
