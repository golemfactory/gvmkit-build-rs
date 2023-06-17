# gvmkit-build

Golem VM Image builder used as companion app for Golem Registry: https://registry.golem.network

# Requirements

Running docker engine is required. Tool supports Linux, Windows and macOS.
Note that when using macOS ARM version use --platform linux/amd64 option for docker builds.

# Images

Golem Network is using gvmi images as base for creating VMs for tasks.
These images are basically squashfs images with some additional metadata.
They can be prepared from docker images using this (gvmkit-build) tool.

# naming image

The tool as main argument is taking docker image name.

Docker Image id is composed maximum of 3 parts:
<username>/<image_name>:<tag>
It can be also for example
<image_name>

Following command will build image and create *.gvmi file in current directory
```
gvmkit-build <docker_image_id>
```

If image not exist locally the tool is trying to pull it from docker hub.

To successfully add image to registry portal you have to name image accordingly or use
```
gvmkit-build <user_name><image_name>/<tag> --push
```
or if your local image name is not compatible use
```
gvmkit-build <docker_image_id> --push-to <user_name><image_name>/<tag>
```


# troubleshooting login to registry portal

The tool is using https://registry.golem.network as default registry portal.
You can change this behaviour by setting `REGISTRY_URL` environment variable.

The tool will ask for login when --login --push or --push-to option is specified.

For storing login information rpassword library is used: https://docs.rs/rpassword/7.2.0/rpassword/

Only one instance of login/token is kept saved at "gvmkit-build-rs/default". So if you login with new user/token old pair will be forgotten.

This option will ask for login to registry portal
```
gvmkit-build --login
```
You can use command for check if your login information is correct
```
gvmkit-build --login-check 
```
If you want to forget your login information you can use, it will clear your login information
```
gvmkit-build --logout 
```

Above command are optional, because you will be asked for login automatically if you use --push or --push-to option.

On some systems when no secure store is provided you will be forced to use following method of keeping login/token pair.
You can optionally put them in .env file from convenience in your working directory.
Remember to set proper permissions to the file (chmod 600 .env) if using on shared machine.

```
REGISTRY_USER=<username-in-registry-portal>
REGISTRY_TOKEN=<access-token-generated-in-registry-portal>
```

When REGISTRY_USER/REGISTRY_TOKEN is set in environment variables, secure rpassword storage won't be used.

# uploading to multiple tags

The tool cannot upload to multiple tags, but you can call it multiple times with different tags.
All steps of operations are cached so no worry about reuploading same file multiple times.

# uploading large files

The tool is using chunked upload with default chunk size 10MB (changing not recommended).
4 upload workers are created by default. If you think your upload is stuck you
can always stop and run tool again to finish download. Only chunks that were not uploaded
will be uploaded again.

Note: Total limit of chunks is set to 1000 (so 10GB by default). If you want to upload larger file you have to set greater chunk size accordingly.

# uploading image without login

You can upload images anonymously. Note that lifetime of such images is limited, 
and they can be removed from registry portal after some time without notice

```
cargo run --release -- <image_name> --push --nologin
```

# changing squashfs options when creating image

You can change compression used in mskquashfs to produce more or less compact images. 

Look for help for more information
```
gvmkit-build --help
```

