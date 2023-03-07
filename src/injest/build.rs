use crate::injest::{
    path_relativizie_path,
    templates::SiteTheme,
};
use bidirectional_map::Bimap;
use color_eyre::{Report, Result};
use id_tree::{InsertBehavior, Node, RemoveBehavior, Tree};
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
use std::collections::HashSet;
use std::str::from_utf8;
use axum::body::HttpBody;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use language_tags::LanguageTag;
use tera::{Context, Filter, Function, Tera};
use tera::{Test, Value};
use tracing::log::{error, log, warn};
use crate::injest::static_file::{process_static_file};
use crate::{mmap_load, walker};

#[derive(Clone, Debug, PartialOrd, PartialEq, Serialize, Deserialize)]
pub struct BuildInformation {
    pub initiated: String,
    pub id: u64,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub status: BuildStatus,
}

#[derive(Copy, Clone, Debug, Ord, PartialOrd, Eq, PartialEq, Serialize, Deserialize)]
pub enum BuildStatus {
    Running,
    Succeeded,
    Failed,
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
    PreBuilt,
}

pub struct LeafPath<T> where T: AsRef<[u8]> {
    file_name: String,
    depth: usize,
    data: Option<LeafPathData<T>>
}

impl<T> LeafPath<T> where T: AsRef<[u8]> {
    pub fn set_data(&mut self, data: LeafPathData<T>) {
        self.data = Some(data)
    }

    pub fn data(&self) -> &Option<LeafPathData<T>> {
        &self.data
    }

    pub fn data_mut(&mut self) -> &mut Option<LeafPathData<T>> {
        &mut self.data
    }
}

pub struct LeafPathData<T>
where
    T: AsRef<[u8]>,
{
    data: Box<T>,
    typ: LeafPathType,
    true_path: PathBuf,
    translations: HashMap<LanguageTag, TranslateLeaf<T>>
}

pub struct TranslateLeaf<T> where T: AsRef<[u8]> {
    data: Box<T>,
    typ: LeafPathType,
    true_path: PathBuf,
}

pub struct FilePath {
    path: PathBuf,
    is_file: bool,
}

const RESERVED_NAMES: &[&str] = &["template", "files", "static", "admin", "user", "me", "api", "stat", "error", "feed"];

const RESERVED_CHARS: &[char] = &[
    '{' , '}' , '|' , '\\' , '^' ,'[' , ']' , '`',
    ';' , '/' , '?' , ':' , '@' , '&' , '=' , '+' , '$' , ',',
    ' ', '<' , '>' , '#' , '%' , '"', '\''
];

