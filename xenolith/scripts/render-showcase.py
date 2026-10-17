"""Render the showcase video and poster from fictional notes and an isolated app capture.

Requires Python 3, Pillow, numpy and imageio-ffmpeg. No network or live wiki access.
Fonts default to Segoe UI on Windows and Avenir Next (Medium and Bold) on macOS. Set
SHOWCASE_FONT and SHOWCASE_FONT_BOLD to use other installed fonts; for a collection (.ttc),
add the face's style or index after a #, as in "/System/Library/Fonts/HelveticaNeue.ttc#Bold".
"""
from pathlib import Path
import math
import os
import subprocess
import sys
import wave
import numpy as np
from PIL import Image, ImageDraw, ImageFont
import imageio_ffmpeg

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'docs' / 'showcase'
OUT.mkdir(parents=True, exist_ok=True)
W, H, FPS, DURATION = 1080, 1350, 24, 32
DEFAULT_FONTS = {
    'win32': ('C:/Windows/Fonts/segoeui.ttf', 'C:/Windows/Fonts/segoeuib.ttf'),
    # Avenir Next's Regular is light next to Segoe UI's; Medium reads the same on video.
    'darwin': ('/System/Library/Fonts/Avenir Next.ttc#Medium', '/System/Library/Fonts/Avenir Next.ttc#Bold'),
}.get(sys.platform, (None, None))
FONT = os.environ.get('SHOWCASE_FONT') or DEFAULT_FONTS[0]
BOLD = os.environ.get('SHOWCASE_FONT_BOLD') or DEFAULT_FONTS[1]
if not FONT or not BOLD:
    raise SystemExit('Set SHOWCASE_FONT and SHOWCASE_FONT_BOLD to a regular and a bold sans-serif font file.')
fonts = {}
WHITE, MUTED, VIOLET, MINT = '#f1f2fa', '#a6b2cd', '#b9a8ff', '#80e0ca'

def load_font(spec, size):
    # "file" or "file#face", where face is a style name ("Bold") or an index into a collection.
    path, sep, face = spec.rpartition('#')
    if not sep:
        return ImageFont.truetype(spec, size)
    if face.isdigit():
        return ImageFont.truetype(path, size, index=int(face))
    for i in range(64):
        try:
            f = ImageFont.truetype(path, size, index=i)
        except OSError:
            break
        if f.getname()[1] == face:
            return f
    raise SystemExit(f'No "{face}" face in {path}')

def font(size, bold=False):
    key = size, bold
    if key not in fonts:
        fonts[key] = load_font(BOLD if bold else FONT, size)
    return fonts[key]

def text(d, pos, s, size=30, color=WHITE, bold=False, anchor=None):
    d.text(pos, s, font=font(size, bold), fill=color, anchor=anchor)

def ease(v):
    v = max(0, min(1, v))
    return 1 - (1-v)**3

def card(d, box, fill='#171d30', edge='#37415e', radius=28):
    d.rounded_rectangle(tuple(map(int, box)), radius, fill=fill, outline=edge, width=2)

def bezier(p0, p1, p2, p3, n=12):
    return [tuple((1-t)**3*a + 3*(1-t)**2*t*b + 3*(1-t)*t**2*c + t**3*e for a,b,c,e in zip(p0,p1,p2,p3)) for t in (i/n for i in range(1, n+1))]

# The Xenolith mark (assets/icon/xenolith.svg, 256 units): a dog-eared page with a speech-bubble tail.
MARK_PAGE = [(52,16),(164,16),(232,84),(232,168),*bezier((232,168),(232,184),(220,196),(204,196)),(116,196),(64,244),(64,196),(52,196),
             *bezier((52,196),(36,196),(24,184),(24,168)),(24,44),*bezier((24,44),(24,28),(36,16),(52,16))]
MARK_FOLD = [(164,16),(164,64),*bezier((164,64),(164,75),(173,84),(184,84)),(232,84)]
MARK_BARS = [(60,68,136,90),(60,118,196,140)]

def icon(im, x, y, size=100):
    # Drawn at 4x and scaled down, so the curves are smooth. size=100 is about 90 x 95 px.
    k, s = 4, size / 240
    w, h = int(240*s)+2, int(240*s)+2
    layer = Image.new('RGBA', (w*k, h*k), (0,0,0,0)); d = ImageDraw.Draw(layer)
    def p(u,v): return ((u-14)*s*k, (v-16)*s*k)
    d.polygon([p(*q) for q in MARK_PAGE], fill='#9787ef')
    d.polygon([p(*q) for q in MARK_FOLD], fill='#d5cdff')
    for x0,y0,x1,y1 in MARK_BARS:
        d.rounded_rectangle((*p(x0,y0),*p(x1,y1)), 11*s*k, fill='white')
    layer = layer.resize((w,h), Image.Resampling.LANCZOS)
    im.paste(layer, (int(x),int(y)), layer)

