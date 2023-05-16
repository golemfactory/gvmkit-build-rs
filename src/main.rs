extern crate core;

mod chunks;
mod docker;
mod image_builder;
mod metadata;
mod upload;
mod wrapper;

use crate::image_builder::ImageBuilder;

use clap::Parser;
use indicatif::ProgressStyle;
use std::path::PathBuf;

use std::env;

const INTERNAL_LOG_LEVEL: &str = "hyper=warn,bollard=warn";
const DEFAULT_LOG_LEVEL: &str = "info";

const COMPRESSION_POSSIBLE_VALUES: &[&str] = &["lzo", "gzip", "lz4", "zstd", "xz"];

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct CmdArgs {
    /// Input Docker image name
    image_name: String,
    /// Output image name
    #[arg(short, long)]
    output: Option<String>,
    /// Force overwriting existing image, even if it matches image
    #[arg(short, long)]
    force: bool,
    /// Upload image to repository
    #[arg(short, long)]
    push: bool,
    /// Specify additional image environment variable
    #[arg(long)]
    env: Vec<String>,
    /// Specify additional image volume
    #[arg(short, long)]
    vol: Vec<String>,
    /// Specify image entrypoint
    #[arg(short, long)]
    entrypoint: Option<String>,
    /// Possible values: lzo, gzip, lz4, zstd, xz
    #[arg(long, default_value = "lzo")]
    compression_method: String,
    /// Possible values: lzo [1-9] (default 8), gzip [1-9] (default 9), zstd [1-22] (default 15)
    /// lz4 and xz do not support this option
    #[arg(long)]
    compression_level: Option<u32>,

    /// Specify chunk size (default 2MB)
    #[arg(long, default_value = "2000000")]
    upload_chunk_size: usize,
    /// Specify number of upload workers (default 4)
    #[arg(long, default_value = "4")]
    upload_workers: usize,

    #[arg(long)]
    upload_username: Option<String>,
    #[arg(long)]
    upload_repository: Option<String>,
    #[arg(long)]
    upload_tag: Option<String>,
}
use console::{style, Emoji};
use tokio::fs;

use crate::chunks::FileChunkDesc;
use crate::upload::full_upload;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

static LOOKING_GLASS: Emoji<'_, '_> = Emoji("🔍  ", "");
#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let log_level = env::var(env_logger::DEFAULT_FILTER_ENV).unwrap_or(DEFAULT_LOG_LEVEL.into());
    let log_filter = format!("{INTERNAL_LOG_LEVEL},{log_level}");
    env::set_var(env_logger::DEFAULT_FILTER_ENV, log_filter);
    env_logger::init();

    let cmdargs = <CmdArgs as Parser>::parse();

    if !COMPRESSION_POSSIBLE_VALUES.contains(&cmdargs.compression_method.as_str()) {
        return Err(anyhow::anyhow!(
            "Not supported compression method: {}, possible values {}",
            cmdargs.compression_method,
            COMPRESSION_POSSIBLE_VALUES.join(", ")
        ));
    }
    let builder = ImageBuilder::new(
        &cmdargs.image_name,
        cmdargs.output,
        cmdargs.force,
        cmdargs.env,
        cmdargs.vol,
        cmdargs.entrypoint,
        cmdargs.compression_method,
        cmdargs.compression_level,
    );

    let _spinner_style = ProgressStyle::with_template("{prefix:.bold.dim} {spinner} {wide_msg}")
        .unwrap()
        .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ ");

    println!(
        "{} {}Resolving packages...",
        style("[1/4]").bold().dim(),
        LOOKING_GLASS
    );
    let _deps = 1232;

    let path = builder.build().await?;
    let descr_path = PathBuf::from(path.display().to_string() + ".descr.bin");
    {
        let path_meta = fs::metadata(&path).await?;
        let mut recrate_descr = false;
        if descr_path.exists() {
            if fs::metadata(&descr_path)
                .await?
                .modified()
                .expect("Modified field has to be here")
                < path_meta.modified().expect("Modified field has to be here")
            {
                println!(" -- File descriptor is older than image, recreating");
                recrate_descr = true;
            } else {
                match tokio::fs::read(&descr_path).await {
                    Ok(file_descr_bytes) => {
                        match FileChunkDesc::deserialize_from_bytes(&file_descr_bytes) {
                            Ok(descr) => {
                                if descr.chunk_size != cmdargs.upload_chunk_size as u64 {
                                    println!(" -- chunk size changed, recreating file descriptor");
                                    recrate_descr = true;
                                } else {
                                    println!(" -- file descriptor already exists and is newer");
                                }
                            }
                            Err(e) => {
                                println!(" -- failed to deserialize file descriptor: {}", e);
                                recrate_descr = true;
                            }
                        };
                    }
                    Err(e) => {
                        println!(
                            " -- failed to read file descriptor: {} {}",
                            descr_path.display(),
                            e
                        );
                        recrate_descr = true;
                    }
                };
            }
        } else {
            recrate_descr = true;
        }
        if recrate_descr {
            println!(" * Writing file descriptor to {}", descr_path.display());
            let mut file = File::create(&descr_path).await?;
            let descr = chunks::create_descriptor(&path, cmdargs.upload_chunk_size).await?;
            file.write_all(&descr.serialize_to_bytes()).await?;
            println!(" -- file descriptor created successfully");
        }
    }

    if cmdargs.push {
        full_upload(&path, &descr_path, cmdargs.upload_workers, cmdargs.upload_username, cmdargs.upload_repository, cmdargs.upload_tag).await?;
    }

    Ok(())
}
