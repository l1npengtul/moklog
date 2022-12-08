use color_eyre::{Report, Result};
use ignore::{Walk, WalkBuilder};
use itertools::Itertools;
use rhai::Dynamic;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use syntect::highlighting::{Theme, ThemeSet};
use tera::Tera;
use tokio::fs::File;
use tokio::io::AsyncReadExt;

pub struct SiteTheme {
    pub syntect_colors: ThemeSet,
    pub tera_templates: Tera,
    pub shortcode: HashMap<String, String>,
    pub functions: HashMap<String, String>,
    pub persistent_function_data: BTreeMap<String, BTreeMap<String, Dynamic>>,
    pub filters: HashMap<String, String>,
    pub metadata: SiteThemeMetadata,
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
    let template_metadata = toml::from_str::<SiteThemeMetadata>(&template_metadata)?;

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

    // compile css

    Ok(())
}
