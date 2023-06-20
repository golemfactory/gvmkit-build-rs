extern crate core;

mod chunks;
mod docker;
mod image;
mod login;
mod metadata;
mod progress;
mod upload;
mod wrapper;

use crate::image::{ImageBuilder, ImageName};

use clap::Parser;
use std::path::PathBuf;

use std::env;

const COMPRESSION_POSSIBLE_VALUES: &[&str] = &["lzo", "gzip", "lz4", "zstd", "xz"];

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct CmdArgs {
    /// Input Docker image name
    image_name: Option<String>,
    /// Output image name
    #[arg(help_heading = Some("Image creation"), short, long)]
    output: Option<String>,
    /// Force overwriting existing image, even if it matches image
    #[arg(help_heading = Some("Image creation"), short, long)]
    force: bool,
    /// Upload image to repository, repository and tag is taken from image name <username>/<repository>:<tag>
    #[arg(help_heading = Some("Image upload"), long)]
    push: bool,
    /// Alternative to --push: Upload image to repository, use format <username>/<repository>:<tag>
    #[arg(help_heading = Some("Image upload"), long)]
    push_to: Option<String>,
    /// Force login action and do not build or push image
    /// Use if you want to change username or personal access token
    #[arg(help_heading = Some("Portal"), long)]
    login: bool,
    ///check if saved login is valid
    #[arg(help_heading = Some("Portal"), long)]
    login_check: bool,
    /// Force logout action (forget saved credentials)
    #[arg(help_heading = Some("Portal"), long)]
    logout: bool,
    /// Skip login to repository (anonymous upload)
    #[arg(help_heading = Some("Portal"), long)]
    nologin: bool,
    /// Specify ready gvmi file to upload to registry, do not use until you now what are you doing
    #[arg(help_heading = Some("Maintenance options"), short, long)]
    direct_file_upload: Option<String>,
    /// Specify additional image environment variable
    #[arg(help_heading = Some("Legacy/unused image options"), long)]
    env: Vec<String>,
    /// Specify additional image volume
    #[arg(help_heading = Some("Legacy/unused image options"), short, long)]
    vol: Vec<String>,
    /// Specify image entrypoint
    #[arg(help_heading = Some("Legacy/unused image options"), short, long)]
    entrypoint: Option<String>,
    /// Possible values: lzo, gzip, lz4, zstd, xz
    #[arg(help_heading = Some("Image creation"), long, default_value = "lzo")]
    compression_method: String,
    /// Possible values: lzo [1-9] (default 8), gzip [1-9] (default 9), zstd [1-22] (default 15)
    /// lz4 and xz do not support this option
    #[arg(help_heading = Some("Image creation"), long)]
    compression_level: Option<u32>,
    /// Specify chunk size (default 2MB)
    #[arg(help_heading = Some("Portal"), long)]
    upload_chunk_size: Option<u64>,
    /// Specify number of upload workers (default 4)
    #[arg(help_heading = Some("Portal"), long, default_value = "4")]
    upload_workers: usize,
    /// Hide progress bars during operation
    #[arg(help_heading = Some("Extra options"), long)]
    hide_progress: bool,
}
use tokio::fs;

use crate::chunks::FileChunkDesc;
use crate::login::remove_credentials;
use crate::progress::set_progress_bar_settings;
use crate::upload::{attach_to_repo, check_login, full_upload, upload_descriptor, REGISTRY_URL};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

