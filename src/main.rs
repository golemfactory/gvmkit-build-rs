extern crate core;

mod docker;
mod image_builder;
mod progress;
mod rwbuf;
mod upload;
mod wrapper;
mod chunks;

use crate::image_builder::ImageBuilder;

use clap::Parser;
use indicatif::ProgressStyle;
use std::{env, fs};
use std::path::Path;
use std::time::Duration;

const INTERNAL_LOG_LEVEL: &str = "hyper=warn,bollard=warn";
const DEFAULT_LOG_LEVEL: &str = "info";

const COMPRESSION_POSSIBLE_VALUES: &[&str] = &["lzo", "gzip", "lz4", "zstd"];

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct CmdArgs {
    /// Output image name
    #[arg(short, long)]
    output: Option<String>,
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
    /// Input Docker image name
    image_name: String, // positional

    #[arg(long, default_value = "lzo")]
    compression_method: String,
    #[arg(long)]
    compression_level: Option<u32>,

}
use console::{style, Emoji};
use indicatif::{HumanDuration, MultiProgress, ProgressBar};
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

    let spinner_style = ProgressStyle::with_template("{prefix:.bold.dim} {spinner} {wide_msg}")
        .unwrap()
        .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ ");

    println!(
        "{} {}Resolving packages...",
        style("[1/4]").bold().dim(),
        LOOKING_GLASS
    );
    let deps = 1232;

    let path = builder.build().await?;
    let descr_path = path.with_extension("chunks.json");
    {
        println!(" * Writing file descriptor to {}", descr_path.display());
        let mut file = File::create(&descr_path).await?;
        let descr = chunks::createDescriptor(&path, 1000 * 1000 * 10).await?;
        file.write(&serde_json::to_vec_pretty(&descr)?).await?;
        println!(" -- file descriptor created successfully");
    }


    if cmdargs.push {
        // upload::upload_image(&cmdargs.output).await?;
    }

    Ok(())
}
