import argparse
import hashlib
import json
from pathlib import Path

from PIL import Image, ImageDraw, ImageOps


parser = argparse.ArgumentParser()
parser.add_argument("root", type=Path)
parser.add_argument("tip")
args = parser.parse_args()
dimensions = {"1x": (1200, 780), "2x": (2400, 1560), "small": (760, 540), "820x540": (820, 540), "guide": (1280, 800)}
canonical = {}
records = []
for matrix, expected in dimensions.items():
    directory = args.root / matrix
    captured = [json.loads(line) for line in (directory / "captures.jsonl").read_text().splitlines()]
    assert len({record["file"] for record in captured}) == len(captured)
    for record in captured:
        path = directory / record["file"]
        assert record["source_commit"] == args.tip, path
        assert tuple(record["size"]) == expected, (path, record["size"], expected)
        digest = record["sha256"]
        relative = str(path.relative_to(args.root))
        destination = canonical.setdefault(digest, relative)
        if path.exists():
            assert hashlib.sha256(path.read_bytes()).hexdigest() == digest, path
            if destination != relative:
                path.unlink()
        assert (args.root / destination).exists(), destination
        records.append(dict(record, matrix=matrix, requested_file=relative, canonical_file=destination, identical_render_alias=destination != relative))
assert len({record["file"] for record in records if record["matrix"] == "1x"}) == 93
for matrix in ["2x", "small", "820x540"]:
    assert {record["file"] for record in records if record["matrix"] == matrix} == {record["file"] for record in records if record["matrix"] == "1x"}
report = {"source_commit": args.tip, "requested_captures": len(records), "distinct_shot_hashes": len(canonical), "identical_render_aliases": len(records) - len(canonical), "policy": "Every requested scenario was freshly rendered. Byte-identical renders have one canonical PNG, not duplicate files; aliases are explicit below.", "captures": records}
(args.root / "shot-index.json").write_text(json.dumps(report, indent=2) + "\n")
review = args.root / "review"
review.mkdir(exist_ok=True)
for matrix in dimensions:
    paths = [args.root / relative for relative in canonical.values() if relative.startswith(matrix + "/")]
    for offset in range(0, len(paths), 12):
        batch = paths[offset:offset + 12]
        sheet = Image.new("RGB", (1440, ((len(batch) + 2) // 3) * 340), "#d8d8d8")
        painter = ImageDraw.Draw(sheet)
        for index, path in enumerate(batch):
            origin = (index % 3 * 480, index // 3 * 340)
            painter.text((origin[0] + 6, origin[1] + 4), path.name, fill="black")
            thumbnail = ImageOps.contain(Image.open(path).convert("RGB"), (476, 310))
            sheet.paste(thumbnail, (origin[0], origin[1] + 24))
        sheet.save(review / f"{matrix}-{offset // 12 + 1:02}.jpg", quality=92)
print(json.dumps({key: value for key, value in report.items() if key != "captures"}, indent=2))
