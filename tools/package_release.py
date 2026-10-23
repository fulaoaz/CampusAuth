"""Build Rust source/deployment archives from explicit public file lists.
This is a release-maintainer utility; installed users never need Python.
"""
from pathlib import Path
import tarfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]
TOP = ("Cargo.toml", "Cargo.lock", "build.rs", "README.md", "LICENSE", "THIRD_PARTY_NOTICES.txt", "VERSION", ".gitignore", ".gitattributes", ".cargo/config.toml")
TREES = {
    "src": {".rs"},
    "desktop": {".rs", ".toml", ".lock", ".json", ".nsh", ".png", ".ico", ".sh", ".ps1", ".md"},
    "ui": {".tsx", ".ts", ".css", ".html", ".json", ".woff2", ".md"},
    "assets": {".svg", ".png", ".ico", ".ttf", ".otf", ".txt", ".html", ".css", ".js", ".md"},
    "deploy": {".sh", ".service", ".md"},
    "tools": {".sh", ".ps1"},
    "examples": {".ini", ".json"},
    "docs": {".md", ".png", ".svg"},
}

def source_paths():
    paths = [ROOT / name for name in TOP]
    paths.append(ROOT / "tools/package_release.py")
    for tree, extensions in TREES.items():
        for path in (ROOT / tree).rglob("*"):
            if {"node_modules", "target", "dist", "gen", ".git"}.intersection(path.relative_to(ROOT / tree).parts):
                continue
            if path.is_file() and path.suffix.lower() in extensions:
                paths.append(path)
    for path in paths:
        if not path.is_file():
            raise FileNotFoundError(path)
    return sorted(set(paths))

def archives(paths, stem, prefix):
    dest = ROOT / "dist"
    dest.mkdir(exist_ok=True)
    with zipfile.ZipFile(dest / (stem + ".zip"), "w", zipfile.ZIP_DEFLATED) as output:
        for path in paths:
            output.write(path, prefix + "/" + path.relative_to(ROOT).as_posix())
    with tarfile.open(dest / (stem + ".tar.gz"), "w:gz") as output:
        for path in paths:
            name = prefix + "/" + path.relative_to(ROOT).as_posix()
            info = output.gettarinfo(str(path), arcname=name)
            info.uid = info.gid = 0
            info.uname = info.gname = ""
            info.mode = 0o755 if path.suffix == ".sh" else 0o644
            with path.open("rb") as content:
                output.addfile(info, content)

def build():
    version = (ROOT / "VERSION").read_text(encoding="ascii").strip()
    archives(source_paths(), "CampusAuth-source-" + version, "CampusAuth")
    for arch in ("amd64", "arm64"):
        binary = ROOT / ("dist/campus-auth-server-linux-" + arch)
        if not binary.exists():
            continue
        paths = [binary, ROOT / "README.md", ROOT / "LICENSE", ROOT / "THIRD_PARTY_NOTICES.txt"]
        paths += sorted((ROOT / "deploy/linux").glob("*.sh"))
        dest = ROOT / "dist" / ("CampusAuth-server-" + version + "-linux-" + arch + ".tar.gz")
        with tarfile.open(dest, "w:gz") as output:
            for path in paths:
                rel = path.name if path == binary else path.relative_to(ROOT).as_posix()
                info = output.gettarinfo(str(path), "CampusAuth/" + rel)
                info.uid = info.gid = 0
                info.uname = info.gname = ""
                info.mode = 0o755 if path == binary or path.suffix == ".sh" else 0o644
                with path.open("rb") as content: output.addfile(info, content)
    return ROOT / "dist"

if __name__ == "__main__":
    print(build())