const SPLITTER: &str = "===";

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
    let mut sitebuild_traveller = walker!(site_build_path.as_ref()).filter_entry(|dir| {
        dir.file_name().to_str().map(|f| {
            RESERVED_NAMES.contains(&f)
        }).unwrap_or(false)
    });

    let mut site_tree = Tree::new();
    let mut node_path_store = Bimap::new();
    let mut root_id = None;

    let mut fs_tree: Tree<LeafPath<[u8]>> = Tree::new();
    let mut fs_path_store = Bimap::new();
    let mut fs_root_id = None;

    let mut files = DashMap::new();

    for (hash, file) in template.files.iter().map(|x| (*x.key(), x.value().clone())) {
        files.insert(hash, path_relativizie_path(&site_build_path, file.path));
    }


    for file in sitebuild_traveller.build() {
        let depth = file?.depth();
        let file = path_relativizie_path(&site_build_path, file?.into_path())?;

        // check if previous exists
        let insert_behaviour = match node_path_store.get(&previous) {
            Some(node_id) => InsertBehavior::UnderNode(node_id),
            None => {
                if root_id.is_none() {
                    InsertBehavior::AsRoot
                } else {
                    warn!("Orphaned Item Detected!");
                    continue;
                }
            },
        };

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

        let file_extension = match file.extension().map(|x| x.to_str()).flatten() {
            Some(ext) => ext,
            None => return Err(Report::msg("non utf8 filename")),
        };

        let file_nonext = match file.file_prefix().map(|x| x.to_str()).flatten() {
            Some(ext) => ext,
            None => return Err(Report::msg("non utf8 filename")),
        };

        if file.is_file() {
            let parent = match file.parent().map(|path | fs_path_store.get_rev(path)).flatten() {
                Some(p) => p,
                None => return Err(Report::msg("no parent path!")),
            };

            let path_type = match file_extension {
                "md" => LeafPathType::Page,
                "html" => LeafPathType::PreBuilt,
                "moklog" => LeafPathType::Moklog,
                _ => continue,
            };

            let filemap: Box<[u8]>  = mmap_load!(&file);

            if ["index.md", "index.html", ".moklog"].contains(&filename) {
                let parent_node = fs_tree.get_mut(parent)?;

                let data = parent_node.data_mut();
                data.data = Some(
                    LeafPathData {
                        data: filemap,
                        typ: path_type,
                        true_path: file,
                        translations: Default::default(),
                    }
                );
            } else if file_extension == "md" || file_extension == "html" || file_extension == "moklog" {
                if let Ok(lang_tag) = LanguageTag::parse(file_nonext) {
                    // get parent
                    let parent_node = fs_tree.get_mut(parent)?;

                    let data = parent_node.data_mut();
                    if let Some(lpd) = data.data_mut() {

                        lpd.translations.insert(lang_tag, TranslateLeaf {
                            data: filemap,
                            typ: path_type,
                            true_path: file,
                        });
                    } else {
                        warn!("orphan file!");
                    }
                } else {
                    warn!("orphan file!");
                }
            } else {
                match process_static_file(file) {
                    Some(file) => {
                        files.insert(file.0, file.1);
                    }
                    None => {
                        warn!("failed to hash file!")
                    }
                }
            }
        } else {
            if let Ok(_) = LanguageTag::parse(filename) {
                return Err(Report::msg("folder cannot be a language tag!"));
            }

            if RESERVED_NAMES.contains(&filename) || filename.contains(RESERVED_CHARS) {
                return Err(Report::msg("folder reserved word/invalid char!"));
            }

            let leaf_path = LeafPath { file_name: filename.to_string(), depth, data: None };
            let node = fs_tree.insert(Node::new(
                leaf_path
            ), insert_behaviour)?;
            if insert_behaviour == InsertBehavior::AsRoot {
                fs_root_id = Some(node.clone());
            }

            fs_path_store.insert(node, file);
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

    let mut categories = HashMap::new();
    let mut category_subcat_map = HashMap::new();
    let mut sub_categories = HashMap::new();


    if let Some(fs_rid) = fs_root_id {
        loop {
            let mut bad_paths = vec![];
            for file_id in fs_tree.traverse_post_order_ids(&fs_rid)? {
                let data = fs_tree.get(&file_id).unwrap();
                if data.data().data.is_none() {
                    bad_paths.push(file_id)
                }
            }
            if bad_paths.len() == 0 {
                break
            }
            for bad in bad_paths {
                if bad != fs_rid {
                    let _err = fs_tree.remove_node(bad, RemoveBehavior::DropChildren);
                }
            }
        }

        for possible_category in sitebuild_traveller.max_depth(Some(2)).build() {
            let possible_category = possible_category?;
            let path = possible_category.path();

            if path.is_dir() {
                let path_data_id = match fs_path_store.get_rev(path) {
                    Some(d) => d,
                    None => continue,
                };

                let path_data = fs_tree.get(path_data_id).unwrap();

                // parse front matter

                match &path_data.data().data {
                    Some(data) => {
                        let (cfg, _) = match from_utf8(&data.data)?.split_once(SPLITTER) {
                            Some(v) => v,
                            None => continue,
                        };

                        let config = toml::from_str::<ConfigMeta>(cfg)?;

                        if let Some(cat_cfg) = config.category {
                            let this_dir = match path.file_prefix().map(|x| x.to_str()).flatten() {
                                Some(pre) => pre,
                                None => continue,
                            };
                            {
                                if possible_category.depth() == 1 {
                                    categories.insert(this_dir.to_string(), cat_cfg);
                                    category_subcat_map.insert(this_dir.to_string(), HashSet::new());
                                } else  {
                                    let parent = match path.parent().unwrap().file_prefix().map(|x| x.to_str()).flatten() {
                                        Some(pre) => pre,
                                        None => continue,
                                    };

                                    if site_categories.contains_key(&parnet) {
                                        category_subcat_map.get_mut(&parent).unwrap().insert(this_dir.to_string());
                                        sub_categories.insert(this_dir.to_string(), cat_cfg);
                                    } else {
                                        warn!("parent not in!");
                                    }
                                }
                            }
                        }
                    }
                    None => continue,
                }
            }
        }
    }

    for fs_node_id in fs_tree.traverse_level_order_ids(&fs_root_id.unwrap())? {
        let fs_node = fs_tree.get(&fs_node_id).unwrap();

        if fs_node_id == fs_root_id.unwrap() {
            let insert_behaviour = InsertBehavior::AsRoot;

            // let materials =
        } else if fs_node.data().depth == 1 {
            
        }
    }

    Ok(())
}
