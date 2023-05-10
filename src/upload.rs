use std::env;

use futures::SinkExt;

use sha3::Digest;
use std::path::Path;

use reqwest::{multipart, Body};

use tokio::fs::File;

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
    }
    Ok(base_url)
}

fn file_to_body(file: File) -> Body {
    use tokio_util::codec::{BytesCodec, FramedRead};
    let stream = FramedRead::new(file, BytesCodec::new());
    let body = Body::wrap_stream(stream);
    body
}

pub async fn push_image(file_path: &Path) -> anyhow::Result<()> {
    //let file_path = PathBuf::from(file_path);

    let repo_url = resolve_repo().await?;
    println!("Uploading image to: {}", repo_url);
    let file = tokio::fs::File::open(&file_path).await?;
    /*
        let file_name = file_path.file_name().ok_or_else(|| anyhow::anyhow!("No filename in path: {}", file_path.display()))?;
        let file_size = file
            .metadata()
            .await?
            .len();



        let (mut tx, rx) = mpsc::channel::<Result<Bytes, awc::error::HttpError>>(1);

        let reader_task = tokio::spawn(async move {
            let mut buf = [0; 1024 * 64];
            let mut reader = BufReader::new(file);

            while let Ok(read) = reader.read(&mut buf[..]).await {
                if read == 0 {
                    break;
                }
                if let Err(e) = tx.send(Ok(Bytes::from(buf[..read].to_vec()))).await {
                    log::error!("Error uploading image: {}", e);
                }
            }
        });
    */
    let descr_endpoint = format!("{repo_url}/v1/image/push/descr").replace("//v1", "/v1");
    println!("Uploading image to: {}", descr_endpoint);
    let client = reqwest::Client::new();
    let form = multipart::Form::new();
    let some_file = multipart::Part::stream(file_to_body(file))
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
