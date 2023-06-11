use bytes::Bytes;
use futures_util::StreamExt;
use futures_util::{stream, Stream};
use std::io::SeekFrom;
use std::path::Path;

use std::sync::{Arc, Mutex};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

struct ProgressContextInner {
    bytes_total: u64,
    bytes_current: u64,
}

#[derive(Clone)]
pub struct ProgressContext {
    inner: Arc<Mutex<ProgressContextInner>>,
}
impl ProgressContext {
    pub fn new() -> Self {
        ProgressContext {
            inner: Arc::new(Mutex::new(ProgressContextInner {
                bytes_total: 0,
                bytes_current: 0,
            })),
        }
    }

    pub fn total_bytes(&self) -> u64 {
        let pc = self.inner.lock().unwrap();
        pc.bytes_total
    }
}

pub fn stream_with_progress(
    stream_in: impl Stream<Item = Result<Bytes, bollard::errors::Error>> + std::marker::Unpin,
    pb: &indicatif::ProgressBar,
    pc: ProgressContext,
) -> impl Stream<Item = Result<Bytes, bollard::errors::Error>> {
    let pb = pb.clone();

    //Progress bar used is in this wrapper is designed for copying files
    //not downloading from internet
    //It has a granularity of 100kB to prevent too many updates

    stream::unfold(stream_in, move |mut stream_in| {
        let pb = pb.clone();
        let pc = pc.clone();
        async move {
            let chunk = stream_in.next().await;
            if let Some(chunk) = chunk {
                match &chunk {
                    Ok(chunk) => {
                        //Prevent too many updates to the progress bar
                        let (do_update, bytes_total) = {
                            let mut pc = pc.inner.lock().unwrap();
                            pc.bytes_total += chunk.len() as u64;
                            pc.bytes_current += chunk.len() as u64;
                            if pc.bytes_current > 100000 {
                                pc.bytes_current = 0;
                                (true, pc.bytes_total)
                            } else {
                                (false, pc.bytes_total)
                            }
                        };
                        if do_update {
                            pb.set_position(bytes_total);
                        }
                    }
                    Err(err) => {
                        log::error!("Error reading stream: {}", err);
                    }
                }
                Some((chunk, stream_in))
            } else {
                None
            }
        }
    })
}

pub async fn stream_file_with_progress(
    file_in: &Path,
    chunk: Option<std::ops::Range<usize>>,
    pb_file: Option<indicatif::ProgressBar>,
    pb_global: Option<indicatif::ProgressBar>,
) -> anyhow::Result<impl Stream<Item = Result<Bytes, anyhow::Error>>> {
    let mut file = File::open(file_in).await?;
    let file_size = file.metadata().await?.len();
    let bytes_to_read = if let Some(range) = chunk {
        if range.end as u64 > file_size {
            log::error!("Range end is greater than file size");
            return Err(anyhow::anyhow!("Range end is greater than file size"));
        }
        file.seek(SeekFrom::Start(range.start as u64)).await?;
        range.end - range.start
    } else {
        file_size as usize
    };

    if let Some(pb) = pb_file.clone() {
        pb.set_length(bytes_to_read as u64)
    };
    let res = stream::unfold((file, bytes_to_read), move |(mut file, bytes_to_read)| {
        let pb_file = pb_file.clone();
        let pb_global = pb_global.clone();
        async move {
            if bytes_to_read == 0 {
                if let Some(pb) = pb_file {
                    pb.finish()
                };
                return None;
            }
            //println!("Bytes to read: {}", bytes_to_read);
            let current_read_size = std::cmp::min(bytes_to_read, 100000);
            let mut buf = vec![0u8; current_read_size];
            let bytes_read = file.read_exact(&mut buf).await.unwrap();
            let bytes = Bytes::from(buf);
            if let Some(pb) = pb_file {
                pb.inc(bytes_read as u64)
            }
            if let Some(pb_global) = pb_global {
                pb_global.inc(bytes_read as u64)
            }
            Some((
                Ok::<Bytes, anyhow::Error>(bytes),
                (file, bytes_to_read - bytes_read),
            ))
        }
    });
    Ok(res)
}
