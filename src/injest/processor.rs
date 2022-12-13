use crate::injest::static_file::StaticFile;
use color_eyre::Result;
use dashmap::mapref::one::Ref;
use dashmap::DashMap;
use lol_html::html_content::Element;
use lol_html::{element, HtmlRewriter, Settings};
use std::io::Write;
use std::sync::Arc;

pub struct DocumentStatistics {
    pub characters: u64,
    pub words: u64,
}

pub fn static_file_rewriter(
    path: String,
    files: Arc<DashMap<String, StaticFile>>,
    out: &mut impl Write,
    data_in: impl AsRef<[u8]>,
) -> Result<()> {
    let mut rewriter = HtmlRewriter::new(
        Settings {
            element_content_handlers: vec![element!("[href]", |el| {
                static_file_rewrite_element(path.as_str(), files.clone(), el)
            })],
            document_content_handlers: vec![],
            ..Default::default()
        },
        |data| out.write_all(data),
    );

    rewriter.write(data_in.as_ref())?;
    Ok(())
}

fn static_file_rewrite_element(
    path: &str,
    files: Arc<DashMap<String, StaticFile>>,
    element: &mut Element,
) {
    let href = element.get_attribute("href").unwrap();

    if let Ok(_) = url::Url::parse(&href) {
        return;
    }

    let file = match if href.starts_with("/") {
        files.get(href.strip_prefix("/").unwrap_or_default())
    } else {
        if path.ends_with("/") {
            files.get(&format!("{path}{href}"))
        } else {
            files.get(&format!("{path}/{href}"))
        }
    } {
        Some(s) => s,
        None => return,
    };

    element.set_attribute("href", &file.file_name).unwrap();
}

pub fn html_post_processor(
    path: &str,
    files: Arc<DashMap<String, StaticFile>>,
    rhai_transformers: Arc<DashMap<String, String>>,
    rewrite_plugins: Arc<DashMap<String, String>>,
    data_in: impl AsRef<[u8]>,
) -> Result<(String, DocumentStatistics)> {
}
