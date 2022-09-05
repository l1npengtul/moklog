use crate::config::Config;
use axum::body::Bytes;
use moka::future::Cache;
use sea_orm::DatabaseConnection;

mod config;
mod injest;

pub struct State {
    pub database: DatabaseConnection,
    pub cache: Cache<String, Bytes>,
    pub config: Config,
}

fn main() {
    println!("Hello, world!");
}
