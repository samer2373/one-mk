"""6D-M1 step B laboratory media: original, locally generated test content only.

Every animated pattern frame is fully determined by (width, height, frame,
frames), so screenshots can be compared with exact expected crops. Thirds of
the source differ by dominant channel (red, green, blue): with the laboratory
canvas the red third is the reserved absent-computer slice, so any red-dominant
pixel on a local monitor means the reserved slice was drawn. A horizontal band
carries a per-frame colour and a lower band the frame number in binary, so
each monitor slice identifies the frame.
"""
import colorsys
import subprocess
from PIL import Image, ImageDraw

GST = ['env', '-i', 'PATH=/usr/bin:/bin', 'HOME=/nonexistent', 'gst-launch-1.0', '-q']


def frame_colour(frame, frames):
    # Hues from yellow-green to magenta only: never red-dominant (red marks
    # the reserved absent-computer third).
    # A stride coprime with the frame count keeps neighbouring frames far apart.
    step = next(stride for stride in (7, 11, 13, 17, 1) if frames % stride)
    red, green, blue = colorsys.hsv_to_rgb(0.2 + 0.6 * ((frame * step) % frames) / max(1, frames), 0.85, 0.95)
    # Keep the band clear of the stripe palette (channels 40..220 with 30/70).
    return (int(red * 255) | 1, int(green * 255) | 1, int(blue * 255) | 1)


_BASES = {}


def base(width, height):
    key = (width, height)
    if key not in _BASES:
        third = width // 3
        stripe = max(1, width // 144)
        band = max(1, height // 18)
        pixels = []
        for y in range(height):
            other = 30 if (y // band) % 2 == 0 else 70
            row = []
            for x in range(width):
                part = min(2, x // third)
                level = 40 + 12 * ((x // stripe) % 16)
                colour = [other, other, other]
                colour[part] = level
                row.append(tuple(colour))
            pixels.extend(row)
        image = Image.new('RGB', (width, height))
        image.putdata(pixels)
        _BASES[key] = image
    return _BASES[key]


def pattern(width, height, frame, frames):
    image = base(width, height).copy()
    draw = ImageDraw.Draw(image)
    stripe = max(1, width // 144)
    top, bottom = int(height * 0.35), int(height * 0.65)
    colour = frame_colour(frame, frames)
    for left in range(0, width, 4 * stripe):
        draw.rectangle([left, top, left + 2 * stripe - 1, bottom - 1], fill=colour)
    # The frame number in binary, repeated across the width: light = 1.
    code_top, code_bottom = int(height * 0.68), int(height * 0.88)
    for block, left in enumerate(range(0, width, 2 * stripe)):
        bit = (frame >> (block % 6)) & 1
        draw.rectangle([left, code_top, left + 2 * stripe - 1, code_bottom - 1], fill=(235, 235, 235) if bit else (15, 15, 15))
    return image


def still(width, height):
    image = pattern(width, height, 0, 1)
    draw = ImageDraw.Draw(image)
    # A distinct still marker: white squares along the band.
    step = width // 24
    for left in range(step // 2, width, step):
        draw.rectangle([left, int(height * 0.42), left + step // 3, int(height * 0.58)], fill=(250, 250, 250))
    return image


def save_images(directory, name, width, height, frames, delay_ms):
    """GIF, APNG and lossless animated WebP of the same pattern frames."""
    images = [pattern(width, height, index, frames) for index in range(frames)]
    paths = {}
    for kind, options in (('gif', {'optimize': False, 'disposal': 1}), ('png', {'default_image': False}),
                          ('webp', {'lossless': True, 'quality': 100, 'method': 0})):
        path = directory / f'{name}.{kind}'
        images[0].save(path, save_all=True, append_images=images[1:], duration=delay_ms, loop=0, **options)
        paths[kind] = path
    return paths


def save_still(directory, width, height):
    path = directory / 'still.png'
    still(width, height).save(path, optimize=False)
    return path


def encode_pattern_video(path, width, height, frames, fps, codec):
    """VP9 or AV1 WebM of the pattern frames, piped raw into gst-launch."""
    encoder = {
        'vp9': ['vp9enc', 'deadline=1', 'cpu-used=4', 'min-quantizer=0', 'max-quantizer=8', 'target-bitrate=8000000',
                f'keyframe-max-dist={fps}'],
        'av1': ['av1enc', 'usage-profile=realtime', 'cpu-used=8', 'min-quantizer=0', 'max-quantizer=12', 'target-bitrate=8000',
                f'keyframe-max-dist={fps}'],
    }[codec]
    command = GST + ['fdsrc', 'fd=0', '!', 'rawvideoparse', f'width={width}', f'height={height}', 'format=rgb',
                     f'framerate={fps}/1', '!', 'videoconvert', '!', 'video/x-raw,format=I420', '!', *encoder, '!',
                     'webmmux', '!', 'filesink', f'location={path}']
    process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    for index in range(frames):
        process.stdin.write(pattern(width, height, index, frames).tobytes())
    process.stdin.close()
    errors = process.stderr.read()
    if process.wait(timeout=300) != 0:
        raise RuntimeError(f'{codec} encode failed: {errors[-500:]!r}')
    return path


def encode_clip_1080p(path, seconds=8, fps=30):
    """A 1080p30 VP9 clip from GStreamer's own moving test source."""
    command = GST + ['videotestsrc', f'num-buffers={seconds * fps}', 'pattern=smpte', 'horizontal-speed=4', '!',
                     f'video/x-raw,width=1920,height=1080,framerate={fps}/1', '!', 'vp9enc', 'deadline=1', 'cpu-used=8',
                     'target-bitrate=4000000', f'keyframe-max-dist={fps}', 'threads=4', '!', 'webmmux', '!',
                     'filesink', f'location={path}']
    subprocess.run(command, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=600)
    return path


def decoded_frames(path):
    """Frames as an image library decodes them (for exactness checks)."""
    with Image.open(path) as image:
        frames = []
        for index in range(getattr(image, 'n_frames', 1)):
            image.seek(index)
            frames.append(image.convert('RGB'))
        return frames
