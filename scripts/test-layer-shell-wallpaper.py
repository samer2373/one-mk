#!/usr/bin/env python3
"""Private Sway video/picture crops, hotplug, scaling, pause, occlusion and CPU acceptance."""
import json, os, pathlib, subprocess, time, struct
from PIL import Image, ImageDraw, ImageChops

parser = __import__('argparse').ArgumentParser(description='Acceptance against an already running PRIVATE headless Sway; never use your desktop runtime.')
parser.add_argument('--runtime', required=True, type=pathlib.Path)
parser.add_argument('--prefix', required=True, type=pathlib.Path)
parser.add_argument('--helper', required=True, type=pathlib.Path)
parser.add_argument('--evidence', required=True, type=pathlib.Path)
args = parser.parse_args()
assert str(args.runtime.resolve()).startswith('/tmp/'), 'Use a private runtime under /tmp'
assert args.runtime.stat().st_mode & 0o777 == 0o700, 'Private runtime must be mode 0700'
ROOT = args.evidence.resolve(); ROOT.mkdir(parents=True, exist_ok=True)
PREFIX = args.prefix.resolve()
ENV = dict(os.environ, XDG_RUNTIME_DIR=str(args.runtime.resolve()), WAYLAND_DISPLAY='wayland-1',
    LD_LIBRARY_PATH=str(PREFIX/'usr/lib/x86_64-linux-gnu'), HOME=str(ROOT), XDG_CONFIG_HOME=str(ROOT))
ENV.pop('DISPLAY', None); ENV.pop('WAYLAND_SOCKET', None)
ENV['SWAYSOCK'] = str(max(args.runtime.glob('sway-ipc.*.sock'),key=lambda path:path.stat().st_mtime))
HELPER = str(args.helper.resolve())
RESULTS = {}
SECOND = 'HEADLESS-2'

def sway(command):
    command = command.replace('HEADLESS-2', SECOND or 'HEADLESS-2')
    result = subprocess.run([str(PREFIX/'usr/bin/swaymsg'), command], env=ENV, check=True, capture_output=True, text=True)
    assert all(item.get('success') for item in json.loads(result.stdout)), result.stdout

def capture(output, name):
    output = output.replace('HEADLESS-2', SECOND)
    path = ROOT / (name + '.png')
    subprocess.run([str(PREFIX/'usr/bin/grim'), '-o', output, str(path)], env=ENV, check=True)
    return Image.open(path).convert('RGB')

def cpu(process, seconds):
    def ticks():
        fields = pathlib.Path(f'/proc/{process.pid}/stat').read_text().split()
        return int(fields[13]) + int(fields[14])
    start = time.monotonic(); before = ticks(); time.sleep(seconds)
    return round((ticks() - before) / os.sysconf('SC_CLK_TCK') / (time.monotonic() - start) * 100, 3)

def command(process, text):
    process.stdin.write(text + '\n'); process.stdin.flush(); time.sleep(.5)
    assert process.poll() is None, f'helper stopped after {text}'

def start(config, name):
    path = ROOT / (name + '.json'); path.write_text(json.dumps(config))
    output = open(ROOT / (name + '.stdout'), 'w'); errors = open(ROOT / (name + '.stderr'), 'w')
    process = subprocess.Popen([HELPER, str(path)], env=ENV, stdin=subprocess.PIPE, stdout=output, stderr=errors, text=True)
    time.sleep(2)
    assert process.poll() is None, (ROOT / (name + '.stderr')).read_text()
    return process

def stop(process):
    process.stdin.write('quit\n'); process.stdin.flush(); assert process.wait(timeout=8) == 0

def compare(actual, expected, label, tolerance=0):
    assert actual.size == expected.size, (label, actual.size, expected.size)
    diff = ImageChops.difference(actual, expected)
    maximum = max(channel[1] for channel in diff.getextrema())
    RESULTS[label] = {'max_channel_error': maximum, 'size': actual.size}
    assert maximum <= tolerance, (label, maximum)

monitors = [dict(computer='test', name='HEADLESS-1', rect=dict(x=0,y=0,width=960,height=1080), local=dict(x=0,y=0,width=960,height=1080), scale=1,online=True),
    dict(computer='test', name='HEADLESS-2', rect=dict(x=960,y=0,width=960,height=720), local=dict(x=960,y=0,width=960,height=720), scale=1,online=True)]
