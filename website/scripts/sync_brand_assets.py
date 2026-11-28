#!/usr/bin/env python3
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Copy the brand files that the site publishes from the production pack.

The pack of the production identity 1.0.0 is the only source of the brand
files. The script copies the icons of the site, the social card and the
downloadable logo files into the public directory of the site, the renders and
the mockups into the assets directory (the image pipeline of the site turns
them into WebP; the script itself writes WebP files of at most 1920 pixels), and writes brand/manifest.json in the public directory with
the SHA-256 digest of every copied file.

Usage:

  sync_brand_assets.py PACK PUBLIC_DIRECTORY ASSETS_DIRECTORY

PACK is the directory design/brand/production of the design repository.
"""

import hashlib
import json
import shutil
import sys
from pathlib import Path

from PIL import Image

ICONS = ["favicon.ico", "favicon.svg", "apple-touch-icon.png", "icon-192.png", "icon-512.png", "icon-maskable-512.png"]
SOCIAL = {"mockups/mockup-social-card.png": "social/social-card.png", "mockups/mockup-social-card@2x.png": "social/social-card@2x.png"}
LAYOUTS = ["horizontal", "vertical", "compact"]
VARIANTS = ["color-dark", "color-light", "black", "white", "gold", "metal"]
RENDERS = ["render-hero-front", "render-hero-angle", "render-macro-edge", "render-light-ground"]
MOCKUPS = ["mockup-terminal-banner", "mockup-docs-header-dark", "mockup-docs-header-light", "mockup-business-card", "mockup-letterhead", "mockup-stationery-set"]


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write_webp(source: Path, assets: Path, folder: str) -> None:
    target = assets / folder / f"{source.stem}.webp"
    target.parent.mkdir(parents=True, exist_ok=True)
    image = Image.open(source).convert("RGB")
    if image.width > 1920:
        image = image.resize((1920, round(image.height * 1920 / image.width)), Image.LANCZOS)
    image.save(target, "WEBP", quality=86, method=6)


def copy(source: Path, target: Path, manifest: dict) -> None:
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, target)
    manifest[str(target)] = {"bytes": target.stat().st_size, "sha256": digest(target)}


def main(argv: list[str]) -> int:
    if len(argv) != 4:
        print(__doc__)
        return 2
    pack, public, assets = Path(argv[1]), Path(argv[2]), Path(argv[3])
    manifest: dict[str, dict] = {}
    for name in ICONS:
        copy(pack / "icons" / "web" / name, public / name, manifest)
    for source, target in SOCIAL.items():
        copy(pack / source, public / target, manifest)
    for layout in LAYOUTS:
        for variant in VARIANTS:
            source = pack / "lockups" / f"lockup-{layout}-{variant}.svg"
            if source.exists():
                copy(source, public / "brand" / "lockups" / source.name, manifest)
    for variant in VARIANTS[:4]:
        for kind in ("mark", "wordmark"):
            source = pack / kind / f"{kind}-{variant}.svg"
            if source.exists():
                copy(source, public / "brand" / kind / source.name, manifest)
    for render in RENDERS:
        write_webp(pack / "render" / f"{render}.png", assets, "renders")
    for mockup in MOCKUPS:
        write_webp(pack / "mockups" / f"{mockup}.png", assets, "mockups")
    copy(pack / "identity.json", public / "brand" / "identity.json", manifest)
    copy(pack / "tokens.css", public / "brand" / "tokens.css", manifest)
    relative = {str(Path(key).relative_to(public)): value for key, value in manifest.items()}
    (public / "brand" / "manifest.json").write_text(json.dumps(relative, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"{len(relative)} files copied into {public}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
