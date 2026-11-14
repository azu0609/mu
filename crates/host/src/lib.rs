mod agent;
mod api;
mod config;
mod devices;
mod directory;
mod image;
mod input;
mod instructions;
mod net;
mod process;
mod profile;
mod server;
mod session;
mod skills;
pub mod tools;
mod upload;

pub use server::{listen_address, serve, serve_stream, socket_path};
use std::{env, path::PathBuf};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn home() -> PathBuf {
    env::home_dir().unwrap_or_else(|| PathBuf::from("."))
}
