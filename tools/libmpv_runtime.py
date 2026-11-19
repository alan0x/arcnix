"""Stage the pinned, replaceable Windows x64 libmpv runtime next to Arcnix."""
import argparse
import hashlib
from pathlib import Path
import shutil
import tempfile
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[1]
VERSION = "0.42.0"
RELEASE = f"https://github.com/mpv-player/mpv/releases/download/v{VERSION}"
URL = f"{RELEASE}/libmpv-v{VERSION}-x86_64-w64-mingw32-lgpl.zip"
DIGEST = "8e5f77430efc53ef175804ba6618527e3af0448a0dcabf04d3cc72659d1a6fce"


def verified(path):
    if not path.is_file():
        return False
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest() == DIGEST


def stage(destination):
    archive = ROOT / "artifacts" / f"libmpv-{VERSION}.zip"
    if not verified(archive):
        archive.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=archive.parent) as temporary:
            download = Path(temporary) / archive.name
            urllib.request.urlretrieve(URL, download)
            if not verified(download):
                raise ValueError("libmpv archive checksum mismatch")
            download.replace(archive)
    notices = destination / "LICENSES" / "libmpv"
    notices.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(archive) as package:
        for entry, target in (("libmpv-2.dll", destination / "libmpv-2.dll"),
                              ("LICENSE.LGPL", notices / "LICENSE.LGPL")):
            with package.open(entry) as src, target.open("wb") as dst:
                shutil.copyfileobj(src, dst)
    (notices / "SOURCE.txt").write_text(
        f"libmpv {VERSION}, Windows x86_64 LGPL build.\nBinary: {URL}\nSHA-256: {DIGEST}\n"
        f"Source and build scripts: https://github.com/mpv-player/mpv/tree/v{VERSION}\n"
        "The DLL can be replaced with a compatible libmpv build.\n", encoding="utf-8")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path)
    stage(parser.parse_args().destination)
