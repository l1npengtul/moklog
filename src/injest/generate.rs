use chrono::{Date, Utc};
use color_eyre::{Report, Result};
use once_cell::sync::Lazy;
use pulldown_cmark::{html, CodeBlockKind, Event, Parser, Tag};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use bidirectional_map::Bimap;
use dashmap::DashMap;
use language_tags::LanguageTag;
use serde_json::Number;
use tantivy::HasLen;
use tera::Tera;
use tracing::log::warn;
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};
use tera::Context;
use toml::Value;
use crate::injest::build::BuildInformation;
use crate::injest::processor::{html_post_processor, ProcessedDocument};

// A root page (index.md) contains a PageMeta + some other Meta
// A translation page (ko.md, ja.md, es.md, etc etc) contains a some other Meta other than ArticleMeta

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageHeader {
    #[serde(flatten)]
    pub page: PageMeta,
    pub page_type: PageTypeMeta,
    pub custom: Custom,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PageTypeMeta {
    SeriesMeta(SeriesMeta),
    ArticleMeta(ArticleMeta),
    GenericMeta(GenericMeta),
    CategoryMeta(GenericMeta),
    None,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Custom {
    #[serde(flatten)]
    pub data: BTreeMap<String, Value>
}

fn toml_v_to_json_v(toml: Value) -> serde_json::Value {
    match toml {
        Value::String(n) => {
            serde_json::Value::String(n)
        }
        Value::Integer(n) => {
            serde_json::Value::Number(Number::from(n))
        }
        Value::Float(n) => {
            serde_json::Value::Number(Number::from(n))
        }
        Value::Boolean(n) => {
            serde_json::Value::Bool(n)
        }
        Value::Datetime(date) => {
            serde_json::Value::String(            date.to_string())
        }
        Value::Array(a) => {
            serde_json::Value::Array(
                a.into_iter().map(
                    toml_v_to_json_v
                ).collect::<Vec<serde_json::Value>>())
        }
        Value::Table(t) => {
            serde_json::Value::Object(
                t.into_iter().map(|(k, v)| (k, toml_v_to_json_v(v))).collect::<serde_json::Map<String, serde_json::Value>>()
            )
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageMeta {
    pub group: Option<String>,
    pub translations: BTreeSet<String>,
    pub rss: bool,
    pub index: bool,
    pub redirect_from: Vec<String>,
    pub redirect_to: Option<String>,
    pub display: String,
    pub children_template: Option<String>,
    pub template: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GenericMeta {
    pub date: Date<Utc>,
    pub title: String,
    pub authors: Vec<String>,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CategoryMeta {
    pub title: String,
    pub pinned_posts: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeriesMeta {
    pub on_going: bool,
    pub date_started: Date<Utc>,
    pub date_completed: Option<Date<Utc>>,
    pub edited_dates: Vec<Date<Utc>>,
    pub title: String,
    pub authors: Vec<String>,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArticleMeta {
    pub title: String,
    pub tags: Vec<String>,
    pub authors: Vec<String>,
    pub date: Date<Utc>,
    pub edited_dates: Vec<Date<Utc>>,
    pub summary: Option<String>,
}

// all of this expects a pre-propagated config!
// page type is exported into the template under "content"

fn populate_page_meta(context: &mut Context, page: &PageMeta) {
    context.insert("page.group", &page.group.unwrap_or("default".into()));
    context.insert("page.translations", &page.translations);
    context.insert("page.rss_enabled", &page.rss);
    context.insert("page.index_enabled", &page.index);
    context.insert("page.template", &page.template);
    context.insert("page.children_template", &page.children_template);
    context.insert("page.display", &page.display);
    context.insert("page.redirect_from", &page.redirect_from);
    context.insert("page.redirect_to", &page.redirect_to);
    context.insert("page.display", &page.display);
}

fn populate_counts(context: &mut Context, content: &str) {
    const READING_WPM: f64 = 150.0;

    let word_count = words_count::count(content);
    let reading_time_seconds = (word_count.words as f64 / READING_WPM).round() as u32;
    let table_of_contents = pulldown_cmark_toc::TableOfContents::new(content).to_cmark();

    context.insert("content.reading_time_seconds", &reading_time_seconds);
    context.insert("content.table_of_contents", &table_of_contents);
    context.insert("content.word_count", &word_count.words);
    context.insert("content.character_count", &word_count.characters);
    context.insert("content.cjk", &word_count.cjk);
    context.insert("content.whitespace", &word_count.whitespaces);
}

fn populate_autos(context: &mut Context, build_info: &BuildInformation) {
    // populate autogenerated data
    // TODO: moklog information (version, etc)
    context.insert("auto.build_time", &build_info.start_time);
    context.insert("auto.build_init", &build_info.initiated);
    context.insert("auto.build_id", &build_info.id);
}

struct CategoryThing<'a> {
    pub display: &'a str,
    pub link: &'a str,
    pub subcategories: &'a HashSet<String>
}

fn populate_categories_subcategories<'a>(context: &'a mut Context, categories: &'a Arc<HashMap<String, String>>, subcategories: &'a Arc<HashMap<String, HashSet<String>>>) {
    let thing = categories.iter().map(|(display, link)| {
        CategoryThing {
            display,
            link,
            subcategories: subcategories.get(link).unwrap(),
        }
    });
    context.insert("page.categories", &thing);
}

fn populate_translations(context: &mut Context, languages: &[&LanguageTag], this_lang: &LanguageTag, default_lang: &LanguageTag, path: &str) {
    context.insert("page.translations", languages.iter().filter(|x| x == this_lang).filter(|x| x == default_lang).map(|x| {
        (x.clone().clone(),)
    }).collect());

    context.insert("page.default_translation", &(default_lang, path));
    if this_lang == default_lang {
        context.insert("page.this_translation", &(this_lang, path));
    } else {
        context.insert("page.this_translation", &(this_lang,  format!("/{}{path}", this_lang.as_str())));
    }
}

fn populate_core_build_stuffs(context: &mut Context, core: CoreBuildStuffs) {
    populate_page_meta(context, core.page);
    populate_counts(context, core.content);
    context.insert("page.base_slug", core.slug);
    populate_autos(context, core.info);
    populate_categories_subcategories(context, &core.categories, &core.subcategories);
    populate_translations(context, core.langauges, core.language, core.default_language, core.path);
    tera_context.insert("content.raw", core.content);

    for (key, value) in core.custom.data.iter() {
        let ins_key = format!("custom.{}", key);
        context.insert(&ins_key, &value);
    }
}

pub struct CoreBuildStuffs<'a> {
    tera: &'a Tera,
    info: &'a BuildInformation,
    page: &'a PageMeta,
    slug: &'a str,
    files: Arc<DashMap<u64, PathBuf>>,
    categories: Arc<HashMap<String, String>>,
    subcategories: Arc<HashMap<String, HashSet<String>>>,
    language: &'a LanguageTag,
    default_language: &'a LanguageTag,
    langauges: &'a [&'a LanguageTag],
    content: &'a str,
    path: &'a str,
    custom: &'a Custom,
}

// TODO: PAM + Permission System
// Basically like discord: there are users, and there are roles, and those roles have permissions.

// TODO: backfill logic by recursively parent tree, then go forward down the backfills until a consistant thing forms
pub fn build() {}

pub fn build_generic(
    generic: &GenericMeta,
    build_stuffs: CoreBuildStuffs
) -> Result<ProcessedDocument> {
    let mut parser = Parser::new(content);
    let mut output = String::with_capacity(content.len());
    let mut tera_context = Context::new();

    populate_core_build_stuffs(&mut tera_context, build_stuffs);
    tera_context.insert("page.type", "generic");
    tera_context.insert("content.date", &generic.date);
    tera_context.insert("content.title", &generic.title);
    tera_context.insert("content.authors", &generic.authors);
    tera_context.insert("content.tags", &generic.tags);

    parser_to_writer(&mut output, parser)?;
    tera_context.insert("content", &output);

    // insert tera templates
    let mut rendered = String::with_capacity(output.len());
    build_stuffs.tera.render_to("generic.html", &tera_context, &mut rendered)?;

    // html stuffs

    Ok(html_post_processor(path, files.clone(), &rendered)?)
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
            "javascript" | "ecmascript" => LANGUAGES.get("js"),
            "rust" => LANGUAGES.get("rs"),
            "kotlin" => LANGUAGES.get("kt"),
            "c#" => LANGUAGES.get("cs"),
            "python" | "python3" | "py3" | "pyw" => LANGUAGES.get("py"),
            "openscad" => LANGUAGES.get("scad"),
            "lisp" | "clojure" | "scheme" | "elisp" | "clj" => LANGUAGES.get("el"),
            "ruby" => LANGUAGES.get("rb"),
            _ => None,
        },
    }
}
