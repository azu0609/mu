mod agent;
mod api;
mod config;
mod image;
mod input;
mod instructions;
mod process;
mod server;
mod session;
mod skills;
pub mod tools;

pub use server::{serve, serve_stream, socket_path};
use std::{env, path::PathBuf};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn home() -> PathBuf {
    env::home_dir().unwrap_or_else(|| PathBuf::from("."))
}
