use anyhow::anyhow;
use bollard::{
    container, exec, image,
    service::{ContainerConfig, HostConfig, Mount, MountTypeEnum},
    Docker,
};
use bytes::{BufMut, Bytes, BytesMut};
use futures::TryStreamExt;

use futures_util::StreamExt;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct DirectoryMount {
    pub host: String,
    pub guest: String,
    pub readonly: bool,
}

#[derive(Debug, Clone)]
pub struct ContainerOptions {
    pub image_name: String,
    pub container_name: String,
    pub mounts: Option<Vec<DirectoryMount>>,
    pub cmd: Option<Vec<String>>,
    pub env: Option<Vec<String>>,
    pub volumes: Option<Vec<String>>,
    pub entrypoint: Option<Vec<String>>,
}

impl Default for ContainerOptions {
    fn default() -> Self {
        ContainerOptions {
            image_name: "".to_owned(),
            container_name: "".to_owned(),
            mounts: None,
            cmd: None,
            env: None,
            volumes: None,
            entrypoint: None,
        }
    }
}
/*
pub struct DockerInstance {
    pub docker: Docker,
}

impl DockerInstance {
    pub async fn new() -> anyhow::Result<DockerInstance> {
        Ok(DockerInstance {
            docker: Docker::connect_with_local_defaults()?,
        })
    }

    pub async fn create_image(&mut self, image_name: &str) -> anyhow::Result<()> {
        log::debug!("Pulling image '{}'", image_name);
        let options = image::CreateImageOptions {
            from_image: image_name,
            ..Default::default()
        };

        self.docker
            .create_image(Some(options), None, None)
            .try_collect::<Vec<_>>()
            .await?;
        Ok(())
    }

    pub async fn try_create_container(&mut self, options: ContainerOptions) -> anyhow::Result<()> {
        let create_options = container::CreateContainerOptions {
            platform: Some("linux/amd64".to_owned()),
            name: options.container_name,
        };

        let host_config = HostConfig {
            mounts: options.mounts.map(|mut mounts| {
                mounts
                    .drain(..)
                    .map(|mount| Mount {
                        target: Some(mount.guest),
                        source: Some(mount.host),
                        read_only: Some(mount.readonly),
                        typ: Some(MountTypeEnum::BIND),
                        ..Default::default()
                    })
                    .collect()
            }),
            ..Default::default()
        };

        let mut vols = HashMap::new();
        if let Some(volumes) = options.volumes {
            volumes.iter().for_each(|v| {
                vols.insert(v.into(), HashMap::new());
            });
        }

        let config = container::Config {
            cmd: options.cmd,
            env: options.env,
            volumes: Some(vols),
            entrypoint: options.entrypoint,
            image: Some(options.image_name),
            host_config: Some(host_config),
            ..Default::default()
        };

        self.docker
            .create_container(Some(create_options), config)
            .await?;
        Ok(())
    }

    pub async fn create_container(&mut self, options: ContainerOptions) -> anyhow::Result<()> {
        match self.try_create_container(options.clone()).await {
            Ok(_) => (),
            Err(err) => {
                if err.to_string().contains("No such image") {
                    // TODO: better way
                    self.create_image(&options.image_name).await?;
                    self.try_create_container(options).await?;
                } else {
                    return Err(err);
                }
            }
        }
        Ok(())
    }

    pub async fn start_container(&mut self, container_name: &str) -> anyhow::Result<()> {
        log::debug!("Starting container '{}'", container_name);
        let options = None::<container::StartContainerOptions<String>>;
        self.docker.start_container(container_name, options).await?;
        Ok(())
    }

    pub async fn run_command<F: Fn(String)>(
        &mut self,
        container_name: &str,
        cmd: Vec<&str>,
        dir: &str,
        on_output: F,
    ) -> anyhow::Result<()> {
        log::debug!("Running '{:?}' in container '{}'", cmd, container_name);
        let config = exec::CreateExecOptions {
            cmd: Some(cmd),
            working_dir: Some(dir),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            ..Default::default()
        };

        let result = self.docker.create_exec(container_name, config).await?;
        match self
            .docker
            .start_exec(
                &result.id,
                Some(exec::StartExecOptions {
                    detach: false,
                    output_capacity: None,
                }),
            )
            .await
        {
            Ok(start_exec_results) => match start_exec_results {
                exec::StartExecResults::Attached {
                    mut output,
                    input: _,
                } => loop {
                    match output.next().await {
                        Some(Ok(stream)) => {
                            log::info!("Output: {}", stream.to_string());
                            on_output(stream.to_string());
                        }
                        Some(Err(err)) => {
                            return Err(anyhow!("Failed to read exec output: {}", err));
                        }
                        None => break,
                    }
                },
                exec::StartExecResults::Detached => (),
            },
            Err(err) => {
                return Err(anyhow!("Failed to start exec: {}", err));
            }
        }

        Ok(())
    }

    pub async fn stop_container(&mut self, container_name: &str) -> anyhow::Result<()> {
        log::debug!("Stopping container '{}'", container_name);
        self.docker
            .stop_container(container_name, None::<container::StopContainerOptions>)
            .await?;
        Ok(())
    }

    pub async fn remove_container(&mut self, container_name: &str) -> anyhow::Result<()> {
        log::debug!("Removing container '{}'", container_name);
        let options = container::RemoveContainerOptions {
            v: true,
            force: true,
            ..Default::default()
        };

        self.docker
            .remove_container(container_name, Some(options))
            .await?;
        Ok(())
    }

    pub async fn download(&mut self, container_name: &str, path: &str) -> anyhow::Result<BytesMut> {
        log::debug!("Downloading '{}' from container '{}'", path, container_name);

        let options = container::DownloadFromContainerOptions { path };
        let bytes = self
            .docker
            .download_from_container(container_name, Some(options))
            .try_fold(BytesMut::new(), |mut bytes, b| async move {
                bytes.put(b);
                Ok(bytes)
            })
            .await?;

        Ok(bytes)
    }

    pub async fn upload(
        &mut self,
        container_name: &str,
        path: &str,
        data: Bytes,
    ) -> anyhow::Result<()> {
        log::debug!("Uploading to '{}' in container '{}'", path, container_name);

        let options = container::UploadToContainerOptions {
            path,
            ..Default::default()
        };
        self.docker
            .upload_to_container(container_name, Some(options), data.into())
            .await?;
        Ok(())
    }

    pub async fn get_config(
        &mut self,
        container_name: &str,
    ) -> anyhow::Result<(String, ContainerConfig)> {
        let cont = self
            .docker
            .inspect_container(container_name, None::<container::InspectContainerOptions>)
            .await?;

        let hash = cont.id.ok_or(anyhow!("Container has no id"))?;
        log::debug!("Container ID: {}", &hash);

        let cfg = cont.config.ok_or(anyhow!("Container has no config"))?;
        Ok((hash, cfg))
    }
}
*/
