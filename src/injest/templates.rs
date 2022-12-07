use color_eyre::Result;
use ignore::{Walk, WalkBuilder};
use rhai::Dynamic;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::BTreeMap;
use syntect::highlighting::Theme;
use tera::Tera;
use tokio::fs::File;
use tokio::io::AsyncReadExt;

pub struct SiteTemplate {
    pub syntect_color: Option<Theme>,
    pub tera_templates: Tera,
    pub shortcode: BTreeMap<String, String>,
    pub functions: BTreeMap<String, String>,
    pub persistent_function_data: BTreeMap<String, BTreeMap<String, Dynamic>>,
    pub filters: BTreeMap<String, String>,
    pub metadata: SiteTemplateMetadata,
}

#[derive(Serialize, Deserialize)]
pub struct SiteTemplateMetadata {
    pub authors: Vec<String>,
    pub name: String,
    pub link: String,
    pub version: Version,
}

pub async fn build_templates() -> Result<SiteTemplate> {
    const TEMPLATE_DIR: &str = ".gumilgi_site/template";

    // read .gmgignore and other standard ignores
    let ignored_walker = WalkBuilder::new(TEMPLATE_DIR)
        .ignore(true)
        .add_custom_ignore_filename(".gmignore")
        .build();

    let mut template_metadata = String::new();
    File::open(TEMPLATE_DIR)
        .await?
        .read_to_string(&mut template_metadata)
        .await?;
    let template_metadata = toml::from_str::<SiteTemplateMetadata>(&template_metadata)?;
}
