use crate::Result;
use std::{env, fs};

pub(crate) fn var(name: &str) -> Result<Option<String>> {
    match env::var(name) {
        Ok(value) if !value.is_empty() => return Ok(Some(value)),
        Ok(_) | Err(env::VarError::NotPresent) => {}
        Err(error) => return Err(error.into()),
    }
    let path = match env::var(format!("{name}_FILE")) {
        Ok(path) => path,
        Err(env::VarError::NotPresent) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let value = fs::read_to_string(&path).map_err(|error| format!("{name}_FILE={path}: {error}"))?;
    let value = value.trim();
    Ok((!value.is_empty()).then(|| value.to_owned()))
}
