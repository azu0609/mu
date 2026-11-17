use super::Run;
use mu_client::ViewImage;
use mu_host::Result;
use std::path::Path;

pub fn run(run: &Run, args: &[String]) -> Result<()> {
    let [path] = args else {
        return Err("Usage: view_image PATH".into());
    };
    let path = Path::new(path).canonicalize().map_err(|e| format!("{path}: {e}"))?;
    run.call(ViewImage { run: run.token.clone(), path: path.clone() })?;
    println!("Image queued: {}", path.display());
    Ok(())
}
