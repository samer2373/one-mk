import json
from pathlib import Path
import sys

from PIL import Image, ImageChops


source = Path(sys.argv[1])
results = []
for path in sorted(source.glob("keyboard-*.png")):
    image = Image.open(path).convert("RGB")
    red, green, blue = image.split()
    mask = ImageChops.multiply(red.point(lambda value: 255 if value > 160 else 0), green.point(lambda value: 255 if value > 200 else 0))
    mask = ImageChops.multiply(mask, blue.point(lambda value: 255 if value < 150 else 0))
    width = image.width
    positions = {index for index, value in enumerate(mask.tobytes()) if value}
    rings = []
    while positions:
        start = positions.pop()
        pending = [start]
        component = []
        while pending:
            index = pending.pop()
            component.append(index)
            for neighbor in [index - width - 1, index - width, index - width + 1, index - 1, index + 1, index + width - 1, index + width, index + width + 1]:
                if neighbor in positions:
                    positions.remove(neighbor)
                    pending.append(neighbor)
        left = min(index % width for index in component)
        right = max(index % width for index in component)
        top = min(index // width for index in component)
        bottom = max(index // width for index in component)
        area = (right - left + 1) * (bottom - top + 1)
        if right - left >= 23 and bottom - top >= 23 and len(component) > 40 and len(component) / area < 0.45:
            rings.append({"bounds_px": [left, top, right, bottom], "signal_pixels": len(component)})
    results.append({"screenshot": path.name, "ring_candidates": rings})
report = {"method": "Signal-colour connected-component heuristic; thin components at least 24x24 physical pixels. Supplement with visual review; this does not establish focused-target identity or semantic correctness.", "captures": len(results), "without_ring_candidate": [row["screenshot"] for row in results if not row["ring_candidates"]], "results": results}
(source / "focus-ring-audit.json").write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps({key: value for key, value in report.items() if key != "results"}))
