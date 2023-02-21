extern crate core;

mod docker;
mod image_builder;
mod progress;
mod rwbuf;
mod upload;

use std::{env, path::Path};
use structopt::StructOpt;
use crate::image_builder::ImageBuilder;

const INTERNAL_LOG_LEVEL: &str = "hyper=warn,bollard=warn";
const DEFAULT_LOG_LEVEL: &str = "info";

#[derive(StructOpt)]
#[structopt(rename_all = "kebab-case")]
struct CmdArgs {
    /// Output image name
    #[structopt(short, long)]
    output: Option<String>,
    /// Upload image to repository
    #[structopt(short, long)]
    push: bool,
    /// Specify additional image environment variable
    #[structopt(long)]
    env: Vec<String>,
    /// Specify additional image volume
    #[structopt(short, long)]
    vol: Vec<String>,
    /// Specify image entrypoint
    #[structopt(short, long)]
    entrypoint: Option<String>,
    /// Input Docker image name
    image_name: String, // positional
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let log_level = env::var(env_logger::DEFAULT_FILTER_ENV).unwrap_or(DEFAULT_LOG_LEVEL.into());
    let log_filter = format!("{INTERNAL_LOG_LEVEL},{log_level}");
    env::set_var(env_logger::DEFAULT_FILTER_ENV, log_filter);
    env_logger::init();

    let cmdargs = CmdArgs::from_args();

    let builder = ImageBuilder::new(&cmdargs.image_name,
                      cmdargs.output,
                      cmdargs.env,
                      cmdargs.vol,
                      cmdargs.entrypoint);
    eprintln!("start");
    builder.build().await?;
    eprintln!("done");

    /*
    ::progress::set_total_steps(if cmdargs.push {
        image_builder::STEPS + upload::STEPS
    } else {
        image_builder::STEPS
    });

    image_builder::build_image(
        &cmdargs.image_name,
        cmdargs.output.map(AsRef::as_ref),
        cmdargs.env,
        cmdargs.vol,
        cmdargs.entrypoint,
    )
    .await?;

    if cmdargs.push {
        upload::upload_image(&cmdargs.output).await?;
    }
    */


    Ok(())
}
