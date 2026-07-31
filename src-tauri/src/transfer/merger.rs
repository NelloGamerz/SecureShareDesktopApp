use crate::transfer::errors::Result;
use std::path::Path;
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
};
pub async fn merge(parts: &Path, destination: &Path, total: u64) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).await?
    };
    let temporary = destination.with_extension("transfer-tmp");
    let mut output = fs::File::create(&temporary).await?;
    for i in 0..total {
        let mut input = fs::File::open(parts.join(format!("{i}.part"))).await?;
        let mut buffer = vec![0; 64 * 1024];
        loop {
            let n = input.read(&mut buffer).await?;
            if n == 0 {
                break;
            }
            output.write_all(&buffer[..n]).await?;
        }
    }
    output.flush().await?;
    drop(output);
    fs::rename(&temporary, destination).await?;
    Ok(())
}
