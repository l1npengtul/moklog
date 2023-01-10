use crate::injest::{
    path_relativizie,
    static_file::{new_filename, StaticFile},
    stylesheet::{compile_sass, optimize_css},
};
use color_eyre::Result;
use dashmap::DashMap;
use ignore::WalkBuilder;
use memmap2::Mmap;
use minify_js::TopLevelMode;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::str::pattern::Pattern;
use std::sync::Arc;
use tera::Tera;
use tokio::{fs::File, io::AsyncReadExt};

pub struct SiteTheme {
    pub metadata: SiteThemeMetadata,
    pub tera_templates: Arc<DashMap<String, String>>,
    pub shortcode: Arc<DashMap<String, String>>,
    pub functions: Arc<DashMap<String, String>>,
    pub filters: Arc<DashMap<String, String>>,
    pub testers: Arc<DashMap<String, String>>,
    pub styles: Arc<DashMap<String, String>>,
    pub js_scripts: Arc<DashMap<String, String>>,
    pub files: Arc<DashMap<String, StaticFile>>,
}

impl From<SerializeSiteTheme> for SiteTheme {
    fn from(sst: SerializeSiteTheme) -> Self {
        SiteTheme {
            metadata: sst.metadata,
            tera_templates: Arc::new(sst.templates.into_iter().collect()),
            shortcode: Arc::new(sst.shortcode.into_iter().collect()),
            functions: Arc::new(sst.functions.into_iter().collect()),
            filters: Arc::new(sst.filters.into_iter().collect()),
            testers: Arc::new(sst.testers.into_iter().collect()),
            styles: Arc::new(sst.styles.into_iter().collect()),
            js_scripts: Arc::new(sst.js_scripts.into_iter().collect()),
            files: Arc::new(sst.files.into_iter().collect()),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct SerializeSiteTheme {
    pub metadata: SiteThemeMetadata,
    pub templates: BTreeMap<String, String>,
    pub shortcode: BTreeMap<String, String>,
    pub functions: BTreeMap<String, String>,
    pub filters: BTreeMap<String, String>,
    pub testers: BTreeMap<String, String>,
    pub styles: BTreeMap<String, String>,
    pub js_scripts: BTreeMap<String, String>,
    pub files: BTreeMap<String, StaticFile>,
}

impl From<SiteTheme> for SerializeSiteTheme {
    fn from(st: SiteTheme) -> Self {
        SerializeSiteTheme {
            metadata: st.metadata,
            templates: st.tera_templates.into_iter().collect(),
            shortcode: st.shortcode.into_iter().collect(),
            functions: st.functions.into_iter().collect(),
            filters: st.filters.into_iter().collect(),
            styles: st.styles.into_iter().collect(),
            js_scripts: st.js_scripts.into_iter().collect(),
            files: st.files.into_iter().collect(),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct SiteThemeMetadata {
    pub authors: Vec<String>,
    pub name: String,
    pub link: String,
    pub version: Version,
}

pub async fn build_site_theme(template_dir: impl AsRef<str>) -> Result<SiteTheme> {
    macro_rules! template_dir {
        ($path:expr) => {
            format!("{template_dir}/{}", $path)
        };
    }
    macro_rules! walker {
        ($dir:expr, $path:expr) => {{
            WalkBuilder::new(template_dir!($dir, $path))
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

    // load shortcodes

    let mut shortcode = DashMap::new();
    // verify in tera
    {
        let mut tera = Tera::new(&template_dir!("shortcodes"))?;
        tera.add_template_files(template_files.into_iter())?;
    }
    for shrtcde in walker!(template_dir, "shortcodes") {
        let shrtcde = shrtcde?;
        let file_name =
            path_relativizie(template_dir!(template_dir, "shortcodes"), shrtcde.path())?;
        let mut short_code = String::new();
        File::open(shrtcde.path())
            .await?
            .read_to_string(&mut short_code)
            .await?;
        shortcode.insert(file_name, short_code);
    }

    // add tera templates

    let mut template_files = vec![];
    for template_entry in walker!(template_dir, "templates") {
        let template_entry = template_entry?;
        let file_extension = template_entry
            .path()
            .extension()
            .unwrap_or_default()
            .to_str()
            .unwrap_or_default();
        let file_name = path_relativizie(
            template_dir!(template_dir, "templates"),
            template_entry.path(),
        )?;
        if file_extension != "html" || file_extension != "tera" {
            continue;
        }
        template_files.push((template_entry.into_path(), Some(file_name)));
    }

    // compile scss, css

    let mut styles = DashMap::new();
    for style_entry in walker!(template_dir, "stylesheets") {
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

        let file_name = path_relativizie(
            template_dir!(template_dir, "stylesheets"),
            style_entry.path(),
        )?;

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
    for script_entry in walker!(template_dir, "scripts") {
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
        let file_name =
            path_relativizie(template_dir!(template_dir, "scripts"), script_entry.path())?;
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

    // load rhai functions

    let mut functions = DashMap::new();
    for func in walker!(template_dir, "functions") {
        let func = func?;
        if ft
            .path()
            .extension()
            .unwrap_or_default()
            .to_str()
            .unwrap_or_default()
            != "rhai"
        {
            continue;
        }
        let file_name = path_relativizie(template_dir!(template_dir, "shortcodes"), func.path())?;
        if file_name.ends_with(".rhai") {
            file_name.strip_suffix_of(".rhai")
        }
        let mut function = String::new();
        File::open(func.path())
            .await?
            .read_to_string(&mut function)
            .await?;
        functions.insert(file_name, function);
    }

    // load rhai filters

    let mut filters = DashMap::new();
    for ft in walker!(template_dir, "filters") {
        let ft = ft?;
        if ft
            .path()
            .extension()
            .unwrap_or_default()
            .to_str()
            .unwrap_or_default()
            != "rhai"
        {
            continue;
        }
        let file_name = path_relativizie("filters", ft.path())?;
        if file_name.ends_with(".rhai") {
            file_name.strip_suffix_of(".rhai")
        }
        let mut filter = String::new();
        File::open(ft.path())
            .await?
            .read_to_string(&mut filter)
            .await?;
        filters.insert(file_name, filter);
    }
    // load rhai testers

    let mut testers = DashMap::new();
    for ft in walker!(template_dir, "testers") {
        let ft = ft?;
        if ft
            .path()
            .extension()
            .unwrap_or_default()
            .to_str()
            .unwrap_or_default()
            != "rhai"
        {
            continue;
        }
        let mut file_name = path_relativizie("testers", ft.path())?;
        if file_name.ends_with(".rhai") {
            file_name.strip_suffix_of(".rhai")
        }
        let mut test = String::new();
        File::open(ft.path())
            .await?
            .read_to_string(&mut test)
            .await?;
        testers.insert(file_name, test);
    }

    // load static files

    let mut files = DashMap::new();
    for file in walker!(template_dir, "static") {
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
        tera_templates,
        shortcode: Arc::new(shortcode),
        functions: Arc::new(functions),
        filters: Arc::new(filters),
        metadata,
        styles: Arc::new(styles),
        js_scripts: Arc::new(js_scripts),
        files: Arc::new(files),
        testers: Arc::new(testers),
    })
}
