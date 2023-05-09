extern crate core;

mod docker;
mod image_builder;
mod progress;
mod rwbuf;
mod upload;
mod wrapper;

use crate::image_builder::ImageBuilder;

use clap::Parser;
use indicatif::ProgressStyle;
use std::env;
use std::time::Duration;

const INTERNAL_LOG_LEVEL: &str = "hyper=warn,bollard=warn";
const DEFAULT_LOG_LEVEL: &str = "info";

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
}
use console::{style, Emoji};
use indicatif::{HumanDuration, MultiProgress, ProgressBar};
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

    builder.build().await?;

    if cmdargs.push {
        // upload::upload_image(&cmdargs.output).await?;
    }

    Ok(())
}
