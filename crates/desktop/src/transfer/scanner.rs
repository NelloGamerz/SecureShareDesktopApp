use crate::{
    models::file_info::{modified_nanos, FileInfo},
    transfer::errors::Result,
};
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
};
use tokio::fs;
pub async fn scan(paths: &[String]) -> Result<Vec<FileInfo>> {
    let mut files = Vec::new();
    for source in paths {
        let p = PathBuf::from(source);
        /*
         * Relative to the *parent*, so a picked folder arrives as itself —
         * `Photos/a.jpg`, not `a.jpg` — and the receiver writes the folder as
         * it was on disk rather than its contents loose in the download root.
         */
        let base = p.parent().unwrap_or(Path::new("")).to_path_buf();
        visit(&p, &base, &mut files).await?;
    }
    Ok(files)
}
async fn visit(path: &Path, base: &Path, out: &mut Vec<FileInfo>) -> Result<()> {
    let meta = fs::metadata(path).await?;
    if meta.is_file() {
        let relative = path
            .strip_prefix(base)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let mut h = DefaultHasher::new();
        path.hash(&mut h);
        meta.len().hash(&mut h);
        out.push(FileInfo {
            id: format!("{:x}", h.finish()),
            path: path.to_path_buf(),
            relative_path: relative,
            size: meta.len(),
            modified_nanos: modified_nanos(&meta),
        });
        return Ok(());
    }
    let mut dirs = vec![path.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let mut entries = fs::read_dir(&dir).await?;
        while let Some(e) = entries.next_entry().await? {
            let m = e.metadata().await?;
            if m.is_dir() {
                dirs.push(e.path())
            } else if m.is_file() {
                let p = e.path();
                let relative = p
                    .strip_prefix(base)
                    .unwrap_or(&p)
                    .to_string_lossy()
                    .replace('\\', "/");
                let mut h = DefaultHasher::new();
                p.hash(&mut h);
                m.len().hash(&mut h);
                out.push(FileInfo {
                    id: format!("{:x}", h.finish()),
                    path: p,
                    relative_path: relative,
                    size: m.len(),
                    modified_nanos: modified_nanos(&m),
                })
            }
        }
    }
    Ok(())
}
