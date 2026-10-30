#!/usr/bin/env python3
"""Summarize retained CSV measurements without rerunning experiments."""
import csv
from pathlib import Path
from statistics import mean

ROOT = Path(__file__).parent
rows = list(csv.DictReader((ROOT / "simulation.csv").open()))
assert len(rows) == 1152, "incomplete simulation sweep"
print("Selected simulation: probe=5s, cap=0.005, drift=200ppm, computers=4, anchors=32, seed=42")
for model in ("wired", "wifi", "bad", "adversarial"):
    group = [row for row in rows if row["model"] == model]
    selected = [row for row in group if all(row[key] == value for key, value in {
        "probe_s": "5", "nudge_limit": "0.005", "ppm": "200", "computers": "4",
        "filter_anchors": "32", "seed": "42"}.items())]
    assert len(selected) == 1
    print(selected[0])
    print("Worst across sweep:", model, {key: max(float(row[key]) for row in group)
          for key in ("p95_ms", "p99_ms", "max_ms", "convergence_s", "recovery_s", "correction_jumps")})
    for parameter in ("probe_s", "nudge_limit", "filter_anchors"):
        for value in sorted({row[parameter] for row in group}, key=float):
            subset = [row for row in group if row[parameter] == value]
            print(model, parameter, value, "worst P95 ms", max(float(row["p95_ms"]) for row in subset),
                  "mean error ms", round(mean(float(row["mean_ms"]) for row in subset), 4))

bench = list(csv.DictReader((ROOT / "gst-benchmark.csv").open()))
assert len(bench) == 60, "incomplete decoder benchmark"
print("Decode: width,height,outputs,decoders,mean CPU percent at 60fps,mean RSS MiB,mean FPS")
for width in (1920, 3840, 7680, 11520):
    for outputs, decoders in ((1, 1), (2, 1), (2, 2), (3, 1), (3, 3)):
        subset = [row for row in bench if (int(row["width"]), int(row["outputs"]), int(row["decoders"])) == (width, outputs, decoders)]
        assert len(subset) == 3
        print(width, subset[0]["height"], outputs, decoders,
              *(round(mean(float(row[key]) for row in subset), 2)
                for key in ("cpu_percent_at_60fps", "max_rss_mib", "decode_fps")))
