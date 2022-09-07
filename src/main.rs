#![feature(async_iterator)]
#![feature(async_iter_from_iter)]

use crate::config::Config;
use axum::body::Bytes;
use moka::future::Cache;
use sea_orm::DatabaseConnection;
use tokio::sync::Mutex;

mod config;
mod injest;
mod models;

pub const SITE_CONTENT: &str = "sitecontents";

pub struct State {
    pub database: DatabaseConnection,
    pub cache: Cache<String, Bytes>,
    pub config: Config,
    pub build_mutex: Mutex<()>,
}

fn main() {
    println!("Hello, world!");
}
