use std::env;

use anyhow::anyhow;
use futures_util::{stream, StreamExt};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::path::{Path, PathBuf};

use reqwest::{multipart, Body};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

use crate::chunks::{FileChunk, FileChunkDesc};
use crate::wrapper::{stream_file_with_progress, ProgressContext};

async fn resolve_repo() -> anyhow::Result<String> {
    Ok(env::var("REPOSITORY_URL").unwrap_or("https://registry.dev.golem.network".to_string()))
    /*
    const PROTOCOL: &str = "http";
    const DOMAIN: &str = "registry.dev.golem.network";
    use trust_dns_resolver::config::{ResolverConfig, ResolverOpts};
    use trust_dns_resolver::TokioAsyncResolver;
            let resolver: TokioAsyncResolver =
                TokioAsyncResolver::tokio(ResolverConfig::google(), ResolverOpts::default())?;

            let lookup = resolver
                .srv_lookup(format!("_girepo._tcp.{DOMAIN}"))
                .await?;
            let srv = lookup
                .iter()
                .next()
                .ok_or_else(|| anyhow::anyhow!("Repository SRV record not found at {DOMAIN}"))?;
            let base_url = format!(
                "{}://{}:{}",
                PROTOCOL,
                srv.target().to_string().trim_end_matches('.'),
                srv.port()
            );
        */
    /*
    let client = awc::Client::new();
    let response = client
        .get(format!("{base_url}/status"))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Repository status check failed: {}", e))?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!(
            "Repository status check failed with code {}",
            response.status().as_u16()
        ));
    }*/
    // Ok(base_url)
}

#[derive(Debug, serde::Deserialize)]
pub struct ValidateUploadResponse {
    pub descriptor: String,
    pub version: Option<u64>,
    pub status: Option<String>,
    pub chunks: Option<Vec<u64>>,
}

pub async fn full_upload(
    path: &Path,
    descr_path: &Path,
    upload_workers: usize,
    username: Option<String>,
    repository: Option<String>,
    tag: Option<String>,
) -> anyhow::Result<()> {
    let vu = validate_upload(descr_path).await?;
    if vu.descriptor != "ok" {
        push_descr(descr_path).await?;
    }
    if let Some(status) = vu.status {
        if status == "full" {
            println!(" -- image already uploaded");
            return Ok(());
        }
    }
    let vu = validate_upload(descr_path).await?;
    if vu.descriptor != "ok" {
        return Err(anyhow!("Failed to register descriptor in repository"));
    }

    if let Some(status) = vu.status {
        if status != "full" {
            push_chunks(path, descr_path, vu.chunks, upload_workers).await?;
        }
    }
    let vu = validate_upload(descr_path).await?;
    if vu.status.unwrap_or_default() != "full" {
        return Err(anyhow!("Failed to validate image upload"));
    }

    if let (Some(username), Some(repository), Some(tag)) = (username, repository, tag) {
        let file_descr_bytes = tokio::fs::read(descr_path).await?;
        let mut sha256 = Sha256::new();
        sha256.update(&file_descr_bytes);
        let descr_sha256 = hex::encode(sha256.finalize());
        let repo_url = resolve_repo().await?;

        let add_tag_endpoint =
            format!("{repo_url}/v1/image/descr/attach/{descr_sha256}").replace("//v1", "/v1");
        let form = multipart::Form::new();
        let form = form.text("tag", tag);
        let form = form.text("username", username);
        let form = form.text("repository", repository);

        let client = reqwest::Client::new();
        let response = client
            .post(add_tag_endpoint)
            .multipart(form)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Repository status check failed: {}", e))?;
    }

    Ok(())
}

pub async fn validate_upload(file_descr: &Path) -> anyhow::Result<ValidateUploadResponse> {
    let file_descr_bytes = tokio::fs::read(file_descr).await?;
    let mut sha256 = Sha256::new();
    sha256.update(&file_descr_bytes);
    let descr_sha256 = hex::encode(sha256.finalize());

    let repo_url = resolve_repo().await?;

    let validate_endpoint =
        format!("{repo_url}/v1/image/descr/{descr_sha256}").replace("//v1", "/v1");
    //println!("Validating image at: {}", validate_endpoint);
    let client = reqwest::Client::new();
    let response = client
        .get(validate_endpoint)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Repository status check failed: {}", e))?;

    let response = response.json::<ValidateUploadResponse>().await?;
    //println!("Response: {:?}", response);
    Ok(response)
}

