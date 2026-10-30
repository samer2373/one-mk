#!/usr/bin/env python3
import argparse
import hashlib
import json
from pathlib import Path
import shutil

from PIL import Image


def archive(label, destination, images):
    lab = Path(__file__).resolve().parents[3] / 'target/wall-m1-lab'
    report = json.loads((lab / f'{label}.json').read_text())
    destination.mkdir(parents=True, exist_ok=True)
    for key, frames in report['measurements'].items():
        if not key.endswith('_shots'):
            continue
        for index, frame in enumerate(frames):
            source = lab / frame['file']
            image = Image.open(source).convert('RGB')
            frame['png_sha256'] = hashlib.sha256(source.read_bytes()).hexdigest()
            frame['dimensions'] = list(image.size)
            frame['samples'] = [list(image.getpixel((image.width * part // 3, image.height // 2)))
                                for part in [1, 2]]
            output = f'{label}-{key}-{index:02}.webp'
            if images:
                image.resize((480, round(image.height * 480 / image.width))).save(
                    destination / output, lossless=True)
            frame['file'] = output if images else None
            source.unlink()
            if not any(source.parent.iterdir()):
                source.parent.rmdir()
    (destination / f'{label}.json').write_text(json.dumps(report, indent=2) + '\n')
    shutil.copyfile(lab / f'{label}-shell.log', destination / f'{label}-shell.log')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('label')
    parser.add_argument('destination', type=Path)
    parser.add_argument('--images', action='store_true')
    arguments = parser.parse_args()
    archive(arguments.label, arguments.destination, arguments.images)
