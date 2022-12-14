use chrono::{DateTime, Utc};
use color_eyre::Result;
use pulldown_cmark::{Event, Options, Parser, Tag};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::instrument;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageMeta {
    pub title: String,
    pub author: String,
    pub date: DateTime<Utc>,
    pub redirects: Vec<String>,
    pub template: Option<String>,
    pub tags: Vec<String>,
    pub lang: Option<String>,
    pub alt_langs: Option<HashMap<String, String>>,
    pub index: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeriesMeta {
    pub title: String,
    pub author: String,
    pub redirects: Vec<String>,
    pub tags: Vec<String>,
    pub lang: Option<String>,
    pub alt_langs: Option<HashMap<String, String>>,
    pub complete: bool,
    pub index: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CategoryMeta {
    pub default_lang: Option<String>,
    pub default_template: Option<String>,
    pub include_rss: bool,
    pub index: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubCategoryMeta {
    pub default_lang: Option<String>,
    pub default_template: Option<String>,
    pub include_rss: bool,
    pub index: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeriesData {
    pub part: i32,
    pub end: i32,
    pub others: Vec<String>,
    pub series: String,
}

struct TableOfContents<'a> {
    pub title_text: &'a str,
    pub level: u32,
}

pub fn build_page(
    page_meta: &PageMeta,
    series_data: Option<&SeriesData>,
    lang_override: Option<&str>,
    content: &str,
) -> Result<String> {
    let renderer = Parser::new_ext(content, Options::all());
    let table_of_content = Vec::new();
}