plan = dict(hash='a'*64,title='Private layer-shell test',media=dict(width=1920,height=1080,duration_ms=8000,frame_rate=30),
    canvas=dict(monitors=monitors), start_ms=int(time.time()*1000),fit='stretch',computers=['test'],pause_on_battery=False,paused=False,picture=False)
config = dict(plan=plan,computer='test',path=str(ROOT/'test-1080p.webm'),clock_offset_ms=0)
outputs=json.loads(subprocess.check_output([str(PREFIX/'usr/bin/swaymsg'),'-t','get_outputs'],env=ENV))
SECOND=next((output['name'] for output in outputs if output['name'] != 'HEADLESS-1'),None)
if SECOND is None:
    sway('create_output')
    outputs=json.loads(subprocess.check_output([str(PREFIX/'usr/bin/swaymsg'),'-t','get_outputs'],env=ENV))
    SECOND=next(output['name'] for output in outputs if output['name'] != 'HEADLESS-1')
sway('output HEADLESS-2 mode 960x720 scale 1 position 960 0')
if not (ROOT/'test-1080p.webm').exists():
    subprocess.run(['gst-launch-1.0','-q','videotestsrc','pattern=smpte','num-buffers=240','!',
        'video/x-raw,width=1920,height=1080,framerate=30/1','!','timeoverlay','!',
        'vp8enc','deadline=1','cpu-used=8','threads=2','target-bitrate=4000000','!',
        'webmmux','!','filesink',f'location={ROOT}/test-1080p.webm'],check=True,timeout=120)
process = start(config, 'video')
try:
    first = capture('HEADLESS-1', 'video-1-a'); time.sleep(.8)
    second = capture('HEADLESS-1', 'video-1-b')
    assert ImageChops.difference(first, second).getbbox(), 'frames did not advance'
    RESULTS['frames_advance'] = True
    RESULTS['playing_1080p_cpu_percent_one_core'] = cpu(process, 6)
    command(process, 'pause')
    paused = capture('HEADLESS-1', 'paused-1-a'); time.sleep(1)
    compare(capture('HEADLESS-1', 'paused-1-b'), paused, 'pause_holds_frame')
    capture('HEADLESS-2', 'paused-2')
    RESULTS['paused_cpu_percent_one_core'] = cpu(process, 5)
    command(process, 'play')
    assert ImageChops.difference(capture('HEADLESS-1','resumed'),paused).getbbox(), 'resume did not advance'
    command(process, 'offset 1500')
    sway('output HEADLESS-2 disable'); time.sleep(.5)
    capture('HEADLESS-1', 'output-removed')
    assert process.poll() is None
    sway('output HEADLESS-2 enable mode 960x720 scale 1 position 960 0'); time.sleep(1)
    assert capture('HEADLESS-2', 'output-reconnected').getbbox(), 'reconnected output is blank'
    RESULTS['video_hotplug_same_pid'] = process.pid
    sway('output HEADLESS-2 mode 1920x1440 scale 2 position 960 0'); time.sleep(1)
    scaled = capture('HEADLESS-2', 'video-scale2'); assert scaled.size == (1920,1440)
    RESULTS['video_scale2_size'] = scaled.size
    sway('output HEADLESS-2 mode 960x720 scale 1 position 960 0'); time.sleep(.5)
    sway('output HEADLESS-1 disable'); sway('output HEADLESS-2 disable'); time.sleep(1)
    RESULTS['no_outputs_cpu_percent_one_core'] = cpu(process, 3)
    sway('output HEADLESS-1 enable mode 960x1080 position 0 0')
    sway('output HEADLESS-2 enable mode 960x720 position 960 0'); time.sleep(1)
    assert capture('HEADLESS-1', 'outputs-restored').getbbox(), 'outputs did not return'
finally:
    stop(process)
off = capture('HEADLESS-1','off-video')
assert off.getpixel((400,400)) == (0,0,0), off.getpixel((400,400))
RESULTS['video_off_reveals_compositor_background'] = True

