import os
import sys
import sysconfig
from pathlib import Path


# copy of https://github.com/charliermarsh/ruff/blob/main/python/ruff/__main__.py
def find_bin() -> Path:
    """Return the binary path."""

    exe = "gvmkit-build" + sysconfig.get_config_var("EXE")

    path = Path(sysconfig.get_path("scripts")) / exe
    if path.is_file():
        return path

    if sys.version_info >= (3, 10):
        user_scheme = sysconfig.get_preferred_scheme("user")
    elif os.name == "nt":
        user_scheme = "nt_user"
    elif sys.platform == "darwin" and sys._framework:
        user_scheme = "osx_framework_user"
    else:
        user_scheme = "posix_user"

    path = Path(sysconfig.get_path("scripts", scheme=user_scheme)) / exe
    if path.is_file():
        return path

    raise FileNotFoundError(path)


if __name__ == "__main__":
    bin = find_bin()
    sys.exit(os.spawnv(os.P_WAIT, bin, ["gvmkit-build", *sys.argv[1:]]))