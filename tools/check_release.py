"""Reject release tags that do not match every package and internal dependency."""
from pathlib import Path
import argparse
import os
import re
import tomllib


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("tag", nargs="?", default=os.environ.get("GITHUB_REF_NAME", ""))
    args = parser.parse_args()
    if not re.fullmatch(r"v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?", args.tag):
        parser.error("Expected a release tag such as v0.2.0")
    root = Path(__file__).resolve().parents[1]
    workspace = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]
    version = workspace["package"]["version"]
    if args.tag != f"v{version}":
        parser.error(f"Tag {args.tag} does not match workspace version {version}")
    packages = {}
    for member in workspace["members"]:
        package = tomllib.loads((root / member / "Cargo.toml").read_text(encoding="utf-8"))["package"]
        actual = version if package.get("version") == {"workspace": True} else package.get("version")
        if actual != version:
            parser.error(f"{member} must have version {version}")
        packages[package["name"]] = version
    for name, dependency in workspace["dependencies"].items():
        if name in packages and dependency.get("version") != packages[name]:
            parser.error(f"Internal dependency {name} must declare version {packages[name]}")
    print(f"Validated {args.tag}: {', '.join(packages)}")


if __name__ == "__main__":
    main()
