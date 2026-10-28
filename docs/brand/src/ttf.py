"""Minimal TrueType glyf -> SVG path extractor (no deps)."""
import struct
class TTF:
    def __init__(self, path):
        self.d = d = open(path, 'rb').read()
        n = struct.unpack('>H', d[4:6])[0]
        self.t = {}
        for i in range(n):
            tag, cs, off, ln = struct.unpack('>4sIII', d[12+16*i:28+16*i])
            self.t[tag.decode()] = (off, ln)
        h = self.t['head'][0]
        self.upm = struct.unpack('>H', d[h+18:h+20])[0]
        self.locfmt = struct.unpack('>h', d[h+50:h+52])[0]
        self.ng = struct.unpack('>H', d[self.t['maxp'][0]+4:self.t['maxp'][0]+6])[0]
        hh = self.t['hhea'][0]
        self.nhm = struct.unpack('>H', d[hh+34:hh+36])[0]
        self.asc, self.desc = struct.unpack('>hh', d[hh+4:hh+8])
        self._cmap()
    def _cmap(self):
        d = self.d; c = self.t['cmap'][0]
        n = struct.unpack('>H', d[c+2:c+4])[0]
        self.cmap = {}
        for i in range(n):
            pid, eid, off = struct.unpack('>HHI', d[c+4+8*i:c+12+8*i])
            o = c + off; fmt = struct.unpack('>H', d[o:o+2])[0]
            if fmt == 4:
                segx2 = struct.unpack('>H', d[o+6:o+8])[0]; s = segx2//2
                ends = struct.unpack('>%dH' % s, d[o+14:o+14+segx2])
                starts = struct.unpack('>%dH' % s, d[o+16+segx2:o+16+2*segx2])
                deltas = struct.unpack('>%dh' % s, d[o+16+2*segx2:o+16+3*segx2])
                ro_off = o+16+3*segx2
                ros = struct.unpack('>%dH' % s, d[ro_off:ro_off+segx2])
                for k in range(s):
                    for ch in range(starts[k], ends[k]+1):
                        if ch == 0xFFFF: continue
                        if ros[k] == 0: g = (ch + deltas[k]) & 0xFFFF
                        else:
                            a = ro_off + 2*k + ros[k] + 2*(ch-starts[k])
                            g = struct.unpack('>H', d[a:a+2])[0]
                            if g: g = (g + deltas[k]) & 0xFFFF
                        self.cmap.setdefault(ch, g)
    def adv(self, g):
        h = self.t['hmtx'][0]; i = min(g, self.nhm-1)
        return struct.unpack('>H', self.d[h+4*i:h+4*i+2])[0]
    def _loc(self, g):
        l = self.t['loca'][0]
        if self.locfmt == 0:
            a, b = struct.unpack('>HH', self.d[l+2*g:l+2*g+4]); return a*2, b*2
        return struct.unpack('>II', self.d[l+4*g:l+4*g+8])
    def contours(self, g, dx=0, dy=0):
        d = self.d; a, b = self._loc(g)
        if a == b: return []
        o = self.t['glyf'][0] + a
        nc = struct.unpack('>h', d[o:o+2])[0]
        if nc < 0:
            res = []; p = o + 10
            while True:
                flags, gi = struct.unpack('>HH', d[p:p+4]); p += 4
                if flags & 1: ax, ay = struct.unpack('>hh', d[p:p+4]); p += 4
                else: ax, ay = struct.unpack('>bb', d[p:p+2]); p += 2
                if flags & 8: p += 2
                elif flags & 0x40: p += 4
                elif flags & 0x80: p += 8
                res += self.contours(gi, dx+ax, dy+ay)
                if not flags & 0x20: break
            return res
        ends = struct.unpack('>%dH' % nc, d[o+10:o+10+2*nc]); p = o+10+2*nc
        il = struct.unpack('>H', d[p:p+2])[0]; p += 2 + il
        npts = ends[-1] + 1; flags = []
        while len(flags) < npts:
            f = d[p]; p += 1; flags.append(f)
            if f & 8:
                r = d[p]; p += 1; flags += [f]*r
        xs = []; v = 0
        for f in flags:
            if f & 2:
                dv = d[p]; p += 1; v += dv if f & 16 else -dv
            elif not f & 16:
                v += struct.unpack('>h', d[p:p+2])[0]; p += 2
            xs.append(v)
        ys = []; v = 0
        for f in flags:
            if f & 4:
                dv = d[p]; p += 1; v += dv if f & 32 else -dv
            elif not f & 32:
                v += struct.unpack('>h', d[p:p+2])[0]; p += 2
            ys.append(v)
        res = []; s = 0
        for e in ends:
            res.append([(xs[i]+dx, ys[i]+dy, flags[i] & 1) for i in range(s, e+1)]); s = e+1
        return res
def path(contours, x, y, scale):
    """Contours (font units, y up) -> SVG path d, placed with origin (x, baseline y)."""
    T = lambda px, py: (x + px*scale, y - py*scale)
    F = lambda v: ("%.2f" % v).rstrip('0').rstrip('.')
    out = []
    for c in contours:
        n = len(c)
        if not n: continue
        # find starting on-curve point
        st = next((i for i in range(n) if c[i][2]), None)
        if st is None:
            p0 = ((c[0][0]+c[1][0])/2, (c[0][1]+c[1][1])/2); pts = c; start = p0
            seq = c
        else:
            pts = c[st:] + c[:st]; start = pts[0][:2]; seq = pts[1:]
        sx, sy = T(*start); cmd = ["M%s %s" % (F(sx), F(sy))]
        prev_off = None
        for (px, py, on) in seq + [(start[0], start[1], 1)]:
            if on:
                X, Y = T(px, py)
                if prev_off: cmd.append("Q%s %s %s %s" % (F(prev_off[0]), F(prev_off[1]), F(X), F(Y)))
                else: cmd.append("L%s %s" % (F(X), F(Y)))
                prev_off = None
            else:
                if prev_off:
                    mx, my = (prev_off[0] + T(px,py)[0])/2, (prev_off[1] + T(px,py)[1])/2
                    cmd.append("Q%s %s %s %s" % (F(prev_off[0]), F(prev_off[1]), F(mx), F(my)))
                prev_off = T(px, py)
        cmd.append("Z"); out.append("".join(cmd))
    return " ".join(out)
