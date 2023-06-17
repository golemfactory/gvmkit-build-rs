use std::collections::VecDeque;
use std::env;

use anyhow::anyhow;
use futures_util::{stream, StreamExt};
use humansize::DECIMAL;
use indicatif::{MultiProgress, ProgressBar};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use reqwest::{multipart, Body};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

use crate::chunks::{FileChunk, FileChunkDesc};
use crate::image::ImageName;
use crate::progress::{create_chunk_pb, ProgressBarType};
use crate::wrapper::stream_file_with_progress;

async fn loads_bytes_and_sha(descr_path: &Path) -> anyhow::Result<(Vec<u8>, String)> {
    let file_descr_bytes = tokio::fs::read(descr_path).await?;
    let mut sha256 = Sha256::new();
    sha256.update(&file_descr_bytes);
    let descr_sha256 = hex::encode(sha256.finalize());
    Ok((file_descr_bytes, descr_sha256))
}

pub async fn resolve_repo() -> anyhow::Result<String> {
    Ok(env::var("REGISTRY_URL").unwrap_or("https://registry.golem.network".to_string()))
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

pub async fn check_login(user_name: &str, pat: &str) -> anyhow::Result<bool> {
    println!(" * Checking credentials for {}...", user_name);
    let repo_url = resolve_repo().await?;

    let check_login_endpoint = format!("{repo_url}/auth/pat/login").replace("//auth", "/auth");
    //println!("Validating image at: {}", validate_endpoint);

    let post_data = json!(
        {
            "username": user_name,
            "password": pat,
        }
    );
    let client = reqwest::Client::new();
    let response = client
        .post(check_login_endpoint)
        .json(&post_data)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Repository status check failed: {}", e))?;

    let response_status = response.status();
    match response_status {
        reqwest::StatusCode::OK => {
            println!(" -- successfully logged in");
            Ok(true)
        }
        reqwest::StatusCode::UNAUTHORIZED => {
            let text = response.text().await.unwrap_or_default();
            println!(" -- failed to log in: {}", text);
            Ok(false)
        }
        _ => {
            let text = response.text().await.unwrap_or_default();
            Err(anyhow::anyhow!(
                "Other error when checking login, status: {}, err: {}",
                response_status,
                text
            ))
        }
    }
}

pub async fn attach_to_repo(
    descr_path: &Path,
    image_name: &ImageName,
    login: &str,
    pat: &str,
    check: bool,
) -> anyhow::Result<()> {
    if image_name.user.is_none() {
        return Err(anyhow::anyhow!("Image name must contain user"));
    }
    let (_, descr_sha256) = loads_bytes_and_sha(descr_path).await?;
    let repo_url = resolve_repo().await?;

    let add_tag_endpoint =
        format!("{repo_url}/v1/image/descr/attach/{descr_sha256}").replace("//v1", "/v1");
    let form = multipart::Form::new();
    let form = form.text("tag", image_name.tag.clone());
    let form = form.text("username", image_name.user.clone().unwrap());
    let form = form.text("repository", image_name.repository.clone());
    let form = form.text("login", login.to_string());
    let form = form.text("token", pat.to_string());
    let form = if check {
        form.text("check", "true")
    } else {
        form
    };

    if check {
        println!(
            " * Checking if image can be added to repository: {}",
            image_name.to_normalized_name()
        );
    } else {
        println!(
            " * Adding image to repository: {}",
            image_name.to_normalized_name()
        );
    }
    let client = reqwest::Client::new();
    let response = client
        .post(add_tag_endpoint)
        .multipart(form)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Repository status check failed: {}", e))?;

    if response.status() != 200 {
        return match response.text().await {
            Ok(text) => Err(anyhow::anyhow!(
                "Not possible to add to repository: {}",
                text
            )),
            Err(e) => Err(anyhow::anyhow!("Not possible to add to repository: {}", e)),
        };
    } else {
        let text = response.text().await?;
        if check {
            println!(" -- checked successfully");
        } else {
            println!(" -- success: {}", text);
        }
    }
    Ok(())
}

//returns if full upload is needed
pub async fn upload_descriptor(descr_path: &Path) -> anyhow::Result<bool> {
    let repo_url = resolve_repo().await?;
    let vu = validate_upload(descr_path).await?;
    if vu.descriptor != "ok" {
        //upload descriptor if not found
        push_descr(descr_path).await?;
    } else {
        let (_, descr_sha256) = loads_bytes_and_sha(descr_path).await?;
        println!(" -- descriptor already uploaded");
        println!(" -- download link: {}/download/{}", repo_url, descr_sha256);
    }
    if let Some(status) = vu.status {
        if status == "full" {
            println!(" -- image already uploaded");
            return Ok(false);
        }
    }

    Ok(true)
}

pub async fn full_upload(
    path: &Path,
    descr_path: &Path,
    upload_workers: usize,
) -> anyhow::Result<()> {
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
    } else {
        println!(" -- image validated successfully");
    }

    Ok(())
}

pub async fn validate_upload(file_descr: &Path) -> anyhow::Result<ValidateUploadResponse> {
    let (_, descr_sha256) = loads_bytes_and_sha(file_descr).await?;

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
    Ok(response)
}

