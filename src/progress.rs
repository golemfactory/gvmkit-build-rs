use indicatif::{ProgressBar, ProgressStyle};
use once_cell::sync::Lazy;
use std::sync::Mutex;

#[derive(Debug, Default, Clone, Copy)]
pub struct ProgressBarSettings {
    pub hidden: bool,
}

static PBS_GLOBAL: Lazy<Mutex<ProgressBarSettings>> =
    Lazy::new(|| Mutex::new(ProgressBarSettings::default()));

pub fn set_progress_bar_settings(pbs: ProgressBarSettings) {
    *PBS_GLOBAL.lock().unwrap() = pbs;
}

pub enum ProgressBarType {
    PullLine1,
    PullLine2,
    PullLayer,
    CreateDescriptor,
    CopyingFiles,
    SingleChunk,
    DescriptorUpload,
    UploadTotal,
    UploadDetails,
    UploadChunks,
}

fn create_internal_style(template: &str) -> ProgressStyle {
    ProgressStyle::with_template(template)
        .unwrap()
        .progress_chars("##-")
}

pub fn create_chunk_pb(len: u64, pbt: ProgressBarType) -> ProgressBar {
    if PBS_GLOBAL.lock().unwrap().hidden {
        ProgressBar::hidden()
    } else {
        let pb_chunk = ProgressBar::new(len);
        #[rustfmt::skip]
        let sty_single_chunk = match pbt {
            ProgressBarType::PullLine1 => create_internal_style(
                "{wide_bar:.cyan/blue}"
            ),
            ProgressBarType::PullLine2 => create_internal_style(
                "{bytes:9}/(est. {total_bytes:9} [{wide_msg}]"
            ),
            ProgressBarType::PullLayer => create_internal_style(
                "[{msg:20}] {wide_bar:.cyan/blue} {bytes:10}/{total_bytes:10}",
            ),
            ProgressBarType::CreateDescriptor => create_internal_style(
                "[{msg:20}] {wide_bar:.cyan/blue} {bytes:10}/{total_bytes:10}",
            ),
            ProgressBarType::CopyingFiles => create_internal_style(
                "[{msg:20}] {wide_bar:.cyan/blue} {bytes:10}/{total_bytes:10}",
            ),
            ProgressBarType::SingleChunk => create_internal_style(
                "[{msg:10}] {elapsed} {wide_bar:.cyan/blue} {bytes:10}/{total_bytes:10}",
            ),
            ProgressBarType::DescriptorUpload => create_internal_style(
                "[{msg:20}] {wide_bar:.cyan/blue} {bytes:10}/{total_bytes:10}",
            ),
            ProgressBarType::UploadTotal => create_internal_style(
                "[{msg:20}] {wide_bar:.cyan/blue} {bytes:10}/{total_bytes:10}",
            ),
            ProgressBarType::UploadDetails => create_internal_style(
                "Upload info: {msg:30}"
            ),
            ProgressBarType::UploadChunks => create_internal_style(
                "Chunks finished: {pos}/{len}"
            ),
        };
        pb_chunk.set_style(sty_single_chunk);
        pb_chunk
    }
}
