pub mod agent;
mod api;
mod config;
mod image;
pub mod input;
mod instructions;
mod process;
pub mod session;
pub mod skills;
pub mod tools;

use std::{env, path::PathBuf};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn home() -> PathBuf {
    env::home_dir().unwrap_or_else(|| PathBuf::from("."))
}
