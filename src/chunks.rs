use std::path::Path;
use indicatif::{ProgressBar, ProgressStyle};
use serde::Serialize;
use sha2::{Sha256, Digest};
use tokio::fs::File;
use tokio::io::AsyncReadExt;

#[derive(Debug, Serialize)]
pub struct FileChunk {
    pub pos: u64,
    pub len: u64,
    pub sha256: String,
}

#[derive(Debug, Serialize)]
pub struct FileChunkDesc {
    pub size: u64,
    pub sha256: String,
    pub chunks: Vec<FileChunk>,
}

pub async fn createDescriptor(path: &Path, chunk_size: usize) -> anyhow::Result<FileChunkDesc> {
    let mut file = File::open(path).await?;
    let metadata = file.metadata().await.unwrap();
    let file_size = metadata.len();
    let pb = ProgressBar::new(file_size);
    let sty1 = ProgressStyle::with_template("[{msg}] {wide_bar:.cyan/blue}")
        .unwrap()
        .progress_chars("##-");
    pb.set_style(sty1);
    pb.set_message("Reading file");

    let mut file_chunks = Vec::new();
    let mut offset = 0;
    let mut sha256 = Sha256::new();
    let mut sha256_chunk = Sha256::new();
    let mut buffer = vec![0; chunk_size];
    while offset < file_size {
        let chunk_size = std::cmp::min(chunk_size, (file_size - offset) as usize);
        buffer.resize(chunk_size, 0);
        let mut chunk = FileChunk {
            pos: offset,
            len: chunk_size as u64,
            sha256: "".to_string(),
        };
        file.read_exact(&mut buffer).await.unwrap();
        sha256.update(&buffer);
        sha256_chunk.update(&buffer);
        chunk.sha256 = hex::encode(sha256_chunk.finalize_reset());
        file_chunks.push(chunk);
        offset += chunk_size as u64;
        pb.inc(chunk_size as u64);
    }
    let mut file_description = FileChunkDesc {
        size: file_size,
        sha256: "".to_string(),
        chunks: file_chunks,
    };
    let mut sha256 = Sha256::new();
    for chunk in file_description.chunks.iter() {
        sha256.update(chunk.sha256.as_bytes());
    }
    file_description.sha256 = hex::encode(sha256.finalize());
    Ok(file_description)
}

