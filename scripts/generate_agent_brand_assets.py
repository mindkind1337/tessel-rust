"""Snapshot public brand artwork; no JavaScript execution or user-profile access.

Usage: python scripts/generate_agent_brand_assets.py C:/Tessel [--check]
Artwork attribution: THIRD_PARTY_NOTICES.md (MIT Lovecast Inc. 2026 and LobeHub).
"""
import argparse
import base64
import hashlib
import json
import math
from pathlib import Path
import re
import xml.etree.ElementTree as ET


def generate(source):
    root = source / "src/renderer/src"
    vue_path = root / "components/BrandIcon.vue"
    raw_path = root / "brandSvgs.js"
    vue = vue_path.read_text(encoding="utf-8")
    raw = re.sub(r"^\s*//.*$", "", raw_path.read_text(encoding="utf-8"), flags=re.M)
    brands = json.loads(raw[raw.index("{"):].strip().rstrip(";"))
    boxes = dict(re.findall(r"(\w+): '([^']+)'", re.search(r"const DRAWN_BOX = \{(.*?)\n\}", vue, re.S)[1]))
    tiles = set(re.findall(r"'([^']+)'", re.search(r"const TILES = new Set\(\[(.*?)\]\)", vue, re.S)[1]))
    constants = {name: float(re.search(r"const " + name + r" = ([0-9.]+)", vue)[1]) for name in ["MARK", "TILE", "TILE_RADIUS"]}
    optical_source = re.search(r"const OPTICAL = \{([^}]+)\}", vue)[1]
    optical = {name: constants[value] if value in constants else float(value) for name, value in re.findall(r"(\w+): ([\w.]+)", optical_source)}
    inputs = [vue_path, raw_path]
    for png in sorted((root / "assets/agent-icons").glob("*.png")):
        inputs.append(png)
        # SVG drawings take precedence over a bundled PNG in BrandIcon.vue.
        brands.setdefault(png.stem, {"box": "0 0 64 64", "body": '<image width="64" height="64" href="data:image/png;base64,' + base64.b64encode(png.read_bytes()).decode("ascii") + '"/>'})
    assert not (set(boxes) - {"claude"}) & set(brands), "Drawn/raw/PNG precedence changed: review the source template"
    rays = []
    for i, length in enumerate([1, .74, .92, .68, .98, .8, .94, .7, 1, .76, .9, .72]):
        angle = math.radians(i * 30 - 82)
        end = 2.2 + length * 8.6
        rays.append(f'<line x1="{12 + math.cos(angle)*2.2:.12g}" y1="{12 + math.sin(angle)*2.2:.12g}" x2="{12 + math.cos(angle)*end:.12g}" y2="{12 + math.sin(angle)*end:.12g}"/>')
    brands["claude"] = {"body": '<g stroke="#d97757" stroke-width="2.3" stroke-linecap="round">' + ''.join(rays) + '</g>'}
    path = re.search(r"const OPENAI_PATH\s*=\s*'([^']+)'", vue)[1]
    for name in ["codex", "openai"]:
        brands[name] = {"body": f'<path d="{path}" fill="#ececec"/>'}
    opencode = re.search(r'<g v-else-if="k === \'opencode\'.*?>(.*?)</g>', vue, re.S)[1]
    for name in ["opencode", "opencode2"]:
        brands[name] = {"body": opencode}
    for name in ["gemini", "powershell", "cmd", "gitbash", "wsl"]:
        body = re.search(r'<template v-else-if="k === \'' + name + r"'.*?\">(.*?)</template>", vue, re.S)[1]
        body = body.replace(':id="gid"', 'id="brand-gradient"').replace(':fill="`url(#${gid})`"', 'fill="url(#brand-gradient)"')
        if name == "powershell":
            for key, color in [("powershell", "#2671be"), ("pwsh", "#34507a")]:
                brands[key] = {"body": re.sub(r':(fill|stroke)="k === \'pwsh\'[^\"]*"', lambda m: f'{m[1]}="{color}"', body)}
        else:
            brands[name] = {"body": body}
    for name in ["bash", "git"]:
        brands[name] = brands["gitbash"].copy()
    icons = {}
    for name, brand in sorted(brands.items()):
        assert re.fullmatch(r"[a-z0-9-]+", name), name
        size = optical.get(name, constants["TILE"] if name in tiles else constants["MARK"])
        origin = 12 - size / 2
        box = boxes.get(name, brand.get("box"))
        assert box, name
        clip = f'<defs><clipPath id="brand-tile"><rect x="{origin}" y="{origin}" width="{size}" height="{size}" rx="{constants['TILE_RADIUS']}"/></clipPath></defs>' if name in tiles else ''
        clipping = ' clip-path="url(#brand-tile)"' if name in tiles else ''
        body = brand["body"].replace("__UID__", "brand")
        svg = f'<svg xmlns="http://www.w3.org/2000/svg" width="13" height="13" viewBox="0 0 24 24" fill="none">{clip}<g{clipping}><svg x="{origin}" y="{origin}" width="{size}" height="{size}" viewBox="{box}" overflow="visible">{body}</svg></g></svg>'
        tree = ET.fromstring(svg)
        for node in tree.iter():
            assert node.tag.split('}')[-1] not in {"script", "foreignObject", "text", "style"}, name
            for key, value in node.attrib.items():
                assert not key.startswith("on") and not key.startswith((":", "v-", "@")) and key != "style", (name, key)
                if key.split('}')[-1] == "href":
                    assert value.startswith(("data:image/png;base64,", "#")), name
        assert all(value.startswith("#") for value in re.findall(r"url\(([^)]+)\)", svg)), name
        icons[name] = svg
    return {"sources": {str(p.relative_to(source)).replace('\\', '/'): hashlib.sha256(p.read_bytes()).hexdigest() for p in inputs}, "icons": icons}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    result = json.dumps(generate(args.source), ensure_ascii=False, indent=2) + "\n"
    destination = Path(__file__).resolve().parent.parent / "src/agent_brand_assets.json"
    if args.check:
        assert destination.read_text(encoding="utf-8") == result, "Brand assets differ from the source snapshot"
    else:
        destination.write_text(result, encoding="utf-8", newline="\n")
    print(f"{len(json.loads(result)['icons'])} embedded brand icons verified")
