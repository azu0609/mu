use crate::{Result, read_limited};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::path::Path;

pub const LIMIT: u64 = 20 * 1024 * 1024;

pub fn mime(bytes: &[u8]) -> Option<&'static str> {
    Some(if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "image/gif"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else {
        return None;
    })
}

pub fn content(bytes: &[u8]) -> Option<Value> {
    let mime = mime(bytes)?;
    Some(json!({"type":"input_image", "image_url": format!("data:{mime};base64,{}", STANDARD.encode(bytes))}))
}

pub fn read(path: &Path) -> Result<Value> {
    content(&load(path)?).ok_or_else(|| "Expected PNG, JPEG, GIF or WebP".into())
}

pub fn load(path: &Path) -> Result<Vec<u8>> {
    let bytes = read_limited(path, LIMIT)?;
    if mime(&bytes).is_none() {
        return Err("Expected PNG, JPEG, GIF or WebP".into());
    }
    Ok(bytes)
}
