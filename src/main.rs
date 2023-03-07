#![feature(async_iterator)]
#![feature(async_iter_from_iter)]
#![feature(arc_unwrap_or_clone)]
#![feature(path_file_prefix)]
use crate::config::Config;
use axum::body::Bytes;
use moka::future::Cache;
use sea_orm::DatabaseConnection;
use tokio::sync::Mutex;

use crate::injest::templates::SiteTheme;
#[cfg(not(target_env = "msvc"))]
use tikv_jemallocator::Jemalloc;

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

mod config;
mod injest;
mod models;
mod plugin;
mod util;

pub const SITE_CONTENT: &str = "sitecontents";
pub const SERVE_DIR: &str = "srv";

pub struct State {
    pub database: DatabaseConnection,
    pub cache: Cache<String, Bytes>,
    pub config: Config,
    pub theme: Option<SiteTheme>,
    pub build_mutex: Mutex<()>,
}

fn main() {
    println!("Hello, world!");
}
