use crate::injest::{
    generate::{CategoryMeta, PageMeta, SeriesMeta, SiteMeta, SubCategoryMeta},
    path_relativizie_path,
    templates::SiteTheme,
};
use bidirectional_map::Bimap;
use color_eyre::{Report, Result};
use id_tree::InsertBehavior::{AsRoot, UnderNode};
use id_tree::{Node, Tree};
use ignore::WalkBuilder;
use itertools::Itertools;
use memmap2::MmapOptions;
use rhai::{Engine, EvalAltResult, Scope, AST};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::{collections::HashMap, path::Path, str::FromStr};
use tera::{Context, Filter, Function, Tera};
use tera::{Test, Value};
use tracing::log::{error, log, warn};

struct Empty {}

impl AsRef<[u8]> for Empty {
    fn as_ref(&self) -> &[u8] {
        const NOTHING: &[u8] = &[];
        NOTHING
    }
}

macro_rules! mmap_load {
    ($path:expr) => {{
        let a: Box<impl AsRef<[u8]>> = match unsafe { MmapOptions::new().map(path.as_path()) } {
            Ok(a) => Box::new(a),
            Err(_) => Box::new(Empty {}),
        };
        a
    }};
}

pub enum ConfigurationType {
    Category,
    SubCategory,
    Redirect,
    Series,
    Page,
    External,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ExternalType {
    InDir,
    Plugin { plugin: String, resource: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConfigMeta {
    pub external: Option<ExternalType>,
    #[serde(flatten)]
    pub page: PageMeta,
    pub category: Option<CategoryMeta>,
    pub subcategory: Option<SubCategoryMeta>,
    pub series: Option<SeriesMeta>,
    pub redirect: Option<String>,
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

fn shell(cmd: &str) -> Result<(i32, String, String), Box<EvalAltResult>> {
    if cmd == "" {
        return Err("Bad Command!".into());
    }
    let exec = cmd.split_once(" ");
    let mut command = match exec {
        None => Command::new(cmd),
        Some((c, a)) => Command::new(c).arg(a),
    };
    let out = match command.output() {
        Ok(out) => out,
        Err(why) => {
            return Err(why.to_string().into());
        }
    };
    let out_stdout = String::from_utf8(out.stdout).unwrap_or_default();
    let out_stderr = String::from_utf8(out.stderr).unwrap_or_default();
    let out_code = match out.status.code() {
        Some(c) => c,
        None => i32::MIN_VALUE,
    };
    Ok((out_code, out_stdout, out_stderr))
}

fn log(out: &str) {
    log!(out)
}

fn warn(out: &str) {
    warn!(out)
}

fn error(out: &str) {
    error!(out)
}

const IGNORES: &'static [&str] = &["build.rhai"];

macro_rules! walker {
    ($path:expr) => {
        WalkBuilder::new($path).add_custom_ignore_filename(".mkignore")
    };
}

fn file_name_from_path(path: impl AsRef<Path>) -> Option<&str> {
    match path.as_ref().file_name() {
        Some(file) => match file.to_str() {
            Some(f) => Some(f),
            None => None,
        },
        None => None,
    }
}

#[derive(Copy, Clone, Debug, PartialOrd, PartialEq)]
pub enum LeafPathType {
    Moklog,
    Page,
}

pub struct LeafPath<T>
where
    T: AsRef<[u8]>,
{
    data: Box<T>,
    typ: LeafPathType,
    true_path: PathBuf,
}

pub struct FilePath {
    path: PathBuf,
    is_file: bool,
}

pub fn build_site(
    site_build_path: impl AsRef<Path>,
    site_output_path: impl AsRef<Path>,
    site_config: &SiteMeta,
    template: &SiteTheme,
) -> Result<()> {
    // run site build script
    let mut engine = Engine::new();
    engine.register_fn("shell", shell);
    engine.register_fn("log", log);
    engine.register_fn("warn", warn);
    engine.register_fn("error", error);
    let ast = match engine.compile_file(site_build_path.as_ref().with_file_name("build.rhai")) {
        Ok(ast) => ast,
        Err(why) => return Err(Report::msg(why.to_string())),
    };
    engine.run_ast(&ast);

    // traverse site build path
    let mut sitebuild_traveller = walker!(&site_build_path).build();
    let mut site_tree = Tree::new();
    let mut node_path_store = Bimap::new();
    let mut root_id = None;

    let mut fs_tree = Tree::new();
    let mut fs_root_id = None;

    for file in sitebuild_traveller {
        let file = path_relativizie_path(&site_build_path, file?.into_path())?;

        let mut spath = file.clone();
        let mut previous = spath.clone();

        let mut is_file = false;

        // check if previous exists
        let insert_behaviour = match node_path_store.get(&previous) {
            Some(node_id) => UnderNode(node_id),
            None => AsRoot,
        };

        if file.is_file() {
            spath.pop();
            previous.pop();
            previous.pop();

            is_file = true;

            let filename = match file.file_name() {
                Some(f) => match f.to_str() {
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

            if ["index.md", "index.html", ".moklog"].contains(&filename) {
                let filemap = mmap_load!(&file);
                let lpt = if filename == ".moklog" {
                    LeafPathType::Moklog
                } else {
                    LeafPathType::Page
                };
                let node = Node::new(LeafPath {
                    data: filemap,
                    typ: lpt,
                    true_path: file,
                });

                let id = site_tree.insert(node, insert_behaviour)?;
                if insert_behaviour == AsRoot {
                    root_id = Some(id.clone());
                }
                node_path_store.insert(spath, id);
                continue;
            }
        }

        let id = fs_tree.insert(
            Node::new(FilePath {
                path: file,
                is_file,
            }),
            insert_behaviour,
        )?;
        if insert_behaviour == AsRoot {
            fs_root_id = Some(id.clone());
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

    if let Some(fs_rid) = fs_root_id {
        for file_path in fs_tree.traverse_pre_order(&fs_rid)? {
            let fp = &file_path.data().path;
            // let filemap = mmap_load!(fp);
            // if let Some(file_extension) = fp.extension() {
            //     if let Some(file_extension) = file_extension.to_str() {}
            // TODO: file optimizations and plugin goodness
            let new_path = site_output_path.as_ref().to_path_buf() + fp;
            std::fs::rename(fp, new_path)?;
        }
    }

    if let Some(rid) = root_id {
        for endpoint_id in site_tree.traverse_post_order_ids(&rid)? {
            let endpoint_path = match node_path_store.get_rev(&endpoint_id) {
                Some(p) => p,
                None => continue, // TODO: error
            };

            let endpoint = site_tree.get(&endpoint_id).unwrap();
        }
    }

    Ok(())
}