image = Image.new('RGB',(1920,1080))
pixels = image.load()
for row in range(1080):
    for column in range(1920):
        pixels[column,row] = (column*255//1919,row*255//1079,(column//80+row//80)%2*200)
ImageDraw.Draw(image).text((900,400),'CANVAS SEAM',fill='white')
image.save(ROOT/'source-picture.png')
source = (ROOT/'source-picture.png').read_bytes()
(ROOT/'picture.bundle').write_bytes(b'oneMKpi2'+struct.pack('<Q',len(source))+source)
config['plan']['picture']=True; config['path']=str(ROOT/'picture.bundle')
process=start(config,'picture')
try:
    left=capture('HEADLESS-1','picture-left'); right=capture('HEADLESS-2','picture-right')
    compare(left,image.crop((0,0,960,1080)),'picture_left_crop')
    compare(right,image.crop((960,0,1920,720)),'picture_right_crop')
    combined=Image.new('RGB',image.size); combined.paste(left,(0,0)); combined.paste(right,(960,0)); combined.save(ROOT/'picture-continuous-canvas.png')
    assert '"picture"' in (ROOT/'picture.stdout').read_text()
    RESULTS['picture_cpu_percent_one_core']=cpu(process,3)
    sway('output HEADLESS-2 disable'); time.sleep(.5)
    sway('output HEADLESS-2 enable mode 960x720 position 960 0'); time.sleep(.5)
    compare(capture('HEADLESS-2','picture-reconnected'),right,'picture_hotplug')
    command(process,'pause'); compare(capture('HEADLESS-1','picture-paused'),left,'picture_ignores_video_pause')
    sway('output HEADLESS-2 mode 1920x1440 scale 2 position 960 0'); time.sleep(.5)
    scaled=capture('HEADLESS-2','picture-scale2')
    compare(scaled,image.crop((960,0,1920,720)).resize((1920,1440),Image.Resampling.NEAREST),'picture_scale2')
    sway('output HEADLESS-2 mode 960x720 scale 1 position 960 0'); time.sleep(.5)
    terminal=subprocess.Popen([str(PREFIX/'usr/bin/foot'),'--app-id=layer-test','--title=Wallpaper occlusion test','sh','-c','printf "WINDOW ABOVE WALLPAPER\n"; sleep 20'],env=ENV,stdout=subprocess.DEVNULL,stderr=open(ROOT/'foot.log','w'))
    try:
        time.sleep(1)
        sway('[app_id="layer-test"] move container to output HEADLESS-1, floating enable, resize set 600 400, move position 100 100')
        covered=capture('HEADLESS-1','window-above-wallpaper')
        assert covered.getpixel((200,200)) != left.getpixel((200,200)), 'window not above wallpaper'
        compare(covered.crop((750,600,850,700)),left.crop((750,600,850,700)),'wallpaper_visible_outside_window')
        RESULTS['window_covers_wallpaper']=True
    finally:
        terminal.terminate(); terminal.wait(timeout=5)
finally:
    stop(process)
off=capture('HEADLESS-1','off-picture')
assert off.getpixel((400,400)) == (0,0,0)
RESULTS['picture_off_reveals_compositor_background']=True
config['plan']['canvas']['monitors'][1]['rect']['x']=1440
for fit in ['fit','fill']:
    config['plan']['fit']=fit
    process=start(config,'picture-'+fit)
    try:
        expected=Image.new('RGB',(2400,1080))
        if fit=='fit': expected.paste(image,(240,0))
        else: expected=image.resize((2400,1350),Image.Resampling.NEAREST).crop((0,135,2400,1215))
        compare(capture('HEADLESS-1','picture-'+fit+'-left'),expected.crop((0,0,960,1080)),fit+'_left',1 if fit=='fill' else 0)
        compare(capture('HEADLESS-2','picture-'+fit+'-right'),expected.crop((1440,0,2400,720)),fit+'_right',1 if fit=='fill' else 0)
    finally:
        process.stdin.close(); assert process.wait(timeout=8)==0
RESULTS['parent_eof_tears_down_surfaces']=not capture('HEADLESS-1','off-eof').getbbox()
assert RESULTS['parent_eof_tears_down_surfaces']
(ROOT/'results.json').write_text(json.dumps(RESULTS,indent=2))
print(json.dumps(RESULTS,indent=2))
