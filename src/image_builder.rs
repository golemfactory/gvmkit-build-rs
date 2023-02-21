use bytes::{Bytes, BytesMut};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use crate::docker::{ContainerOptions, DockerInstance};
use crate::progress::{from_progress_output, Progress, ProgressResult, Spinner, SpinnerResult};
use crate::rwbuf::RWBuffer;
use awc::error::SendRequestError::Http;
use bollard::container;
use bollard::container::UploadToContainerOptions;
use bollard::service::ContainerConfig;
use crc::{Crc, CRC_32_ISO_HDLC};
use std::rc::Rc;

pub(crate) const STEPS: usize = 4;

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
    let mut stream_in = docker
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

pub async fn build_image(
    image_name: &str,
    output: &Path,
    env: Vec<String>,
    volumes: Vec<String>,
    entrypoint: Option<String>,
) -> anyhow::Result<()> {
    let spinner = Spinner::new(format!("Downloading '{image_name}'")).ticking();
    let mut docker = DockerInstance::new().await.spinner_err(&spinner)?;
    let source_container_name = "source";
    let squash_fs_container_name = "squashfs";

    tokio::select! {
        _ = build_image_int(&mut docker, image_name, output, env, volumes, entrypoint, source_container_name, squash_fs_container_name) => {
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

fn add_file(tar: &mut tar::Builder<RWBuffer>, path: &Path, data: &[u8]) -> anyhow::Result<()> {
    let mut header = tar::Header::new_ustar();
    header.set_path(path)?;
    header.set_size(data.len() as u64);
    header.set_cksum();

    tar.append(&header, data)?;
    Ok(())
}

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

fn add_metadata_outside(image_path: &Path, config: &ContainerConfig) -> anyhow::Result<()> {
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
    let mut _bytes_written = file.write(&crc.to_le_bytes())?;
    _bytes_written += file.write(&json_buf.bytes)?;
    _bytes_written += file.write(format!("{meta_size:08}").as_bytes())?;
    Ok(())
}

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

fn finish_tar(tar: tar::Builder<RWBuffer>) -> anyhow::Result<RWBuffer> {
    let buf = tar.into_inner()?;
    log::debug!("Bytes in tar archive: {}", buf.bytes.len());
    Ok(buf)
}