pub async fn push_descr(file_path: &Path) -> anyhow::Result<()> {
    let repo_url = resolve_repo().await?;
    println!("Uploading image to: {}", repo_url);
    let file_descr_bytes = tokio::fs::read(file_path).await?;
    let mut sha256 = Sha256::new();
    sha256.update(&file_descr_bytes);
    let descr_sha256 = hex::encode(sha256.finalize());

    let descr_endpoint = format!("{repo_url}/v1/image/push/descr").replace("//v1", "/v1");
    println!("Uploading image to: {}", descr_endpoint);
    let client = reqwest::Client::new();
    let form = multipart::Form::new();
    let pc = ProgressContext::new();
    let pb = ProgressBar::new(1);
    let sty = ProgressStyle::with_template("[{msg:20}] {wide_bar:.cyan/blue} {pos:9}/{len:9}")
        .unwrap()
        .progress_chars("##-");

    pb.set_style(sty.clone());

    let file_stream = stream_file_with_progress(file_path, None, &pb, pc).await?;
    let body = Body::wrap_stream(file_stream);
    let some_file = multipart::Part::stream(body)
        .file_name("descriptor.txt")
        .mime_str("application/octet-stream")?;
    let form = form.part("file", some_file);

    let res = client
        .post(descr_endpoint)
        .multipart(form)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Image upload error: {}", e));

    pb.finish_and_clear();
    match res {
        Ok(res) => {
            if res.status().is_success() {
                let resp = res.text().await?;
                //let result = serde_json::from_str(&resp)?;
                println!("Descriptor uploaded successfully {:?}", resp);
                println!("Download link is: {}/download/{}", repo_url, descr_sha256);
            } else {
                return Err(anyhow::anyhow!(
                    "Image upload failed with code {}",
                    res.status().as_u16()
                ));
            }
            Ok(())
        }
        Err(e) => {
            println!("Image upload failed: {}", e);
            Err(e)
        }
    }
}

pub async fn upload_single_chunk(
    file_path: PathBuf,
    chunk: FileChunk,
    //file_descr: FileChunkDesc,
    descr_sha256: String,
    mc: MultiProgress,
    sty: ProgressStyle,
    pb: ProgressBar,
) -> anyhow::Result<()> {
    let repo_url = resolve_repo().await?;
    //println!("Uploading image to: {}", repo_url);
    let descr_endpoint = format!("{repo_url}/v1/image/push/chunk").replace("//v1", "/v1");
    //println!("Uploading image to: {}", descr_endpoint);
    let client = reqwest::Client::new();
    let form = multipart::Form::new();
    let form = form.text("descr-sha256", descr_sha256.clone());
    let form = form.text("chunk-no", chunk.chunk_no.to_string());
    let form = form.text("chunk-sha256", hex::encode(chunk.sha256));
    let form = form.text("chunk-pos", chunk.pos.to_string());
    let form = form.text("chunk-len", chunk.len.to_string());
    let pc = ProgressContext::new();
    let pb_chunk = ProgressBar::new(chunk.len);
    pb_chunk.set_style(sty.clone());
    mc.add(pb_chunk.clone());
    pb_chunk.set_message(format!("Chunk {}", chunk.chunk_no + 1,));

    let chunk_stream = stream_file_with_progress(
        &file_path,
        Some(std::ops::Range {
            start: chunk.pos as usize,
            end: (chunk.pos + chunk.len) as usize,
        }),
        &pb_chunk,
        pc,
    )
    .await?;
    let body = Body::wrap_stream(chunk_stream);
    let some_file = multipart::Part::stream(body)
        .file_name("descriptor.txt")
        .mime_str("application/octet-stream")?;

    let form = form.part("file", some_file);

    let res = client
        .post(descr_endpoint)
        .multipart(form)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Image upload error: {}", e));

    mc.remove(&pb_chunk);
    match res {
        Ok(res) => {
            if res.status().is_success() {
                pb.inc(1);
                //println!("Image uploaded successfully");
                Ok(())
            } else {
                let status = res.status();

                Err(anyhow::anyhow!(
                    "Image upload failed with code {}",
                    status.as_u16()
                ))
            }
        }
        Err(e) => Err(e),
    }
}

