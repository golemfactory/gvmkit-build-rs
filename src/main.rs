extern crate core;

mod chunks;
mod docker;
mod image;
mod metadata;
mod upload;
mod wrapper;

use crate::image::{ImageBuilder, ImageName};

use clap::Parser;
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
    /// Upload image to repository, you can provide optional argument in format <username>/<repository>:<tag>
    /// Otherwise username, repository and tag is taken from image name
    #[arg(long)]
    push: bool,
    /// Specify additional image label
    #[arg(long)]
    push_to: Option<String>,
    /// Skip login to repository
    #[arg(long)]
    no_login: bool,
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
}
use tokio::fs;

use crate::chunks::FileChunkDesc;
use crate::upload::{attach_to_repo, full_upload, upload_descriptor};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

#[tokio::main()]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
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

    //parse image name to check if proper name is provided
    let _ = ImageName::from_str_name(&cmdargs.image_name)?;

    let push_image_name = if !cmdargs.no_login {
        if let Some(push_to) = &cmdargs.push_to {
            //pushing to user/repository:tag given by the user
            let push_image_name = ImageName::from_str_name(push_to)?;
            if push_image_name.user.is_none() {
                return Err(anyhow::anyhow!(
                    "You have to specify username in push-to argument"
                ));
            }
            Some(push_image_name)
        } else if cmdargs.push {
            //pushing to user/repository:tag from image name
            let push_image_name = ImageName::from_str_name(&cmdargs.image_name)?;
            if push_image_name.user.is_none() {
                return Err(anyhow::anyhow!(
                "You have to specify username. Instead of --push you can use --push-to <username>/<repository>:<tag>"
            ));
            }
            Some(push_image_name)
        } else {
            //not pushing at all
            None
        }
    } else {
        None
    };

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

    if cmdargs.push || cmdargs.push_to.is_some() {
        let full_upload_needed = upload_descriptor(&descr_path).await?;

        if full_upload_needed {
            if let Some(push_image_name) = &push_image_name {
                //check if we can attach to the repo
                attach_to_repo(&descr_path, push_image_name, true).await?;
            }
            full_upload(&path, &descr_path, cmdargs.upload_workers).await?;
        }
        if let Some(push_image_name) = &push_image_name {
            //attach to repo after upload
            attach_to_repo(&descr_path, push_image_name, false).await?;
        }
    }

    Ok(())
}
