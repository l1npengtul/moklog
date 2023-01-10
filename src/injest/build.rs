use crate::injest::{
    generate::{CategoryMeta, PageMeta, SeriesMeta, SiteMeta, SubCategoryMeta},
    path_relativizie,
    templates::SiteTheme,
};
use color_eyre::{Report, Result};
use ignore::{DirEntry, WalkBuilder};
use memmap2::MmapOptions;
use petgraph::{prelude::GraphMap, Directed};
use rhai::{Engine, Scope, AST};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    str::{from_utf8, FromStr},
};
use tera::{Context, Filter, Function, Tera};
use tera::{Test, Value};

struct Empty {}

impl AsRef<[u8]> for Empty {
    fn as_ref(&self) -> &[u8] {
        const NOTHING: &[u8] = &[];
        NOTHING
    }
}

pub enum ConfigurationType {
    Category(CategoryMeta),
    SubCategory(SubCategoryMeta),
    Redirect(RedirectMeta),
    Series(SeriesMeta),
    Page,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RedirectMeta {
    pub to: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConfigMeta {
    pub external: bool,
    #[serde(flatten)]
    pub page: PageMeta,
    pub category: Option<CategoryMeta>,
    pub subcategory: Option<SubCategoryMeta>,
    pub series: Option<SeriesMeta>,
    pub redirect: Option<RedirectMeta>,
}

struct RhaiFilter {
    engine: Engine,
    script: AST,
    times_exec: AtomicU64,
}

impl Filter for RhaiFilter {
    fn filter(&self, value: &Value, args: &HashMap<String, Value>) -> tera::Result<Value> {
        let mut scope = Scope::new();
        let exectimes = self.times_exec.load(Ordering::SeqCst);
        let result = self
            .engine
            .call_fn::<Value>(&mut scope, &self.script, "filter", (value, args, exectimes))
            .map_err(|why| Err(tera::Error::msg(why.to_string())))?;
        self.times_exec.fetch_add(1, Ordering::SeqCst);

        Ok(result)
    }
}

struct RhaiTester {
    engine: Engine,
    script: AST,
    times_exec: AtomicU64,
}

impl Test for RhaiTester {
    fn test(&self, value: Option<&Value>, args: &[Value]) -> tera::Result<bool> {
        let mut scope = Scope::new();
        let exectimes = self.times_exec.load(Ordering::SeqCst);
        let result = self
            .engine
            .call_fn::<Value>(&mut scope, &self.script, "test", (value, args, exectimes))
            .map_err(|why| Err(tera::Error::msg(why.to_string())))?;
        self.times_exec.fetch_add(1, Ordering::SeqCst);

        Ok(result)
    }
}

struct RhaiFunction {
    engine: Engine,
    script: AST,
    times_exec: AtomicU64,
}

impl Function for RhaiFunction {
    fn call(&self, args: &HashMap<String, Value>) -> tera::Result<Value> {
        let mut scope = Scope::new();
        let exectimes = self.times_exec.load(Ordering::SeqCst);
        let result = self
            .engine
            .call_fn::<Value>(&mut scope, &self.script, "main", (args, exectimes))
            .map_err(|why| Err(tera::Error::msg(why.to_string())))?;
        self.times_exec.fetch_add(1, Ordering::SeqCst);

        Ok(result)
    }
}

struct Shortcode {
    tera: RefCell<Tera>,
    times_exec: AtomicU64,
}

impl Function for Shortcode {
    fn call(&self, args: &HashMap<String, Value>) -> tera::Result<Value> {
        let mut ctx = Context::new();
        for (name, arg) in args {
            ctx.insert(name, arg)
        }
        ctx.insert("times", &self.times_exec.load(Ordering::SeqCst));
        let mut tera = self.tera.borrow_mut();
        let render = tera.render_str("shortcode", &ctx)?;
        self.times_exec.fetch_add(1, Ordering::SeqCst);
        Ok(Value::String(render))
    }
}

pub fn build_site(
    site_build_path: impl AsRef<Path>,
    site_output_path: impl AsRef<Path>,
    site_config: &SiteMeta,
    template: &SiteTheme,
) -> Result<()> {
    // traverse site build path
    let mut sitebuild_traveller = WalkBuilder::new(&site_build_path)
        .build()
        .collect::<Result<Vec<DirEntry>>>()?;
    sitebuild_traveller.sort_by(|a, b| a.depth().cmp(&b.depth()));

    let mut sc_backing = HashMap::new();
    let mut pages = HashMap::new();
    let mut site_children: GraphMap<&str, Directed, _> = GraphMap::new();

    for file in sitebuild_traveller {
        let mut path_str = path_relativizie(&site_build_path, file)?;
        let mut path = PathBuf::from_str(&path_str)?;

        let filename = match path.file_name() {
            Some(file) => match file.to_str() {
                Some(f) => f,
                None => return Err(Report::msg("non utf8 filename")),
            },
            None => {
                if let Some(end) = path.into_iter().last() {
                    match end.to_str() {
                        Some(end) => {
                            if !end.chars().next().unwrap().is_alphabetic() {
                                return Err(Report::msg(
                                    "folder cannot start with non-ascii-alphanumeric character!",
                                ));
                            }
                            continue;
                        }
                        None => return Err(Report::msg("non utf8 filename")),
                    }
                }
            }
        };

        let filemap: Box<impl AsRef<[u8]>> = match unsafe { MmapOptions::new().map(path.as_path()) }
        {
            Ok(a) => Box::new(a),
            Err(_) => Box::new(Empty {}),
        };

        path.pop();
        if filename == "index.md" || filename == "index.html" {
            let (cfg, content) = match from_utf8(&filemap)?.split_once("===") {
                Some((cfg, cot)) => (cfg, Some(cot)),
                None => (from_utf8(&filemap)?, None),
            };

            let meta = toml::from_str::<ConfigMeta>(cfg)?;
            pages.insert(&path_str, (meta.page, content));

            let mut previous = "";
            for p in path.iter() {
                let nodestr = p.to_str().unwrap();
                if !site_children.contains_node(nodestr) {
                    site_children.add_node(nodestr);
                    if previous != "" {
                        site_children.add_edge(previous, nodestr, Directed {});
                    }
                }
                previous = nodestr;
            }

            match (meta.series, meta.subcategory, meta.category, meta.redirect) {
                (Some(series), None, None, None) => {
                    sc_backing.insert(&path_str, ConfigurationType::Series(series));
                }
                (None, Some(subcat), None, None) => {
                    sc_backing.insert(&path_str, ConfigurationType::SubCategory(subcat));
                }
                (None, None, Some(cat), None) => {
                    sc_backing.insert(&path_str, ConfigurationType::Category(cat));
                }
                (None, None, None, Some(redirect)) => {
                    sc_backing.insert(&path_str, ConfigurationType::Redirect(redirect));
                }
                (None, None, None, None, None) => {
                    sc_backing.insert(&path_str, ConfigurationType::Page)
                }
                _ => Report::msg("Bad configuration file"),
            }
        } else if filename == ".gumilgi" {
            let meta = toml::from_slice::<ConfigMeta>(&filemap)?;
        }
    }

    // start actual sitebuild

    let mut tera = Tera::default();
    tera.add_raw_templates(template.tera_templates.iter())?;

    for filter in template.filters.iter() {
        let engine = Engine::new();
        let script = engine.compile(filter.value())?;
        tera.register_filter(
            filter.key(),
            RhaiFilter {
                engine,
                script,
                times_exec: AtomicU64::new(0),
            },
        )
    }

    for test in template.testers.iter() {
        let engine = Engine::new();
        let script = engine.compile(test.value())?;
        tera.register_tester(
            test.key(),
            RhaiTester {
                engine,
                script,
                times_exec: AtomicU64::new(0),
            },
        )
    }

    for function in template.functions.iter() {
        let engine = Engine::new();
        let script = engine.compile(function.value())?;
        tera.register_function(
            function.key(),
            RhaiFunction {
                engine,
                script,
                times_exec: AtomicU64::new(0),
            },
        )
    }

    for shortcode in template.shortcode.iter() {
        let mut tera = Tera::default();
        tera.add_raw_template("shortcode", shortcode.value())?;
        tera.register_function(
            shortcode.key(),
            Shortcode {
                tera: RefCell::new(tera),
                times_exec: AtomicU64::new(0),
            },
        )
    }

    for endpoint in WalkBuilder::new(&site_build_path).build() {
        let endpoint = endpoint?;
    }

    Ok(())
}
