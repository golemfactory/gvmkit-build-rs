use bytes::Bytes;
use futures_util::StreamExt;
use futures_util::{stream, Stream};
use std::rc::Rc;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

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
        return ProgressContext {
            inner: Arc::new(Mutex::new(ProgressContextInner {
                bytes_total: 0,
                bytes_current: 0,
            })),
        };
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