pub async fn push_descr(file_path: &Path) -> anyhow::Result<()> {
    let repo_url = resolve_repo().await?;
    println!(" * Uploading image descriptor to: {}", repo_url);
    let (_, descr_sha256) = loads_bytes_and_sha(file_path).await?;

    let descr_endpoint = format!("{repo_url}/v1/image/push/descr").replace("//v1", "/v1");
    let client = reqwest::Client::new();
    let form = multipart::Form::new();
    let pb = create_chunk_pb(1, ProgressBarType::DescriptorUpload);

    let file_stream =
        stream_file_with_progress(file_path, None, Some(pb.clone()), None, None).await?;
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
                println!(" -- descriptor uploaded successfully");
                println!(" -- download link: {}/download/{}", repo_url, descr_sha256);
            } else {
                return Err(anyhow::anyhow!(
                    "Image upload failed with code {}: {}",
                    res.status().as_u16(),
                    res.text().await.unwrap_or_default()
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
    pb_chunks: ProgressBar,
    pb_details: ProgressBar,
    pb_total: ProgressBar,
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
    let pb_chunk = create_chunk_pb(chunk.len, ProgressBarType::SingleChunk);
    if !pb_chunk.is_hidden() {
        mc.add(pb_chunk.clone());
    }
    pb_chunk.set_message(format!("Chunk {}", chunk.chunk_no + 1,));

    let chunk_stream = stream_file_with_progress(
        &file_path,
        Some(std::ops::Range {
            start: chunk.pos as usize,
            end: (chunk.pos + chunk.len) as usize,
        }),
        Some(pb_chunk.clone()),
        Some(pb_details.clone()),
        Some(pb_total.clone()),
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
                pb_chunks.inc(1);
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
    let (file_descr_bytes, descr_sha256) = loads_bytes_and_sha(file_descr).await?;
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
    let total_chunk_length = file_descr.chunks.len();

    let mc = MultiProgress::new();
    let pb_total = create_chunk_pb(file_descr.size, ProgressBarType::UploadTotal);
    let pb_details = create_chunk_pb(file_descr.size, ProgressBarType::UploadDetails);
    let pb_chunks = create_chunk_pb(total_chunk_length as u64, ProgressBarType::UploadChunks);
    if !pb_total.is_hidden() {
        mc.add(pb_total.clone());
    }
    if !pb_details.is_hidden() {
        mc.add(pb_details.clone());
    }
    if !pb_chunks.is_hidden() {
        mc.add(pb_chunks.clone());
    }

    let chunks_to_upload = if let Some(uploaded_chunks) = uploaded_chunks {
        let mut chunks = Vec::<FileChunk>::new();
        for f in file_descr.chunks {
            let is_uploaded = *uploaded_chunks
                .get(f.chunk_no as usize)
                .ok_or(anyhow!("Chunk number {} is out of bounds", f.chunk_no))?;
            if is_uploaded == 1 {
                pb_chunks.inc(1);
                pb_details.inc(f.len);
                pb_total.inc(f.len);
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

    pb_chunks.set_message("Chunked upload");
    pb_total.set_message("Total upload");

    let upload_speed = tokio::spawn({
        let pb_details = pb_details.clone();
        async move {
            let mut ticks = VecDeque::<u64>::new();
            let mut instant = Instant::now();
            let total_start = Instant::now();
            let total_start_pos = pb_details.position();
            let mut loop_no = 0_u64;
            pb_details.set_message("Upload speed: NA, Total speed: NA, ETA: NA");
            loop {
                ticks.push_front(pb_details.position());
                if ticks.len() > 11 {
                    ticks.pop_back();
                }

                if ticks.len() > 1 {
                    let speed =
                        (ticks[0] - ticks[ticks.len() - 1]) as f64 / (ticks.len() - 1) as f64;
                    let total_speed = (pb_details.position() - total_start_pos) as f64
                        / total_start.elapsed().as_secs_f64();
                    let eta_str = if speed > 100.0 {
                        let sec_left = (pb_details.length().unwrap_or(1) - pb_details.position())
                            as f64
                            / speed;
                        if sec_left > 0.0 {
                            humantime::format_duration(Duration::from_secs(sec_left as u64))
                                .to_string()
                        } else {
                            "NA".to_string()
                        }
                    } else {
                        "NA".to_string()
                    };
                    pb_details.set_message(format!(
                        "Upload speed: {}/s, Total speed: {}/s, ETA: {}",
                        humansize::format_size(speed as u64, DECIMAL),
                        humansize::format_size(total_speed as u64, DECIMAL),
                        eta_str
                    ));
                }

                //log::error!("{}", pb_details.position());

                let elapsed = instant.elapsed().as_secs_f64();
                loop_no += 1;
                let target_elapsed = loop_no as f64;
                let sleep_time = target_elapsed - elapsed;
                if sleep_time < 0.0 {
                    //something went wrong (probably sleep or hang)
                    instant = Instant::now();
                    loop_no = 0;
                    ticks.clear();
                    continue;
                }
                tokio::time::sleep(Duration::from_secs_f64(sleep_time)).await;
            }
        }
    });

    let mut futures = stream::iter(chunks_to_upload.iter().map(|chunk| {
        tokio::spawn(upload_single_chunk(
            PathBuf::from(file_path),
            chunk.clone(),
            descr_sha256.clone(),
            mc.clone(),
            pb_chunks.clone(),
            pb_details.clone(),
            pb_total.clone(),
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
                log::error!("Image upload failed: {:?}", e);
                return Err(anyhow!("Image upload failed: {:?}", e));
            }
        }
    }
    //stop task that updates upload speed
    upload_speed.abort();
    pb_chunks.finish_and_clear();
    pb_details.finish_and_clear();
    pb_total.finish_and_clear();
    mc.remove(&pb_chunks);
    mc.remove(&pb_details);
    mc.remove(&pb_total);

    println!(" -- chunked upload finished successfully");
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
