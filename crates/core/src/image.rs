use crate::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{fs, io::Read, path::Path};

pub const LIMIT: u64 = 20 * 1024 * 1024;

pub fn content(bytes: &[u8]) -> Option<Value> {
    let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "image/gif"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else {
        return None;
    };
    Some(json!({"type":"input_image", "image_url": format!("data:{mime};base64,{}", STANDARD.encode(bytes))}))
}

pub fn read(path: &Path) -> Result<Value> {
    if !fs::metadata(path)?.is_file() {
        return Err(format!("{} is not a regular file", path.display()).into());
    }
    let mut bytes = vec![];
    fs::File::open(path)?.take(LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        return Err("Image exceeds 20 MiB".into());
    }
    content(&bytes).ok_or_else(|| "Expected PNG, JPEG, GIF or WebP".into())
}
