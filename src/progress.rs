use once_cell::sync::Lazy;
use std::sync::Mutex;
use indicatif::{ProgressBar, ProgressStyle};

#[derive(Debug, Default, Clone, Copy)]
pub struct ProgressBarSettings {
    pub hidden: bool,
}

static PBS_GLOBAL: Lazy<Mutex<ProgressBarSettings>> = Lazy::new(|| Mutex::new(ProgressBarSettings::default()));

pub fn set_progress_bar_settings(pbs: ProgressBarSettings) {
    *PBS_GLOBAL.lock().unwrap() = pbs;
}

pub enum ProgressBarType {
    SingleChunk,
    DescriptorUpload,
    UploadTotal,
    UploadDetails,
    UploadChunks,
}

pub fn create_chunk_pb(len: u64, pbt: ProgressBarType) -> ProgressBar {
    if PBS_GLOBAL.lock().unwrap().hidden {
        ProgressBar::hidden()
    } else {
        let pb_chunk = ProgressBar::new(len);
        let sty_single_chunk = match pbt {
            ProgressBarType::SingleChunk => {
                ProgressStyle::with_template("[{msg:10}] {elapsed} {wide_bar:.cyan/blue} {bytes:10}/{total_bytes:10}")
                    .unwrap()
                    .progress_chars("##-")
            }
            ProgressBarType::DescriptorUpload => {
                ProgressStyle::with_template("[{msg:20}] {wide_bar:.cyan/blue} {bytes:9}/{total_bytes:9}")
                    .unwrap()
                    .progress_chars("##-")
            }
            ProgressBarType::UploadTotal => {
                ProgressStyle::with_template("[{msg:20}] {wide_bar:.cyan/blue} {bytes:9}/{total_bytes:9} {eta_precise}")
                    .unwrap()
                    .progress_chars("##-")
            }
            ProgressBarType::UploadDetails => {
                ProgressStyle::with_template("[{msg:20}] {wide_bar:.cyan/blue} {bytes:9}/{total_bytes:9}")
                    .unwrap()
                    .progress_chars("##-")
            }
            ProgressBarType::UploadChunks => {
                ProgressStyle::with_template("Chunks finished: {pos}/{len} - elapsed: {elapsed} {msg:20}")
                    .unwrap()
                    .progress_chars("##-")
            }

        };
        pb_chunk.set_style(sty_single_chunk);
        pb_chunk
    }
}
