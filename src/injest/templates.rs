use crate::injest::{
    path_relativizie,
    static_file::{new_filename, StaticFile},
    stylesheet::{compile_sass, optimize_css},
};
use color_eyre::{Report, Result};
use dashmap::DashMap;
use ignore::WalkBuilder;
use memmap2::Mmap;
use minify_js::TopLevelMode;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use syntect::highlighting::ThemeSet;
use tera::Tera;
use tokio::{fs::File, io::AsyncReadExt};

pub struct SiteTheme {
    pub metadata: SiteThemeMetadata,
    pub syntect_colors: ThemeSet,
    pub tera_templates: Tera,
    pub shortcode: Arc<DashMap<String, String>>,
    pub functions: Arc<DashMap<String, String>>,
    pub filters: Arc<DashMap<String, String>>,
    pub styles: Arc<DashMap<String, String>>,
    pub js_scripts: Arc<DashMap<String, String>>,
    pub files: Arc<DashMap<String, StaticFile>>,
}

impl TryFrom<SerializeSiteTheme> for SiteTheme {
    type Error = Report;

    fn try_from(sst: SerializeSiteTheme) -> std::result::Result<Self, Self::Error> {
        let mut tera = Tera::default();
        tera.add_raw_templates(sst.templates.into_iter())?;
        Ok(SiteTheme {
            metadata: sst.metadata,
            syntect_colors: sst.syntect_colors,
            tera_templates: tera,
            shortcode: sst.shortcode,
            functions: sst.functions,
            filters: sst.filters,
            styles: sst.styles,
            js_scripts: sst.js_scripts,
            files: sst.files,
        })
    }
}

impl From<SiteTheme> for SerializeSiteTheme {
    fn from(st: SiteTheme) -> Self {
        Self {
            metadata: st.metadata,
            syntect_colors: st.syntect_colors,
            templates: st.tera_templates.get_template_names().map(|name| {
                st.tera_templates.get_template(name).unwrap().
            }),
            shortcode: Arc::new(Default::default()),
            functions: Arc::new(Default::default()),
            filters: Arc::new(Default::default()),
            styles: Arc::new(Default::default()),
            js_scripts: Arc::new(Default::default()),
            files: Arc::new(Default::default()),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct SerializeSiteTheme {
    pub metadata: SiteThemeMetadata,
    pub syntect_colors: ThemeSet,
    pub templates: BTreeMap<String, String>,
    pub shortcode: Arc<DashMap<String, String>>,
    pub functions: Arc<DashMap<String, String>>,
    pub filters: Arc<DashMap<String, String>>,
    pub styles: Arc<DashMap<String, String>>,
    pub js_scripts: Arc<DashMap<String, String>>,
    pub files: Arc<DashMap<String, StaticFile>>,
}

#[derive(Serialize, Deserialize)]
pub struct SiteThemeMetadata {
    pub authors: Vec<String>,
    pub name: String,
    pub link: String,
    pub version: Version,
}

pub async fn build_site_theme(template_dir: impl AsRef<str>) -> Result<SiteTheme> {
    let template_dir = template_dir.as_ref();
    macro_rules! template_dir {
        ($path:expr) => {
            format!("{template_dir}/{}", $path)
        };
    }
    macro_rules! walker {
        ($path:expr) => {{
            WalkBuilder::new(template_dir!($path))
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
    syntect_colors.add_from_folder(format!("{template_dir}/highlighting"))?;

    // add tera templates

    let mut tera_templates = Tera::default();
    let mut template_files = vec![];
    for template_entry in walker!("templates") {
        let template_entry = template_entry?;
        let file_extension = template_entry
            .path()
            .extension()
            .unwrap_or_default()
            .to_str()
            .unwrap_or_default();
        let file_name = path_relativizie(template_dir!("templates"), template_entry.path())?;
        if file_extension != "html" || file_extension != "tera" {
            continue;
        }
        template_files.push((template_entry.into_path(), Some(file_name)));
    }
    tera_templates.add_template_files(template_files.into_iter())?;

    // compile scss, css

    let mut styles = DashMap::new();
    for style_entry in walker!("stylesheets") {
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

        let file_name = path_relativizie(template_dir!("stylesheets"), style_entry.path())?;

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

    let mut js_scripts = DashMap::new();
    for script_entry in walker!("scripts") {
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
        let file_name = path_relativizie(template_dir!("scripts"), script_entry.path())?;
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

    let mut shortcode = DashMap::new();
    // verify in tera
    let _ = Tera::new(&template_dir!("shortcodes"))?;
    for shrtcde in walker!("shortcodes") {
        let shrtcde = shrtcde?;
        let file_name = path_relativizie(template_dir!("shortcodes"), shrtcde.path())?;
        let mut short_code = String::new();
        File::open(shrtcde.path())
            .await?
            .read_to_string(&mut short_code)
            .await?;
        shortcode.insert(file_name, short_code);
    }

    // load rhai functions

    let mut functions = DashMap::new();
    for func in walker!("functions") {
        let func = func?;
        let file_name = path_relativizie(template_dir!("shortcodes"), func.path())?;
        let mut function = String::new();
        File::open(func.path())
            .await?
            .read_to_string(&mut function)
            .await?;
        functions.insert(file_name, function);
    }

    // load rhai filters

    let mut filters = DashMap::new();
    for ft in walker!("filters") {
        let ft = ft?;
        let file_name = path_relativizie("filters", ft.path())?;
        let mut filter = String::new();
        File::open(ft.path())
            .await?
            .read_to_string(&mut filter)
            .await?;
        functions.insert(file_name, filter);
    }

    // load static files

    let mut files = DashMap::new();
    for file in walker!("static") {
        let file = file?;
        if file.metadata()?.len() != 0 {
            let data = unsafe { Mmap::map(file.path())? };
            let mut filename = file.into_path();
            let last = filename.file_name().unwrap().to_str().unwrap_or_default();
            let (hash, newfname) = new_filename(data.as_ref(), last);
            let filename = filename.with_file_name(newfname);
            let new_filename = path_relativizie(file, filename)?;
            files.insert(
                new_filename.clone(),
                StaticFile {
                    file_hash: hash,
                    file_name: new_filename,
                    path: file.into_path().to_str().unwrap_or_default().to_string(),
                },
            )
        }
    }

    Ok(SiteTheme {
        syntect_colors,
        tera_templates,
        shortcode: Arc::new(shortcode),
        functions: Arc::new(functions),
        filters: Arc::new(filters),
        metadata,
        styles: Arc::new(styles),
        js_scripts: Arc::new(js_scripts),
        files: Arc::new(files),
    })
}