#[tokio::main()]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
    let log_level = env::var("RUST_LOG").unwrap_or("info,bollard=warn,hyper=warn".to_string());
    env::set_var("RUST_LOG", log_level);
    env_logger::init();

    let cmdargs = <CmdArgs as Parser>::parse();

    set_progress_bar_settings(progress::ProgressBarSettings {
        hidden: cmdargs.hide_progress,
    });

    if cmdargs.nologin && cmdargs.push_to.is_some() {
        return Err(anyhow::anyhow!(
            "Options --push-to and --nologin are incompatible"
        ));
    }
    if !COMPRESSION_POSSIBLE_VALUES.contains(&cmdargs.compression_method.as_str()) {
        return Err(anyhow::anyhow!(
            "Not supported compression method: {}, possible values {}",
            cmdargs.compression_method,
            COMPRESSION_POSSIBLE_VALUES.join(", ")
        ));
    }
    if cmdargs.login {
        println!("Logging in to golem registry: {}", REGISTRY_URL.as_str());
        login::login(None, true).await?;
        return Ok(());
    }
    if cmdargs.logout {
        remove_credentials().await?;
        return Ok(());
    }
    if cmdargs.login_check {
        println!(
            "Checking login to golem registry: {}",
            REGISTRY_URL.as_str()
        );
        return if login::check_if_valid_login().await? {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Login is not valid"))
        };
    }
    let push_image_name = if !cmdargs.nologin {
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
            let cmd_image_name = match cmdargs.image_name.clone() {
                Some(image_name) => image_name,
                None => return Err(anyhow::anyhow!("You have to specify image name to build")),
            };
            //pushing to user/repository:tag from image name
            let push_image_name = ImageName::from_str_name(&cmd_image_name)?;
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

    let (user_name, pat) = if !cmdargs.nologin && (cmdargs.push_to.is_some() || cmdargs.push) {
        println!("Logging in to golem registry: {}", REGISTRY_URL.as_str());

        if let (Ok(registry_user), Ok(registry_token)) =
            (env::var("REGISTRY_USER"), env::var("REGISTRY_TOKEN"))
        {
            println!(" -- Using credentials from environment variables (REGISTRY_USER and REGISTRY_TOKEN)");
            let res = check_login(&registry_user, &registry_token).await?;
            if !res {
                return Err(anyhow::anyhow!(
                    "Login to golem registry: {} failed",
                    REGISTRY_URL.as_str()
                ));
            }
            (registry_user, registry_token)
        } else if let Some(user_name) = &push_image_name.as_ref().map(|x| &x.user).unwrap_or(&None)
        {
            login::login(Some(user_name), false).await?
        } else {
            return Err(anyhow::anyhow!(
                    "You have to specify username. Instead of --push you can use --push-to <username>/<repository>:<tag>"
                ));
        }
    } else {
        (String::new(), String::new())
    };

    let path = if let Some(direct_file_upload) = cmdargs.direct_file_upload {
        //skip build and upload file directly
        let path_buf = PathBuf::from(&direct_file_upload);
        if path_buf.exists() {
            path_buf
        } else {
            return Err(anyhow::anyhow!(
                "File {} does not exist",
                direct_file_upload
            ));
        }
    } else {
        let cmd_image_name = match cmdargs.image_name {
            Some(image_name) => image_name,
            None => return Err(anyhow::anyhow!("You have to specify image name to build")),
        };
        //parse image name to check if proper name is provided
        let _ = ImageName::from_str_name(&cmd_image_name)?;

        let builder = ImageBuilder::new(
            &cmd_image_name,
            cmdargs.output,
            cmdargs.force,
            cmdargs.env,
            cmdargs.vol,
            cmdargs.entrypoint,
            cmdargs.compression_method,
            cmdargs.compression_level,
        );

        builder.build().await?
    };

    let image_file_size = fs::metadata(&path).await?.len();
    let chunk_size = if let Some(chunk_size) = cmdargs.upload_chunk_size {
        chunk_size
    } else if image_file_size > 1024 * 1024 * 500 {
        10 * 1024 * 1024
    } else if image_file_size > 1024 * 1024 * 200 {
        5 * 1024 * 1024
    } else {
        2 * 1024 * 1024
    };

    let descr_path = PathBuf::from(path.display().to_string() + ".descr.bin");
    let descr = {
        let path_meta = fs::metadata(&path).await?;
        let descr = if descr_path.exists() {
            if fs::metadata(&descr_path)
                .await?
                .modified()
                .expect("Modified field has to be here")
                < path_meta.modified().expect("Modified field has to be here")
            {
                println!(" -- File descriptor is older than image, recreating");
                None
            } else {
                match tokio::fs::read(&descr_path).await {
                    Ok(file_descr_bytes) => {
                        match FileChunkDesc::deserialize_from_bytes(&file_descr_bytes) {
                            Ok(descr) => {
                                if descr.chunk_size != chunk_size {
                                    println!(" -- chunk size changed, recreating file descriptor");
                                    None
                                } else {
                                    println!(" -- file descriptor already exists and is newer");
                                    println!(" -- image hash: sha3:{}", hex::encode(descr.sha3));
                                    Some(descr)
                                }
                            }
                            Err(e) => {
                                println!(" -- failed to deserialize file descriptor: {}", e);
                                None
                            }
                        }
                    }
                    Err(e) => {
                        println!(
                            " -- failed to read file descriptor: {} {}",
                            descr_path.display(),
                            e
                        );
                        None
                    }
                }
            }
        } else {
            None
        };
        if let Some(descr) = descr {
            descr
        } else {
            println!(" * Writing file descriptor to {}", descr_path.display());
            let mut file = File::create(&descr_path).await?;
            let descr = chunks::create_descriptor(&path, chunk_size as usize).await?;
            file.write_all(&descr.serialize_to_bytes()).await?;
            println!(" -- file descriptor created successfully");
            println!(" -- image hash: sha3:{}", hex::encode(descr.sha3));
            descr
        }
    };

    if cmdargs.push || cmdargs.push_to.is_some() {
        println!(
            "Uploading image to golem registry: {}",
            REGISTRY_URL.as_str()
        );
        let full_upload_needed = upload_descriptor(&descr_path).await?;

        if full_upload_needed {
            if let Some(push_image_name) = &push_image_name {
                //check if we can attach to the repo before uploading the file
                attach_to_repo(
                    &descr.get_descr_hash_str(),
                    push_image_name,
                    &user_name,
                    &pat,
                    true,
                )
                .await?;
            }
            full_upload(&path, &descr, cmdargs.upload_workers).await?;
        }
        if let Some(push_image_name) = &push_image_name {
            //attach to repo after upload
            attach_to_repo(
                &descr.get_descr_hash_str(),
                push_image_name,
                &user_name,
                &pat,
                false,
            )
            .await?;
        }
    }

    Ok(())
}