pub async fn push_chunks(
    file_path: &Path,
    file_descr: &Path,
    uploaded_chunks: Option<Vec<u64>>,
    upload_workers: usize,
) -> anyhow::Result<()> {
    let file_descr_bytes = tokio::fs::read(file_descr).await?;
    let mut sha256 = Sha256::new();
    sha256.update(&file_descr_bytes);
    let descr_sha256 = hex::encode(sha256.finalize());
    let file_descr = FileChunkDesc::deserialize_from_bytes(&file_descr_bytes)?;
    {
        //check if file readable and close immediately
        //it's easier to check now than later in stream wrapper
        let mut file = tokio::fs::File::open(&file_path).await.map_err(|e| {
            anyhow!(
                "File not found or cannot be opened: {} {e:?}",
                file_path.display()
            )
        })?;
        file.read_i8()
            .await
            .map_err(|e| anyhow!("File not readable: {} {e:?}", file_path.display()))?;
    }
    let chunks_to_upload = if let Some(uploaded_chunks) = uploaded_chunks {
        let mut chunks = Vec::<FileChunk>::new();
        for f in file_descr.chunks {
            let is_uploaded = *uploaded_chunks
                .get(f.chunk_no as usize)
                .ok_or(anyhow!("Chunk number {} is out of bounds", f.chunk_no))?;
            if is_uploaded == 1 {
                //println!("Chunk {} already uploaded, skipping", f.chunk_no);
                log::debug!("Chunk {} already uploaded, skipping", f.chunk_no);
                continue;
            } else {
                chunks.push(f);
            }
        }
        chunks
    } else {
        file_descr.chunks
    };

    let sty = ProgressStyle::with_template("[{msg:20}] {wide_bar:.cyan/blue} {pos:9}/{len:9}")
        .unwrap()
        .progress_chars("##-");
    let mc = MultiProgress::new();
    let pb = ProgressBar::new(chunks_to_upload.len() as u64);
    pb.set_style(sty.clone());
    mc.add(pb.clone());

    pb.set_message("Chunked upload");

    let mut futures = stream::iter(chunks_to_upload.iter().map(|chunk| {
        tokio::spawn(upload_single_chunk(
            PathBuf::from(file_path),
            chunk.clone(),
            descr_sha256.clone(),
            mc.clone(),
            sty.clone(),
            pb.clone(),
        ))
    }))
    .buffer_unordered(upload_workers);
    while let Some(fut) = futures.next().await {
        match fut {
            Ok(join_res) => match join_res {
                Ok(_res) => {}
                Err(e) => {
                    return Err(e);
                }
            },
            Err(e) => {
                println!("Image upload failed: {:?}", e);
                return Err(anyhow!("Image upload failed: {:?}", e));
            }
        }
    }
    Ok(())
}
/*
pub async fn upload_image<P: AsRef<Path>>(file_path: P) -> anyhow::Result<()> {
    let file_path = file_path.as_ref();
    let progress = Arc::new(Progress::with_eta(
        format!("Uploading '{}'", file_path.display()),
        0,
    ));

    let file = tokio::fs::File::open(&file_path)
        .await
        .with_context(|| format!("Failed to open file: {}", file_path.display()))
        .progress_err(&progress)?;
    let file_name = file_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("No filename in path: {}", file_path.display()))
        .progress_err(&progress)?
        .to_string_lossy()
        .to_string();
    let file_size = file
        .metadata()
        .await
        .with_context(|| format!("Failed to retrieve file metadata: {}", file_path.display()))
        .progress_err(&progress)?
        .len();

    (*progress).set_total(file_size);

    let repo_url = resolve_repo().await.progress_err(&progress)?;
    log::debug!("Repository URL: {}", repo_url);

    let (mut tx, rx) = mpsc::channel::<Result<Bytes, awc::error::HttpError>>(1);
    let (htx, hrx) = oneshot::channel();

    let progress_ = progress.clone();
    tokio::spawn(async move {
        let mut buf = [0; 1024 * 64];
        let mut reader = BufReader::new(file);
        let mut hasher = sha3::Sha3_224::new();

        while let Ok(read) = reader.read(&mut buf[..]).await {
            if read == 0 {
                break;
            }
            if let Err(e) = tx.send(Ok(Bytes::from(buf[..read].to_vec()))).await {
                log::error!("Error uploading image: {}", e);
            }
            hasher.update(&buf[..read]);
            progress_.inc(read as u64);
        }

        if let Err(e) = htx.send(hasher.finalize().encode_hex()) {
            log::error!("Error during hash finalization: {}", e);
        }
    });

    let client = awc::Client::builder().disable_timeout().finish();
    client
        .put(format!("{repo_url}/upload/{file_name}"))
        .send_stream(rx)
        .await
        .map_err(|e| anyhow::anyhow!("Image upload error: {}", e))
        .progress_result(&progress)?;

    let hash: String = hrx.await?;
    let bytes = format!("{repo_url}/{file_name}").as_bytes().to_vec();
    let spinner = Spinner::new(format!("Uploading link for {file_name}")).ticking();
    client
        .put(format!("{repo_url}/upload/image.{hash}.link"))
        .send_body(bytes)
        .await
        .map_err(|e| anyhow::anyhow!("Image descriptor upload error: {e}"))
        .spinner_result(&spinner)?;

    println!("{hash}");
    Ok(())
}*/
