use std::collections::HashMap;

use std::{
    fs,
    path::{Path, PathBuf},
};

use bollard::container;
use bollard::container::{
    DownloadFromContainerOptions, LogOutput, LogsOptions, UploadToContainerOptions,
};

use crate::wrapper::{stream_with_progress, ProgressContext};
use anyhow::anyhow;

use futures_util::TryStreamExt;
use humansize::DECIMAL;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

use crate::image::name::ImageName;
use crate::metadata::{add_metadata_outside, read_metadata_outside};
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct ImageBuilder {
    image_name: String,
    output: Option<String>,
    force_overwrite: bool,
    env: Vec<String>,
    volumes: Vec<String>,
    entrypoint: Option<String>,
    compression_method: String,
    compression_level: Option<u32>,
}

impl ImageBuilder {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        image_name: &str,
        output: Option<String>,
        force_overwrite: bool,
        env: Vec<String>,
        volumes: Vec<String>,
        entrypoint: Option<String>,
        compression_method: String,
        compression_level: Option<u32>,
    ) -> Self {
        ImageBuilder {
            image_name: image_name.to_string(),
            output,
            force_overwrite,
            env,
            volumes,
            entrypoint,
            compression_method,
            compression_level,
        }
    }

    pub async fn build(&self) -> anyhow::Result<PathBuf> {
        use bollard::{
            image,
            service::{HostConfig, Mount, MountTypeEnum},
            Docker,
        };
        println!("Building image: {}", self.image_name);
        let docker = match Docker::connect_with_local_defaults() {
            Ok(docker) => docker,
            Err(err) => {
                log::error!("Failed to connect to docker: {}", err);
                return Err(anyhow::anyhow!("Failed to connect to docker: {}", err));
            }
        };

        let sty = ProgressStyle::with_template("[{msg:20}] {wide_bar:.cyan/blue} {pos:9}/{len:9}")
            .unwrap()
            .progress_chars("##-");

        let mp = MultiProgress::new();
        let layers = Arc::new(Mutex::new(HashMap::<String, ProgressBar>::new()));

        println!(
            "* Step1 - create image from given name: {} ...",
            self.image_name
        );

        let parsed_name = ImageName::from_str_name(&self.image_name)?;

        let image_base_name = parsed_name.to_base_name();
        let tag_from_image_name = parsed_name.tag;

        println!(
            " -- image: {}\n -- tag: {}",
            image_base_name, tag_from_image_name
        );
        match docker
            .create_image(
                Some(image::CreateImageOptions {
                    from_image: self.image_name.as_str(),
                    tag: &tag_from_image_name,
                    ..Default::default()
                }),
                None,
                None,
            )
            .try_for_each(|ev| async {
                // let pb = pb.clone();
                let layers = layers.clone();
                //log::info!("{:?}", ev);
                if let Some(id) = ev.clone().id {
                    if ev.progress_detail.is_none() {
                        //log::info!(" -- {:?}", ev);
                        return Ok(());
                    };
                    let pb = {
                        let mut layers = layers.lock().await;
                        if let Some(pb) = layers.get(&id) {
                            pb.clone()
                        } else {
                            //log::info!(" -- new Layer: {:?}", ev);
                            let pb = mp.add(ProgressBar::new(1));
                            pb.set_style(sty.clone());
                            layers.insert(id.clone(), pb.clone());
                            pb
                        }
                    };
                    if let Some(status) = ev.status {
                        pb.set_message(status);
                    }

                    match ev.progress_detail {
                        Some(detail) => {
                            //log::info!(" -- {:?}", detail);
                            pb.set_length(detail.total.unwrap_or(1) as u64);
                            pb.set_position(detail.current.unwrap_or(0) as u64);
                        }
                        None => {
                            pb.set_length(1);
                            pb.set_position(0);
                        }
                    }
                }
                Ok(())
            })
            .await
        {
            Ok(_) => {}
            Err(err) => {
                log::error!("Failed to create image: {}", err);
                return Err(anyhow::anyhow!("Failed to create image: {}", err));
            }
        }
        for (_, pb) in layers.lock().await.iter() {
            pb.finish_and_clear();
        }

        //pb.finish_and_clear();

        println!("* Step2 - inspect created image: {} ...", self.image_name);
        let image = docker.inspect_image(&self.image_name).await?;
        let image_id = image.id.unwrap();
        let image_id = if image_id.starts_with("sha256:") {
            image_id.replace("sha256:", "")
        } else {
            log::error!("Image id is not sha256: {}", image_id);
            return Err(anyhow::anyhow!("Image id is not sha256: {}", image_id));
        };

        let image_size = image.size.unwrap_or(0);

        println!(
            " -- Image name: {}\n -- Image id: {}\n -- Image size: {}",
            self.image_name,
            image_id,
            humansize::format_size(image_size as u64, DECIMAL)
        );

        let path = if let Some(path) = &self.output {
            path.clone()
        } else {
            format!(
                "{}-{}-{}.gvmi",
                image_base_name.replace('/', "-"),
                tag_from_image_name,
                &image_id[0..10]
            )
        };

        let path = Path::new(&path);
        if path.exists() {
            let meta_out = read_metadata_outside(path).await;
            match meta_out {
                Ok(meta_out) => {
                    if let Some(image_left) = meta_out.image {
                        if image_left == image_id {
                            if self.force_overwrite {
                                println!(
                                    " -- GVMI image already exists - overwriting: {}",
                                    path.display()
                                );
                            } else {
                                println!(" -- GVMI image already exists: {}", path.display());
                                return Ok(PathBuf::from(path));
                            }
                        } else {
                            println!(" -- GVMI image id mismatch: {}", path.display());
                        }
                    }
                }
                Err(_err) => {
                    println!(
                        " -- Failed to read metadata from GVMI image: {}",
                        path.display()
                    );
                }
            }
        }
        if let Err(err) = fs::write(path, "") {
            log::error!("Failed to create output file: {} {}", path.display(), err);
            return Err(anyhow::anyhow!("Failed to create output file: {}", err));
        }

        let path = path
            .canonicalize()?
            .display()
            .to_string()
            .replace(r"\\?\", ""); // strip \\?\ prefix on windows
        println!(" -- GVMI image output path: {}", path);

        println!(
            "* Step3 - create container from image: {} ...",
            self.image_name
        );

        let container = docker
            .create_container::<String, String>(
                None,
                container::Config {
                    image: Some(image_id.clone()),
                    host_config: Some(HostConfig {
                        auto_remove: Some(true),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )
            .await?;
        let container_id = container.id;
        let cont = docker
            .inspect_container(&container_id, None::<container::InspectContainerOptions>)
            .await?;

        println!(" -- Container id: {}", &container_id[0..12]);

        let tool_image_name = "scx1332/squashfs";
        println!(
            "* Step4 - create tool container used for gvmi generation: {} ...",
            tool_image_name
        );

        match docker
            .create_image(
                Some(image::CreateImageOptions {
                    from_image: tool_image_name,
                    tag: "latest",
                    ..Default::default()
                }),
                None,
                None,
            )
            .try_for_each(|ev| async move {
                log::debug!("{:?}", ev);
                Ok(())
            })
            .await
        {
            Ok(_) => {}
            Err(err) => {
                log::error!("Failed to create image: {} {}", tool_image_name, err);
                return Err(anyhow::anyhow!("Failed to create image: {}", err));
            }
        };

        let copy_result: anyhow::Result<_> = async {
            let mut mksquash_command = vec![
                "mksquashfs".to_string(),
                "/work/in".to_string(),
                "/work/out/image.squashfs".to_string(),
                "-info".to_string(),
                "-comp".to_string(),
                self.compression_method.clone(),
                "-noappend".to_string(),
            ];
            if let Some(compression_level) = self.compression_level {
                mksquash_command.push("-Xcompression-level".to_string());
                mksquash_command.push(compression_level.to_string());
            }
            println!(" -- Image command: {}", mksquash_command.join(" "));

            let tool_container = docker
                .create_container::<String, String>(
                    None,
                    container::Config {
                        image: Some(tool_image_name.to_string()),
                        cmd: Some(mksquash_command),
                        attach_stderr: Some(true),
                        attach_stdout: Some(true),
                        host_config: Some(HostConfig {
                            auto_remove: Some(true),
                            mounts: Some(vec![Mount {
                                target: Some("/work/out/image.squashfs".to_string()),
                                source: Some(path.clone()),
                                typ: Some(MountTypeEnum::BIND),
                                ..Default::default()
                            }]),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                )
                .await?;

            println!(" -- Tool container id: {}", &tool_container.id[0..12]);

            println!(
                "* Step5 - copy data between containers {} and {} ...",
                &container_id[0..12],
                &tool_container.id[0..12]
            );

            let input = docker.download_from_container(
                &container_id,
                Some(DownloadFromContainerOptions { path: "/" }),
            );

            let pc = ProgressContext::new();
            let pb = ProgressBar::new(image_size as u64);
            pb.set_style(sty.clone());
            pb.set_message("Copying files from /");
            let input = stream_with_progress(input, &pb, pc.clone());

            docker
                .upload_to_container::<String>(
                    &tool_container.id,
                    Some(UploadToContainerOptions {
                        path: "/work/in".to_owned(),
                        ..Default::default()
                    }),
                    hyper::Body::wrap_stream(input),
                )
                .await?;
            pb.finish_and_clear();
            println!(
                " -- Copying data finished. Copied {} bytes vs {} bytes image",
                pc.total_bytes(),
                image_size
            );
            Ok(tool_container)
        }
        .await;

        docker.remove_container(&container_id, None).await?;
        let tool_container = copy_result?;

        docker
            .start_container::<String>(&tool_container.id, None)
            .await?;
        println!(
            "* Step6 - Starting tool container to create image: {}",
            &tool_container.id[0..12]
        );

        let mp = MultiProgress::new();

        let pg1 = ProgressBar::new(image_size as u64);
        let pg2 = ProgressBar::new(image_size as u64);

        mp.add(pg1.clone());
        mp.add(pg2.clone());

        let sty1 = ProgressStyle::with_template("{wide_bar:.cyan/blue}")
            .unwrap()
            .progress_chars("##-");
        let sty2 = ProgressStyle::with_template("{pos:9}/{len:9} [{wide_msg}]")
            .unwrap()
            .progress_chars("##-");

        pg1.set_style(sty1);
        pg2.set_style(sty2);

        docker
            .logs::<String>(
                &tool_container.id,
                Some(LogsOptions {
                    follow: true,
                    stderr: true,
                    stdout: true,
                    ..Default::default()
                }),
            )
            .try_for_each(|ev| {
                let pg1 = pg1.clone();
                let pg2 = pg2.clone();

                async move {
                    match ev {
                        LogOutput::StdOut { message } => {
                            let message = String::from_utf8(message.to_vec()).unwrap_or_default();
                            if message.contains("uncompressed size") {
                                let mut spl = message.split("uncompressed size");
                                let filename = spl.next();
                                let value = spl
                                    .next()
                                    .unwrap_or("0 bytes")
                                    .split("bytes")
                                    .next()
                                    .unwrap_or("0")
                                    .trim()
                                    .parse::<u64>()
                                    .unwrap_or(0);
                                //log::info!("uncompressed size :: {}", value);
                                if value != 0 {
                                    pg1.inc(value);
                                    pg2.inc(value);
                                    let part = filename.unwrap_or_default();
                                    if part.starts_with("file") {
                                        let mut split = part.split("file");
                                        split.next();
                                        let part2 = split.next().unwrap_or_default();
                                        let part2 = part2.split(',').next().unwrap_or_default();
                                        pg2.set_message(part2.trim().to_string());
                                    }
                                }
                            }
                            //parse to int

                            //log::info!("stdout :: {}", message.trim());
                        }
                        LogOutput::StdErr { message } => {
                            let message = String::from_utf8(message.to_vec()).unwrap_or_default();
                            log::info!("stderr :: {}", message.trim());
                        }
                        _ => {}
                    }
                    Ok(())
                }
            })
            .await?;

        pg1.finish_and_clear();
        pg2.finish_and_clear();
        println!("* Step7 - Waiting for tool container to finish...");
        //tokio::time::sleep(Duration::from_secs(1)).await;
        match docker
            .wait_container::<String>(&tool_container.id, None)
            .try_for_each(|ev| async move {
                log::debug!("end :: {:?}", ev);
                Ok(())
            })
            .await
        {
            Ok(_) => {
                println!(" -- Tool container finished");
            }
            Err(err) => {
                if err.to_string().contains("No such container") {
                    println!(" -- Tool container already removed");
                } else {
                    log::warn!("Failed to wait for tool container: {}", err)
                }
            }
        }

        print!("* Step8 - Adding metadata...");
        //        let hash = cont.id.ok_or(anyhow!("Container has no id"))?;

        let cfg = cont.config.ok_or(anyhow!("Container has no config"))?;

        let mut meta_cfg = cfg.clone();
        //meta_cfg.hostname = Some("".to_string());
        meta_cfg.domainname = Some("".to_string());
        meta_cfg.user = None;

        let bytes = add_metadata_outside(&PathBuf::from(&path), &meta_cfg).await?;
        let conf = read_metadata_outside(&PathBuf::from(&path)).await?;
        log::debug!("conf :: {:?}", conf);
        println!(" -- container metadata ({} bytes) added", bytes);
        let file_length = fs::metadata(&path)?.len();
        if file_length == 0 {
            return Err(anyhow!("Output gvmi image is empty"));
        }
        println!(
            " -- Output gvmi image size: {} ({} bytes), path: {}",
            humansize::format_size(file_length, DECIMAL),
            file_length,
            &path
        );
        Ok(PathBuf::from(&path))
    }
}

/*
pub async fn build_image_int(
    docker: &mut DockerInstance,
    image_name: &str,
    output: &Path,
    env: Vec<String>,
    volumes: Vec<String>,
    entrypoint: Option<String>,
    source_container_name: &str,
    squash_fs_container_name: &str,
) -> anyhow::Result<()> {
    let options = ContainerOptions {
        image_name: image_name.to_owned(),
        container_name: source_container_name.to_owned(),
        mounts: None,
        cmd: None,
        env: if env.is_empty() { None } else { Some(env) },
        volumes: if volumes.is_empty() {
            None
        } else {
            Some(volumes)
        },
        entrypoint: entrypoint.map(|e| vec![e]),
    };

    let spinner = Spinner::new(format!("Starting '{image_name}'")).ticking();
    docker
        .create_container(options)
        .await
        .spinner_result(&spinner)?;
    docker
        .start_container(source_container_name)
        .await
        .spinner_result(&spinner)?;

    let spinner = Spinner::new("Copying image contents".to_string()).ticking();
    let (hash, cfg) = docker
        .get_config(source_container_name)
        .await
        .spinner_err(&spinner)?;

    let options = container::DownloadFromContainerOptions {
        path: "/".to_owned(),
    };
    let stream_in = docker
        .docker
        .download_from_container(source_container_name, Some(options));

    let squashfs_image = "prekucki/squashfs-tools:latest";
    let start_cmd = vec!["tail", "-f", "/dev/null"]; // prevent container from exiting
    let options = ContainerOptions {
        image_name: squashfs_image.to_owned(),
        container_name: squash_fs_container_name.to_owned(),
        mounts: None,
        cmd: Some(start_cmd.iter().map(|s| s.to_string()).collect()),
        env: None,
        volumes: None,
        entrypoint: None,
    };
    docker.create_container(options).await?;
    docker.start_container(squash_fs_container_name).await?;
    let opt = UploadToContainerOptions {
        path: "/work/in".to_owned(),
        no_overwrite_dir_non_dir: "1".to_string(),
    };
    let body = hyper::Body::wrap_stream(stream_in);
    let upload_stream =
        docker
            .docker
            .upload_to_container(squash_fs_container_name, Some(opt), body);
    let res = upload_stream.await?;
    println!("res: {:?}", res);

    let mut tar = tar::Builder::new(RWBuffer::new());

    let progress = Rc::new(Progress::new(
        format!("Building  '{}'", output.display()),
        0,
    ));
    let progress_ = progress.clone();

    async move {
        let mut work_dir = PathBuf::from(&format!("work-{hash}"));
        fs::create_dir_all(&work_dir)?; // path must exist for canonicalize()
        work_dir = work_dir.canonicalize()?;

        let work_dir_out = work_dir.join("out");
        fs::create_dir_all(&work_dir_out)?;

        add_metadata_inside(&mut tar, &cfg)?;
        let squashfs_image_path = repack(
            tar,
            docker,
            &work_dir_out,
            progress_,
            squash_fs_container_name,
        )
        .await?;
        add_metadata_outside(&squashfs_image_path, &cfg)?;

        fs::rename(&squashfs_image_path, output)?;
        fs::remove_dir_all(work_dir_out)?;
        fs::remove_dir_all(work_dir)?;

        Ok::<_, anyhow::Error>(())
    }
    .await
    .progress_result(&progress)?;
    Ok(())
}
*/
/*
pub async fn build_image(
    image_name: &str,
    output: Option<&Path>,
    env: Vec<String>,
    volumes: Vec<String>,
    entrypoint: Option<String>,
) -> anyhow::Result<()> {
    let spinner = Spinner::new(format!("Downloading '{image_name}'")).ticking();
    let mut docker = DockerInstance::new().await.spinner_err(&spinner)?;
    let source_container_name = "source";
    let squash_fs_container_name = "squashfs";

    tokio::select! {
        _ = build_image_int(&mut docker, image_name, output.unwrap(), env, volumes, entrypoint, source_container_name, squash_fs_container_name) => {
        },
        _ = tokio::signal::ctrl_c() => {
            println!("Kill signal received");
        },
    };

    let spinner = Spinner::new(format!("Stopping containers and cleanup")).ticking();
    docker
        .stop_container(source_container_name)
        .await
        .spinner_result(&spinner)?;
    docker
        .remove_container(source_container_name)
        .await
        .spinner_result(&spinner)?;
    docker
        .stop_container(squash_fs_container_name)
        .await
        .spinner_result(&spinner)?;
    docker
        .remove_container(squash_fs_container_name)
        .await
        .spinner_result(&spinner)?;
    Ok(())
}
*/
/*
fn add_file(tar: &mut tar::Builder<RWBuffer>, path: &Path, data: &[u8]) -> anyhow::Result<()> {
    let mut header = tar::Header::new_ustar();
    header.set_path(path)?;
    header.set_size(data.len() as u64);
    header.set_cksum();

    tar.append(&header, data)?;
    Ok(())
}*/
/*
fn add_meta_file(
    tar: &mut tar::Builder<RWBuffer>,
    path: &Path,
    strings: &Option<Vec<String>>,
) -> anyhow::Result<()> {
    log::debug!("Adding metadata file '{}': {:?}", path.display(), strings);
    match strings {
        Some(val) => add_file(tar, path, val.join("\n").as_bytes())?,
        None => add_file(tar, path, &[])?,
    }
    Ok(())
}

fn add_metadata_inside(
    tar: &mut tar::Builder<RWBuffer>,
    config: &ContainerConfig,
) -> anyhow::Result<()> {
    add_meta_file(tar, Path::new(".env"), &config.env)?;
    add_meta_file(tar, Path::new(".entrypoint"), &config.entrypoint)?;
    add_meta_file(tar, Path::new(".cmd"), &config.cmd)?;
    add_meta_file(
        tar,
        Path::new(".working_dir"),
        &config.working_dir.as_ref().map(|s| vec![s.clone()]),
    )?;
    Ok(())
}
*/
/*
async fn add_metadata_outside(
    image_path: &Path,
    config: &ContainerConfig,
) -> anyhow::Result<usize> {
    let mut json_buf = RWBuffer::new();
    serde_json::to_writer(&mut json_buf, config)?;
    let mut file = fs::OpenOptions::new().append(true).open(image_path)?;
    let meta_size = json_buf.bytes.len();
    let crc = {
        //CRC_32_ISO_HDLC is another name of CRC-32-IEEE which was used in previous version of crc
        let crc_algo = Crc::<u32>::new(&CRC_32_ISO_HDLC);
        let mut digest = crc_algo.digest();
        digest.update(&json_buf.bytes);
        digest.finalize()
    };
    let mut bytes_written = file.write(&crc.to_le_bytes())?;
    bytes_written += file.write(&json_buf.bytes)?;
    bytes_written += file.write(format!("{meta_size:08}").as_bytes())?;
    Ok(bytes_written)
}
*/

/*
async fn repack(
    tar: tar::Builder<RWBuffer>,
    docker: &mut DockerInstance,
    dir_out: &Path,
    progress: Rc<Progress>,
    squash_fs_container_name: &str,
) -> anyhow::Result<PathBuf> {
    progress.set_total(9 /* local */ + 100 /* mksquashfs percent */);

    let img_as_tar = finish_tar(tar)?;
    progress.inc(1);

    let path_in = "/work/in";
    let path_out = "/work/out/image.squashfs";

    docker
        .upload(squash_fs_container_name, path_in, img_as_tar.bytes.freeze())
        .await?;
    progress.inc(1);

    let pre_run_progress = progress.position();
    let on_output = |s: String| {
        for s in s.split('\n').filter(|s| s.trim_end().ends_with('%')) {
            if let Some(v) = from_progress_output(s) {
                let delta = v as u64 - (progress.position() - pre_run_progress);
                progress.inc(delta);
            }
        }
    };

    docker
        .run_command(
            squash_fs_container_name,
            vec!["mksquashfs", path_in, path_out, "-comp", "lzo", "-noappend"],
            "/",
            on_output,
        )
        .await?;

    let final_img_tar = docker.download(squash_fs_container_name, path_out).await?;
    progress.inc(1);
    let mut tar = tar::Archive::new(RWBuffer::from(final_img_tar));
    progress.inc(1);
    tar.unpack(dir_out)?;
    progress.inc(1);

    Ok(dir_out.join(Path::new(path_out).file_name().unwrap()))
}
*/
/*
fn tar_from_bytes(bytes: &Bytes) -> anyhow::Result<tar::Builder<RWBuffer>> {
    // the tar builder doesn't have any method for constructing an archive from in-memory
    // representation of the whole file, so we need to do that in chunks

    let buf = RWBuffer::new();
    let mut tar = tar::Builder::new(buf);

    let mut offset: usize = 0;
    loop {
        if offset + 2 * 0x200 > bytes.len() {
            // tar file is terminated by two zeroed chunks
            log::debug!(
                "reading tar: Break at offset 0x{:x}: EOF (incomplete file)",
                offset
            );
            break;
        }

        // check for zeroed chunks (TODO: better way)
        let term = &bytes[offset..offset + 2 * 0x200];
        if term.iter().fold(0, |mut val, b| {
            val |= b;
            val
        }) == 0
        {
            log::debug!("reading tar: Break at offset 0x{:x}: EOF", offset);
            break;
        }

        let hdr = tar::Header::from_byte_slice(&bytes[offset..offset + 0x200]);
        let entry_size = hdr.entry_size()? as usize;
        offset += 0x200;
        tar.append(hdr, &bytes[offset..offset + entry_size])?;
        offset += entry_size;
        if entry_size > 0 && entry_size % 0x200 != 0 {
            // round up to chunk size
            offset |= 0x1ff;
            offset += 1;
        }
    }

    Ok(tar)
}
*/

/*
fn finish_tar(tar: tar::Builder<RWBuffer>) -> anyhow::Result<RWBuffer> {
    let buf = tar.into_inner()?;
    log::debug!("Bytes in tar archive: {}", buf.bytes.len());
    Ok(buf)
}*/
