extern crate core;

mod chunks;
mod docker;
mod image_builder;
mod progress;
mod rwbuf;
mod upload;
mod wrapper;

use crate::image_builder::ImageBuilder;

use clap::Parser;
use indicatif::ProgressStyle;
use std::path::PathBuf;

use std::env;

const INTERNAL_LOG_LEVEL: &str = "hyper=warn,bollard=warn";
const DEFAULT_LOG_LEVEL: &str = "info";

const COMPRESSION_POSSIBLE_VALUES: &[&str] = &["lzo", "gzip", "lz4", "zstd"];

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
}
use console::{style, Emoji};

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
    let descr_path = PathBuf::from(path.display().to_string() + "chunks.json");
    {
        println!(" * Writing file descriptor to {}", descr_path.display());
        let mut file = File::create(&descr_path).await?;
        let descr = chunks::createDescriptor(&path, 1000 * 1000 * 10).await?;
        file.write(&serde_json::to_vec_pretty(&descr)?).await?;
        println!(" -- file descriptor created successfully");
    }

    if cmdargs.push {
        upload::upload_image(&path).await?;
    }

    Ok(())
}