yy, xx = np.mgrid[0:H,0:W]
glow = np.exp(-((xx-850)**2+(yy-550)**2)/360000)
base = np.empty((H,W,3),dtype=np.uint8)
for c,b in enumerate([8,12,23]): base[:,:,c] = b+glow*[22,17,43][c]
BG = Image.fromarray(base)
capture = Image.open(OUT/'demo-page.png').convert('RGB').crop((0,150,449,645))

def background(t):
    im = BG.copy(); d = ImageDraw.Draw(im)
    for x in range(0,W,60): d.line((x,0,x,H), fill='#151b2c')
    for y in range(0,H,60): d.line((0,y,W,y), fill='#151b2c')
    for i in range(24):
        x = (i*137+30*math.sin(t*.2+i))%W
        y = (i*199-t*10)%H
        d.ellipse((x,y,x+2,y+2),fill='#5a597e')
    icon(im,64,62,40); text(d,(118,61),'Xenolith',30,bold=True)
    text(d,(1010,71),'DEMO DATA',18,MUTED,anchor='ra')
    d.line((64,1260,1016,1260),fill='#32394f',width=2)
    d.line((64,1260,64+952*t/DURATION,1260),fill=VIOLET,width=3)
    text(d,(64,1290),'SHARED CONTEXT / PLAIN MARKDOWN',17,MUTED)
    text(d,(1016,1290),f'{min(5,int(t//6.4)+1):02} / 05',17,MUTED,anchor='ra')
    return im

def scene(index, u, t):
    im = background(t); d = ImageDraw.Draw(im)
    rise = int(38*(1-ease(u/1.1)))
    if index == 0:
        text(d,(64,210+rise),'A little less',80,bold=True)
        text(d,(64,305+rise),'starting over.',80,VIOLET,True)
        text(d,(68,445),'Shared context for every app and agent.',35,MUTED)
        cx,cy = 540,850
        for i,(label,angle) in enumerate([('Claude Code',-150),('Codex',-30),('Any MCP client',90)]):
            a=math.radians(angle)+.04*math.sin(t*.6)
            x,y = cx+330*math.cos(a),cy+250*math.sin(a)
            d.line((x,y,cx,cy),fill='#5a527f',width=3)
            q=(u*.24+i*.32)%1
            px,py=x+(cx-x)*q,y+(cy-y)*q
            d.ellipse((px-6,py-6,px+6,py+6),fill=MINT)
            half=max(104,d.textlength(label,font=font(29))/2+34)
            card(d,(x-half,y-38,x+half,y+38))
            text(d,(x,y-20),label,29,anchor='ma')
        card(d,(440,750,640,950),fill='#27213e',edge='#706298',radius=44)
        icon(im,490,800,110)
    elif index == 1:
        text(d,(64,210+rise),'Keep the useful bits.',67,bold=True)
        text(d,(68,315),'Decisions. Preferences. What comes next.',30,MUTED)
        notes=[('A decision','Keep the first version small.'),('A preference','Warm colors. Plain language.'),('The next step','Pick three photos for the story.')]
        for i,(label,body) in enumerate(notes):
            z=ease((u-i*.45)/.85); x=64+int((1-z)*180); y=460+i*207
            card(d,(x,y,1016+int((1-z)*180),y+168))
            text(d,(x+32,y+25),label.upper(),19,VIOLET)
            text(d,(x+32,y+72),body,35,bold=True)
            d.ellipse((x+895,y+28,x+907,y+40),fill=MINT)
        text(d,(68,1150),'One place to pick up the thread.',33,MUTED)
    elif index == 2:
        text(d,(64,190+rise),'Find it again.',76,bold=True)
        card(d,(64,310,1016,392),fill='#20243a')
        query='weekend studio'; typed=query[:min(len(query),int(max(0,u-.35)*15))]
        d.ellipse((89,335,109,355),outline=MUTED,width=2); d.line((108,354,118,365),fill=MUTED,width=2)
        text(d,(139,326),typed,31)
        if u<1.7: d.line((143+d.textlength(typed,font=font(31)),330,143+d.textlength(typed,font=font(31)),366),fill=VIOLET,width=2)
        offset=int(70*(1-ease((u-.8)/.9)))
        shot=capture.resize((650,717),Image.Resampling.LANCZOS)
        card(d,(202,438+offset,878,1181+offset),edge='#645987',radius=20)
        im.paste(shot,(215,451+offset))
        text(d,(540,1200),'ACTUAL APP • FICTIONAL NOTES',18,MUTED,anchor='ma')
    elif index == 3:
        text(d,(64,205+rise),'You get a say.',76,bold=True)
        text(d,(68,310),'Use manual approval to review changes.',32,MUTED)
        card(d,(64,462,1016,1038))
        text(d,(104,506),'WEEKEND STUDIO',21,VIOLET)
        text(d,(104,570),'A proposed update',44,bold=True)
        text(d,(104,662),'Publish one story each week.',34)
        text(d,(104,718),'Start with the three photos you picked.',32,MUTED)
        for i,(label,color) in enumerate([('Review',VIOLET),('Keep',MINT),('Undo',MUTED)]):
            x=104+i*287
            card(d,(x,860,x+250,954),fill='#24283e',edge=color)
            text(d,(x+125,884),label,30,color,anchor='ma')
        q=ease((u-1)/1.3)
        d.line((104,803,104+850*q,803),fill=MINT,width=3)
        text(d,(68,1127),'Your notes stay readable as Markdown.',33,MUTED)
        text(d,(68,1180),'ILLUSTRATED WORKFLOW',17,MUTED)
    else:
        icon(im,454,250+rise,190)
        text(d,(540,500),'Xenolith',92,bold=True,anchor='ma')
        text(d,(540,628),'Shared context for SI.',40,anchor='ma')
        text(d,(540,730),'A little more continuity.',43,VIOLET,anchor='ma')
        text(d,(540,798),'A little less explaining it again.',38,MUTED,anchor='ma')
        card(d,(155,960,925,1052),edge='#5c527a')
        text(d,(540,984),'github.com/willctl/xenonet',32,anchor='ma')
        text(d,(540,1130),'Xenolith is part of XenoNet. See if it fits.',29,MUTED,anchor='ma')
    return im

