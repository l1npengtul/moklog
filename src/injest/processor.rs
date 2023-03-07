use crate::injest::static_file::new_filename;
use color_eyre::Result;
use dashmap::DashMap;
use lol_html::html_content::{Element, TextType};
use lol_html::{element, rewrite_str, text, HtmlRewriter, Settings};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use crate::mmap_load;

pub fn title_make_url_safe(title: &str) -> String {
    let mut no_whitespace = title.replace(" ", "-");
    url_escape::encode(&no_whitespace, &url_escape::PATH).to_string()
}

pub struct DocumentStatistics {
    pub characters: u64,
    pub words: u64,
}

pub fn static_file_rewriter(
    path: String,
    files: Arc<DashMap<String, String>>,
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
    files: Arc<DashMap<u64, PathBuf>>,
    element: &mut Element,
) {
    let (da_linkie, attr) = match (element.get_attribute("href"), element.get_attribute("src")) {
        (Some(linkie), None) => (linkie, "href"),
        (None, Some(linkie)) => (linkie, "src"),
        (_, _) => return,
    };

    if let Ok(_) = url::Url::parse(&da_linkie) {
        return;
    }

    let file_read = mmap_load!(&da_linkie);

    let (_, filename) = match new_filename(file_read, &da_linkie) {
        Some(h) => h,
        None => return,
    };

    let filename = format!("/{filename}");

    element.set_attribute(attr, &filename).unwrap();
}

pub struct ProcessedDocument {
    document: String,
    summary: String,
}

pub fn html_post_processor(
    path: &str,
    files: Arc<DashMap<u64, PathBuf>>,
    data_in: &str,
) -> Result<ProcessedDocument> {
    let character_count = AtomicU64::new(0);
    let mut skip: bool = false;

    let summary_generator = Settings {
        element_content_handlers: vec![
            element!("*", |el| {
                if character_count.load(Ordering::SeqCst) > 200 {
                    if el.tag_name() == "p" {
                        skip = true;
                    }
                }

                if skip {
                    el.remove();
                }
            }),
            text!("*", |txt| {
                if TextType::Data == txt.text_type() {
                    character_count.fetch_add(txt.as_str().len() as u64, Ordering::SeqCst);
                }
            }),
        ],
        ..Settings::default()
    };

    let fc = files.clone();
    let settings = Settings {
        element_content_handlers: vec![
            element!("a[href]|img[src]", |el| {
                static_file_rewrite_element(path, fc, el)
            }),
            element!("img|iframe|audio|video", |el| {
                el.set_attribute("loading", "lazy")
            }),
            element!("video", |el| { el.set_attribute("preload", "metadata") }),
        ],
        ..Default::default()
    };

    let new_document = ProcessedDocument {
        document: rewrite_str(data_in, settings)?,
        summary: rewrite_str(data_in, summary_generator)?,
    };

    Ok(new_document)
}
