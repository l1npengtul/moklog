use chrono::{DateTime, Utc};
use color_eyre::{Report, Result};
use once_cell::sync::Lazy;
use pulldown_cmark::{html, CodeBlockKind, Event, Options, Parser, Tag};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap};
use tera::{Context, Tera};
use tracing::log::warn;
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SiteServe {
    Cache {
        size: i32,
        store: String,
    },
    Memory,
}

impl Default for SiteServe {
    fn default() -> Self {
        SiteServe::Cache { size: 25, store: "srv".to_string() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SiteMeta {
    pub site_name: String,
    pub serve: SiteServe,
    pub categories: Vec<String>,
    pub rss: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageMeta {
    pub title: String,
    pub author: String,
    pub rehydrate: i64,
    pub date: DateTime<Utc>,
    pub redirects: Vec<String>,
    pub template: Option<String>,
    pub tags: Vec<String>,
    pub lang: Option<String>,
    pub alt_langs: Option<Vec<String>>,
    pub min_permission: Option<String>,
    pub index: bool,
    pub rss: bool,
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
    pub rss: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CategoryMeta {
    pub display: String,
    pub translations: Option<Vec<(String, String)>>,
    pub default_template: Option<String>,
    pub include_rss: bool,
    pub index: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeriesData {
    pub series_name: String,
    pub base_path: String,
    pub part: i32,
    pub parts_names_parts: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Children {
    pub categories: Vec<(String, String)>,
    pub subcategories: Vec<(String, String)>,
    pub pages: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WhereTheFuckAreYou {
    pub pagetype: String,
    pub path: Vec<String>,
}

struct TableOfContents<'a> {
    pub title_text: &'a str,
    pub level: u32,
}

pub fn build_page(
    tera: &Tera,
    site_meta: &SiteMeta,
    children: &Children,
    pagetype: &WhereTheFuckAreYou,
    series_meta: &Option<SeriesData>,
    page_meta: &PageMeta,
    language: Option<&str>,
    content: &str,
) -> Result<String> {
    let mut renderer = Parser::new_ext(content, Options::all());
    let table_of_content = pulldown_cmark_toc::TableOfContents::new(content);
    let words = words_count::count(content).words;

    let mut rendered_html = String::new();
    parser_to_writer(&mut rendered_html, renderer)?;

    let mut context = Context::new();
    context.insert("content", &rendered_html);
    context.insert("title", &page_meta.title);
    context.insert("tags", &page_meta.tags);
    context.insert("author", &page_meta.author);
    context.insert("date", &page_meta.date.date_naive().to_string());
    context.insert("current_lang", language.unwrap_or_default());
    context.insert("other_langs", &page_meta.alt_langs.unwrap_or_default());
    context.insert("toc", &table_of_content.to_cmark());
    context.insert("min_permission", &page_meta.min_permission);
    context.insert("indexed", &page_meta.index);
    context.insert("redirects", &page_meta.redirects);
    context.insert("rss", &page_meta.rss);
    context.insert("word_count", &words);
    context.insert("reading_time_min", &((words as f32/25.0).round() / 10.0));

    // site
    context.insert("site.name", &site_meta.site_name);
    context.insert("site.headers", &site_meta.categories);
    context.insert("site.rss", &site_meta.rss_link);

    // children (vaush????)
    context.insert("children.categories", &children.categories);
    context.insert("children.pages", &children.pages);
    context.insert("children.subcategories", &children.subcategories);

    // pagetype
    context.insert("page.type", &pagetype.pagetype);
    context.insert("page.path", &pagetype.path);

    if let Some(series_data) = series_meta {
        context.insert("series.name", &series_data.series_name);
        context.insert("series.base_path", &series_data.base_path);
        context.insert("series.current_part", &series_data.part);
        let (titles, links) = series_data.parts_names_parts.iter().map(|(a, b)|, (a, b)).collect::<(Vec<&String>, Vec<&String>)>();
        context.insert("series.titles", &titles);
        context.insert("series.links", &links);
    }

    // template it
    let rendered = tera.render(&page_meta.template.unwrap_or_default(), &context)?;
    Ok(rendered)
}

struct Code {
    pub language: String,
    pub code: String,
}

pub fn parser_to_writer<W>(writer: W, parser: Parser) -> Result<()>
where
    W: std::fmt::Write,
{
    let mut code = None;

    let iter = parser.map(|event| {
        match &event {
            Event::Start(start) => match start {
                Tag::CodeBlock(CodeBlockKind::Fenced(lang)) => {
                    code = Some(Code {
                        language: lang.to_string(),
                        code: "".to_string(),
                    });
                }
                _ => {}
            },
            Event::End(end) => match end {
                Tag::CodeBlock(CodeBlockKind::Fenced(_)) => {
                    if let Some(code) = code.take() {
                        let mut out = String::new();
                        //
                        write!(out, r#"<pre>"#).ok();

                        if code.language != "" {
                            write!(out, r#"<div class="lang-tag">{}</div>"#, code.language).ok();
                        }
                        write!(out, r#"<div class="code-block"><code>"#).ok();

                        if let Err(why) =
                            parse_highlight_write_code(&mut out, &code.code, Some(&code.language))
                        {
                            warn!(why);
                            escape_to_writer(&mut out, &code.code).ok();
                        }
                        write!(&mut out, "</div></code></pre>").ok();
                        return Event::Html(out.into());
                    }
                }
                _ => {}
            },
            Event::Text(txt) => {
                if let Some(mut code) = code {
                    code.code.push_str(txt);
                }
            }
            _ => {}
        }
        event
    });

    html::write_html(writer, iter)?;
    Ok(())
}

pub fn parse_highlight_write_code<W>(writer: &mut W, source: &str, lang: Option<&str>) -> Result<()>
where
    W: std::fmt::Write,
{
    let mut highlighter = Highlighter::new();
    let config = match lang {
        None => return Err(Report::msg("Lang cannot be None")),
        Some(code) => match config_by_language_name(code) {
            None => return Err(Report::msg("unknown lang")),
            Some(cfg) => cfg,
        },
    };
    let highlights = highlighter.highlight(config, source.as_ref(), None, |cb| {
        config_by_language_name(cb)
    })?;

    for highlight in highlights {
        let highlight = highlight.unwrap();
        match highlight {
            HighlightEvent::Source { start, end } => {
                escape_to_writer(writer, &source[start..end]).unwrap()
            }
            HighlightEvent::HighlightStart(start) => {
                write!(writer, r#"<i class=chl-{}>"#, start.0).unwrap();
            }
            HighlightEvent::HighlightEnd => {
                write!(writer, r#"</i>"#).unwrap();
            }
        }
    }

    Ok(())
}

pub fn escape_to_writer<W>(writer: &mut W, code: &str) -> Result<()>
where
    W: std::fmt::Write,
{
    html_escape::encode_safe_to_writer(code, writer).into()
}

pub fn config_by_language_name(lang: &str) -> Option<&HighlightConfiguration> {
    const HIGHLIGHT_NAMES: &[&str] = &[
        "attribute",
        "constant",
        "function.builtin",
        "function",
        "keyword",
        "operator",
        "property",
        "punctuation",
        "punctuation.bracket",
        "punctuation.delimiter",
        "string",
        "string.special",
        "tag",
        "type",
        "type.builtin",
        "variable",
        "variable.builtin",
        "variable.parameter",
    ];

    static LANGUAGES: Lazy<HashMap<&'static str, HighlightConfiguration>> = Lazy::new(|| {
        let mut hashmap = HashMap::new();

        let mut c_lang = HighlightConfiguration::new(
            tree_sitter_c::language(),
            tree_sitter_c::HIGHLIGHT_QUERY,
            "",
            "",
        )
        .unwrap();
        c_lang.configure(HIGHLIGHT_NAMES);
        let mut r_lang =
            HighlightConfiguration::new(tree_sitter_r::language(), "", "", "").unwrap();
        r_lang.configure(HIGHLIGHT_NAMES);
        let mut go_lang = HighlightConfiguration::new(
            tree_sitter_go::language(),
            tree_sitter_go::HIGHLIGHT_QUERY,
            "",
            "",
        )
        .unwrap();
        go_lang.configure(HIGHLIGHT_NAMES);
        let mut cpp_lang = HighlightConfiguration::new(
            tree_sitter_cpp::language(),
            tree_sitter_cpp::HIGHLIGHT_QUERY,
            "",
            "",
        )
        .unwrap();
        cpp_lang.configure(HIGHLIGHT_NAMES);
        let mut lua_lang =
            HighlightConfiguration::new(tree_sitter_lua::language(), "", "", "").unwrap();
        lua_lang.configure(HIGHLIGHT_NAMES);
        let mut typescript_lang = HighlightConfiguration::new(
            tree_sitter_typescript::language_typescript(),
            tree_sitter_typescript::HIGHLIGHT_QUERY,
            "",
            tree_sitter_typescript::LOCALS_QUERY,
        )
        .unwrap();
        typescript_lang.configure(HIGHLIGHT_NAMES);
        let mut tsx_lang = HighlightConfiguration::new(
            tree_sitter_typescript::language_tsx(),
            tree_sitter_typescript::HIGHLIGHT_QUERY,
            "",
            tree_sitter_typescript::LOCALS_QUERY,
        )
        .unwrap();
        tsx_lang.configure(HIGHLIGHT_NAMES);
        let mut js_lang = HighlightConfiguration::new(
            tree_sitter_javascript::language(),
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::INJECTION_QUERY,
            tree_sitter_javascript::LOCALS_QUERY,
        )
        .unwrap();
        js_lang.configure(HIGHLIGHT_NAMES);
        let mut jsx_lang = HighlightConfiguration::new(
            tree_sitter_javascript::language(),
            tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
            tree_sitter_javascript::INJECTION_QUERY,
            tree_sitter_javascript::LOCALS_QUERY,
        )
        .unwrap();
        jsx_lang.configure(HIGHLIGHT_NAMES);
        let mut java_lang = HighlightConfiguration::new(
            tree_sitter_java::language(),
            tree_sitter_java::HIGHLIGHT_QUERY,
            "",
            "",
        )
        .unwrap();
        java_lang.configure(HIGHLIGHT_NAMES);
        let mut css_lang = HighlightConfiguration::new(
            tree_sitter_css::language(),
            tree_sitter_css::HIGHLIGHTS_QUERY,
            "",
            "",
        )
        .unwrap();
        css_lang.configure(HIGHLIGHT_NAMES);
        let mut html_lang = HighlightConfiguration::new(
            tree_sitter_html::language(),
            tree_sitter_html::HIGHLIGHT_QUERY,
            tree_sitter_html::INJECTION_QUERY,
            "",
        )
        .unwrap();
        html_lang.configure(HIGHLIGHT_NAMES);
        let mut toml_lang = HighlightConfiguration::new(
            tree_sitter_toml::language(),
            tree_sitter_toml::HIGHLIGHT_QUERY,
            "",
            "",
        )
        .unwrap();
        toml_lang.configure(HIGHLIGHT_NAMES);
        let mut rust_lang = HighlightConfiguration::new(
            tree_sitter_rust::language(),
            tree_sitter_rust::HIGHLIGHT_QUERY,
            "",
            "",
        )
        .unwrap();
        rust_lang.configure(HIGHLIGHT_NAMES);
        let mut json_lang = HighlightConfiguration::new(
            tree_sitter_json::language(),
            tree_sitter_json::HIGHLIGHT_QUERY,
            "",
            "",
        )
        .unwrap();
        json_lang.configure(HIGHLIGHT_NAMES);
        let mut kotlin_lang =
            HighlightConfiguration::new(tree_sitter_kotlin::language(), "", "", "").unwrap();
        kotlin_lang.configure(HIGHLIGHT_NAMES);
        let mut swift_lang = HighlightConfiguration::new(
            tree_sitter_swift::language(),
            tree_sitter_swift::HIGHLIGHTS_QUERY,
            "",
            tree_sitter_swift::LOCALS_QUERY,
        )
        .unwrap();
        swift_lang.configure(HIGHLIGHT_NAMES);
        let mut vue_lang = HighlightConfiguration::new(
            tree_sitter_vue::language(),
            tree_sitter_vue::HIGHLIGHTS_QUERY,
            tree_sitter_vue::INJECTIONS_QUERY,
            "",
        )
        .unwrap();
        vue_lang.configure(HIGHLIGHT_NAMES);
        let mut vue3_lang = HighlightConfiguration::new(
            tree_sitter_vue3::language(),
            tree_sitter_vue3::HIGHLIGHTS_QUERY,
            tree_sitter_vue3::INJECTIONS_QUERY,
            "",
        )
        .unwrap();
        vue3_lang.configure(HIGHLIGHT_NAMES);
        let mut svelte_lang = HighlightConfiguration::new(
            tree_sitter_svelte::language(),
            tree_sitter_svelte::HIGHLIGHT_QUERY,
            tree_sitter_svelte::INJECTION_QUERY,
            tree_sitter_svelte::TAGGING_QUERY,
        )
        .unwrap();
        svelte_lang.configure(HIGHLIGHT_NAMES);
        let mut csharp_lang = HighlightConfiguration::new(
            tree_sitter_c_sharp::language(),
            tree_sitter_c_sharp::HIGHLIGHT_QUERY,
            "",
            "",
        )
        .unwrap();
        csharp_lang.configure(HIGHLIGHT_NAMES);
        let mut python_lang = HighlightConfiguration::new(
            tree_sitter_python::language(),
            tree_sitter_python::HIGHLIGHT_QUERY,
            "",
            "",
        )
        .unwrap();
        python_lang.configure(HIGHLIGHT_NAMES);
        let mut openscad_lang =
            HighlightConfiguration::new(tree_sitter_openscad::language(), "", "", "").unwrap();
        openscad_lang.configure(HIGHLIGHT_NAMES);
        let mut elisp_lang =
            HighlightConfiguration::new(tree_sitter_elisp::language(), "", "", "").unwrap();
        elisp_lang.configure(HIGHLIGHT_NAMES);
        let mut ruby_lang = HighlightConfiguration::new(
            tree_sitter_ruby::language(),
            tree_sitter_ruby::HIGHLIGHT_QUERY,
            "",
            tree_sitter_ruby::LOCALS_QUERY,
        )
        .unwrap();
        ruby_lang.configure(HIGHLIGHT_NAMES);

        hashmap.insert("c", c_lang);
        hashmap.insert("r", r_lang);
        hashmap.insert("go", go_lang);
        hashmap.insert("cpp", cpp_lang);
        hashmap.insert("lua", lua_lang);
        hashmap.insert("ts", typescript_lang);
        hashmap.insert("tsx", tsx_lang);
        hashmap.insert("js", js_lang);
        hashmap.insert("jsx", jsx_lang);
        hashmap.insert("java", java_lang);
        hashmap.insert("css", css_lang);
        hashmap.insert("html", html_lang);
        hashmap.insert("toml", toml_lang);
        hashmap.insert("rust", rust_lang);
        hashmap.insert("json", json_lang);
        hashmap.insert("kt", kotlin_lang);
        hashmap.insert("swift", swift_lang);
        hashmap.insert("vue", vue_lang);
        hashmap.insert("svelte", svelte_lang);
        hashmap.insert("vue3", vue3_lang);
        hashmap.insert("cs", csharp_lang);
        hashmap.insert("py", python_lang);
        hashmap.insert("scad", openscad_lang);
        hashmap.insert("el", elisp_lang);
        hashmap.insert("rb", ruby_lang);
        hashmap
    });

    let lang = lang.to_ascii_lowercase();
    match LANGUAGES.get(&lang) {
        Some(l) => Some(l),
        None => match lang.as_str() {
            "c_plus_plus" | "c++" => LANGUAGES.get("cpp"),
            "luau" | "luajit" => LANGUAGES.get("lua"),
            "typescript" => LANGUAGES.get("ts"),
            "javascript" => LANGUAGES.get("js"),
            "rust" => LANGUAGES.get("rs"),
            "kotlin" => LANGUAGES.get("kt"),
            "c#" => LANGUAGES.get("cs"),
            "python" | "python3" | "py3" => LANGUAGES.get("py"),
            "openscad" => LANGUAGES.get("scad"),
            "lisp" | "clojure" | "scheme" | "elisp" | "clj" => LANGUAGES.get("el"),
            "ruby" => LANGUAGES.get("rb"),
            _ => None,
        },
    }
}
