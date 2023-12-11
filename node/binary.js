// this example uses the binary generated by the rust project in the parent directory
// said project is released on GitHub, and the correct URL is constructed based on
// the target operating system and the version in package.json

// your binary could be downloaded from any URL and could use any logic you want
// to construct said URL. You could even A/B test two different binary distribution
// solutions!

const { existsSync, mkdirSync } = require("fs");
const axios = require("axios");
const tar = require("tar");
const unzipper = require("unzipper");
const rimraf = require("rimraf");

const { Binary } = require("@cloudflare/binary-install");
const os = require("os");
const { join } = require("path");
const cTable = require("console.table");

const error = (msg) => {
  console.error(msg);
  process.exit(1);
};

const { version, name, repository } = require("./package.json");

class GvmKitBuildBinary extends Binary {
  install() {
    const dir = this._getInstallDirectory();
    if (!existsSync(dir)) {
      mkdirSync(dir, { recursive: true });
    }

    this.binaryDirectory = join(dir, "bin");

    if (existsSync(this.binaryDirectory)) {
      rimraf.sync(this.binaryDirectory);
    }

    mkdirSync(this.binaryDirectory, { recursive: true });

    console.log(`Downloading release from ${this.url}`);
    const is_zip = this.url.endsWith(".zip");

    return axios({ url: this.url, responseType: "stream" })
      .then(async (res) => {
        if (is_zip) {
          await res.data.pipe(unzipper.Extract({ path: this.binaryDirectory }));
        } else {
          await res.data.pipe(tar.x({ C: this.binaryDirectory }));
        }
      })
      .then(() => {
        console.log(
          `${this.name ? this.name : "Your package"} has been installed!`
        );
      })
      .catch((e) => {
        error(`Error fetching release: ${e.message}`);
      });
  }
}

const supportedPlatforms = [
  {
    TYPE: "Windows_NT",
    ARCHITECTURE: "x64",
    RUST_TARGET: "x86_64-pc-windows-msvc",
    PACKING: "zip",
  },
  {
    TYPE: "Linux",
    ARCHITECTURE: "x64",
    RUST_TARGET: "x86_64-unknown-linux-musl",
    PACKING: "tar.gz",
  },
  {
    TYPE: "Darwin",
    ARCHITECTURE: "x64",
    RUST_TARGET: "x86_64-apple-darwin",
    PACKING: "tar.gz",
  },
  {
    TYPE: "Darwin",
    ARCHITECTURE: "arm64",
    RUST_TARGET: "aarch64-apple-darwin",
    PACKING: "tar.gz",
  },
];

const getPlatform = () => {
  const type = os.type();
  const architecture = os.arch();

  for (let index in supportedPlatforms) {
    let supportedPlatform = supportedPlatforms[index];
    if (
      type === supportedPlatform.TYPE &&
      architecture === supportedPlatform.ARCHITECTURE
    ) {
      return supportedPlatform;
    }
  }

  error(
    `Platform with type "${type}" and architecture "${architecture}" is not supported by ${name}.\nYour system must be one of the following:\n\n${cTable.getTable(
      supportedPlatforms
    )}`
  );
};

const getBinary = () => {
  const platform = getPlatform();
  // the url for this binary is constructed from values in `package.json`
  // https://github.com/cloudflare/binary-install/releases/download/v1.0.0/binary-install-example-v1.0.0-x86_64-apple-darwin.tar.gz
  const url = `${repository.url}/releases/download/v0.3.17/gvmkit-build-${platform.RUST_TARGET}.${platform.PACKING}`;
  return new GvmKitBuildBinary(url, { name: "gvmkit-build" });
};

const run = () => {
  const binary = getBinary();
  binary.run();
};

const install = () => {
  const binary = getBinary();
  binary.install();
};

const uninstall = () => {
  const binary = getBinary();
  binary.uninstall();
};

module.exports = {
  install,
  run,
  uninstall,
};