def frame(t):
    i=min(4,int(t/6.4)); u=t-i*6.4
    im=scene(i,u,t)
    if u<.48 and i>0:
        im=Image.blend(scene(i-1,6.4+u,t),im,ease(u/.48))
    if t<.65: im=Image.blend(Image.new('RGB',(W,H),'#080c17'),im,ease(t/.65))
    return im

# Original, quiet instrumental bed: soft chords, a pluck and a light pulse. No samples.
rate=48000
times=np.arange(rate*DURATION)/rate
audio=np.zeros_like(times)
for start,notes in [(0,[146.83,220,261.63]),(8,[130.81,196,246.94]),(16,[174.61,220,261.63]),(24,[146.83,220,293.66])]:
    z=times-start
    env=np.clip(z/1.2,0,1)*np.clip((9-z)/2,0,1)*(z>=0)
    for freq in notes: audio+=.035*env*np.sin(2*np.pi*freq*times+.1*np.sin(times*.7))
for beat in np.arange(.5,31,.5):
    z=times-beat; env=np.exp(-np.maximum(z,0)*8)*(z>=0)
    freq=[440,523.25,659.25,587.33][int(beat*2)%4]
    audio+=.026*env*np.sin(2*np.pi*freq*z)
audio*=np.minimum(1,times/2)*np.minimum(1,(DURATION-times)/2)
stereo=np.stack([audio,audio*.94],axis=1)
wav=OUT/'soundtrack.wav'
with wave.open(str(wav),'wb') as f:
    f.setnchannels(2);f.setsampwidth(2);f.setframerate(rate);f.writeframes((stereo*32767).astype('<i2').tobytes())
ffmpeg=imageio_ffmpeg.get_ffmpeg_exe()
cmd=[ffmpeg,'-y','-loglevel','error','-f','rawvideo','-vcodec','rawvideo','-pix_fmt','rgb24','-s',f'{W}x{H}','-r',str(FPS),'-i','-','-i',str(wav),'-c:v','libx264','-preset','medium','-crf','20','-pix_fmt','yuv420p','-c:a','aac','-b:a','160k','-map_metadata','-1','-metadata','title=Xenolith: a little less starting over','-movflags','+faststart','-shortest',str(OUT/'xenolith-demo.mp4')]
proc=subprocess.Popen(cmd,stdin=subprocess.PIPE)
try:
    for n in range(FPS*DURATION):
        im=frame(n/FPS)
        proc.stdin.write(im.tobytes())
        if n in [72,216,384,528,696]: im.save(OUT/f'frame-{n:03}.png')
        if n==72: im.save(OUT/'poster.png')
finally:
    proc.stdin.close()
if proc.wait()!=0: raise SystemExit('Video encoding failed')
wav.unlink()
print(f'Rendered {DURATION}s, {W}x{H}, {FPS}fps')
