from pathlib import Path
import sys
from PIL import Image, ImageDraw, ImageFont

source = Path(sys.argv[1])
destination = Path(sys.argv[2])
destination.mkdir(parents=True, exist_ok=True)
files = sorted(source.glob('*.png'))
font = ImageFont.truetype('/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf', 17)
for start in range(0, len(files), 6):
    batch = files[start:start + 6]
    sheet = Image.new('RGB', (1920, 2010), '#22282a')
    draw = ImageDraw.Draw(sheet)
    for index, path in enumerate(batch):
        image = Image.open(path).convert('RGB')
        image.thumbnail((950, 625))
        left = index % 2 * 960
        top = index // 2 * 670
        draw.text((left + 8, top + 8), path.name, font=font, fill='white')
        sheet.paste(image, (left + 5, top + 36))
    target = destination / f'contact-{start // 6 + 1:02}.jpg'
    sheet.save(target, quality=92)
    print(target)
