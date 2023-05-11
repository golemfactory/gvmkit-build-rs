use std::env;

use anyhow::anyhow;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::Path;

use reqwest::{multipart, Body};
use tokio::io::AsyncReadExt;

use crate::chunks::FileChunkDesc;
use crate::wrapper::{stream_file_with_progress, ProgressContext};
use trust_dns_resolver::config::{ResolverConfig, ResolverOpts};
use trust_dns_resolver::TokioAsyncResolver;

const PROTOCOL: &str = "http";
const DOMAIN: &str = "dev.golem.network";

async fn resolve_repo() -> anyhow::Result<String> {
    if let Ok(repository_url) = env::var("REPOSITORY_URL") {
        return Ok(repository_url);
    }
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
    Ok(base_url)
}

pub async fn push_descr(file_path: &Path) -> anyhow::Result<()> {
    let repo_url = resolve_repo().await?;
    println!("Uploading image to: {}", repo_url);
    {
        //check if file readable and close immediately
        //it's easier to check now than later in stream wrapper
        let mut file = tokio::fs::File::open(&file_path).await.map_err(|e| {
            anyhow!(
                "Descriptor not found or cannot be opened: {} {e:?}",
                file_path.display()
            )
        })?;
        file.read_i8()
            .await
            .map_err(|e| anyhow!("Descriptor not readable: {} {e:?}", file_path.display()))?;
    }

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

    match res {
        Ok(res) => {
            if res.status().is_success() {
                println!("Image uploaded successfully");
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

pub async fn push_chunks(file_path: &Path, file_descr: &Path) -> anyhow::Result<()> {
    let file_descr = tokio::fs::read(file_descr).await?;
    let file_descr = FileChunkDesc::deserialize_from_bytes(&file_descr)?;
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

    for (chunk_no, chunk) in file_descr.chunks.iter().enumerate() {
        let repo_url = resolve_repo().await?;
        println!("Uploading image to: {}", repo_url);
        let descr_endpoint = format!("{repo_url}/v1/image/push/chunk").replace("//v1", "/v1");
        println!("Uploading image to: {}", descr_endpoint);
        let client = reqwest::Client::new();
        let form = multipart::Form::new();
        let form = form.text("chunk-no", chunk_no.to_string());
        let form = form.text("chunk-sha", hex::encode(chunk.sha256).to_string());
        let form = form.text("chunk-pos", chunk.pos.to_string());
        let form = form.text("chunk-len", chunk.len.to_string());
        let pc = ProgressContext::new();
        let pb = ProgressBar::new(1);
        let sty = ProgressStyle::with_template("[{msg:20}] {wide_bar:.cyan/blue} {pos:9}/{len:9}")
            .unwrap()
            .progress_chars("##-");

        pb.set_style(sty.clone());
        let chunk_stream = stream_file_with_progress(
            file_path,
            Some(std::ops::Range {
                start: chunk.pos as usize,
                end: (chunk.pos + chunk.len) as usize,
            }),
            &pb,
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

        match res {
            Ok(res) => {
                if res.status().is_success() {
                    println!("Image uploaded successfully");
                } else {
                    let status = res.status();
                    log::error!(
                        "Image upload failed with code {}",
                        res.text().await.unwrap_or("".to_string())
                    );
                    return Err(anyhow::anyhow!(
                        "Image upload failed with code {}",
                        status.as_u16()
                    ));
                }
            }
            Err(e) => {
                println!("Image upload failed: {}", e);
                return Err(e);
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
