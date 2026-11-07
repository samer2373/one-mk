//! Frosted glass: a panel shows what is behind it, blurred. egui has no
//! backdrop blur, so this is a paint callback that copies the pixels
//! behind the panel from the window at a quarter size, shrinks them to an
//! eighth, blurs them twice there and draws them back inside the panel's
//! rounded shape. The tint, edges and contents are ordinary egui shapes
//! drawn after it.
//!
//! It costs nothing at rest: the window only paints when something
//! changes, and each panel keeps its blurred picture (an eighth of its
//! size, a few kilobytes) until the window says what is behind the panels
//! changed (`changed()`) or the panel moves. A hover or a keystroke reuses
//! it: one textured rectangle. The working textures are a quarter and two
//! eighths of the window, about 1.5 MB on a 2400x1560 window.
//!
//! Anything that fails (an old OpenGL without framebuffer blits, a shader
//! that won't compile, a GL error) turns the blur off for good and panels
//! fall back to a near-solid fill: `works()` says which to draw. So does
//! `ONE_MK_NO_GLASS=1`, for people who want no transparency at all.

use eframe::egui_glow::{self, ShaderVersion};
use eframe::glow::{self, HasContext};
use egui::Rect;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// How far outside a panel the blur reads, in pixels, so its edges blur
/// what is beside them rather than a hard border.
const MARGIN_PX: i32 = 48;

/// How a piece of glass is tinted (premultiplied, top and bottom) and how
/// strongly its rim bends what is behind (0: flat; 1: a thick lens).
#[derive(Clone, Copy, Debug)]
pub struct Look {
    pub top: egui::Color32,
    pub bottom: egui::Color32,
    pub bend: f32,
}

#[derive(Clone)]
pub struct Glass {
    res: Arc<Mutex<Option<Res>>>,
    ok: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    frame: Arc<AtomicU64>,
}

impl Default for Glass {
    fn default() -> Self {
        let off = refused();
        Self { res: Default::default(), ok: Arc::new(AtomicBool::new(!off)), generation: Default::default(), frame: Default::default() }
    }
}

/// No transparency at all: `ONE_MK_NO_GLASS=1`, or the desktop's own
/// "Reduce transparency" setting (macOS accessibility).
pub fn refused() -> bool {
    std::env::var_os("ONE_MK_NO_GLASS").is_some_and(|v| v != "0") || crate::chrome::reduce_transparency()
}

impl Glass {
    /// Whether panels are blurred (else they are drawn near-solid).
    pub fn works(&self) -> bool {
        self.ok.load(Ordering::Relaxed)
    }

    /// What is behind the panels changed: blur again at the next paint.
    pub fn changed(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    /// A new frame starts: blurs no panel used last frame are let go.
    pub fn frame(&self) {
        self.frame.fetch_add(1, Ordering::Relaxed);
    }

    /// The glass for panel `key` at `rect` with corner `radius` points (the
    /// backdrop bent and blurred, `look`'s tint, the lit rim), as shapes to
    /// place; draw its contents after them.
    pub fn backdrop_shapes(&self, out: &mut Vec<egui::Shape>, key: u64, rect: Rect, radius: f32, look: Look) {
        if !self.works() || !rect.is_positive() {
            return;
        }
        let (res, ok) = (self.res.clone(), self.ok.clone());
        let generation = self.generation.load(Ordering::Relaxed);
        let frame = self.frame.load(Ordering::Relaxed);
        let cb = egui_glow::CallbackFn::new(move |info, painter| {
            if !ok.load(Ordering::Relaxed) {
                return;
            }
            let gl = painter.gl();
            let mut guard = res.lock().unwrap_or_else(|e| e.into_inner());
            if guard.is_none() {
                match unsafe { Res::new(gl) } {
                    Ok(r) => *guard = Some(r),
                    Err(e) => {
                        log_once(&e);
                        ok.store(false, Ordering::Relaxed);
                        return;
                    }
                }
            }
            let Some(r) = guard.as_mut() else { return };
            r.evict(gl, frame);
            if let Err(e) = unsafe { r.paint(gl, &info, radius, look, key, generation, frame) } {
                log_once(&e);
                ok.store(false, Ordering::Relaxed);
            }
        });
        out.push(egui::Shape::Callback(egui::PaintCallback { rect, callback: Arc::new(cb) }));
    }
}

fn log_once(e: &str) {
    static SAID: AtomicBool = AtomicBool::new(false);
    if !SAID.swap(true, Ordering::Relaxed) {
        eprintln!("one-mk-app: glass panels are drawn solid ({e})");
    }
}

/// Working textures that cover the window at 1/4 and 1/8 size (two at
/// 1/8, to blur between; each pass works on one panel's part only), and
/// each panel's finished picture.
struct Res {
    blur: glow::Program,
    show: glow::Program,
    _vbo: glow::Buffer,
    vao: glow::VertexArray,
    fbo: glow::Framebuffer,
    size: [i32; 2],
    tex: Vec<(glow::Texture, [i32; 2])>,
    /// Per panel: its picture, where it was read from, the window size
    /// and the generation it was made at.
    kept: HashMap<u64, Kept>,
    /// The window is multisampled (some drivers force it): its pixels can
    /// only be copied at full size, into this, then shrunk.
    full: Option<(glow::Texture, [i32; 2])>,
    multisampled: bool,
    u_blur_tex: Option<glow::UniformLocation>,
    u_blur_dir: Option<glow::UniformLocation>,
    u_blur_src: Option<glow::UniformLocation>,
    u_show_tex: Option<glow::UniformLocation>,
    u_show_src: Option<glow::UniformLocation>,
    u_show_size: Option<glow::UniformLocation>,
    u_show_radius: Option<glow::UniformLocation>,
    u_show_sharp: Option<glow::UniformLocation>,
    u_show_texel: Option<glow::UniformLocation>,
    u_show_ppp: Option<glow::UniformLocation>,
    u_show_tint_top: Option<glow::UniformLocation>,
    u_show_tint_bottom: Option<glow::UniformLocation>,
    u_show_bend: Option<glow::UniformLocation>,
    u_show_mask: Option<glow::UniformLocation>,
}

struct Kept {
    /// The frame it was last drawn in.
    used: u64,
    tex: glow::Texture,
    /// What is behind at a quarter size, unblurred: the lens at the rim.
    sharp: glow::Texture,
    texels: [i32; 2],
    region: Px,
    screen: [i32; 2],
    generation: u64,
}

const VERT: &str = r"
IN vec2 a_pos;
OUT vec2 v_uv;
void main() {
    v_uv = a_pos * 0.5 + 0.5;
    gl_Position = vec4(a_pos, 0.0, 1.0);
}";

// Nine-tap Gaussian folded into five linear samples. `u_src` is the part
// of the texture being worked on (xy origin, zw size, in UV); samples stay
// inside it so stale pixels around it never bleed in.
const BLUR: &str = r"
uniform sampler2D u_tex;
uniform vec2 u_dir;
uniform vec4 u_src;
IN vec2 v_uv;
OUT_COLOR
vec4 at(vec2 uv) {
    return TEXTURE(u_tex, clamp(uv, u_src.xy, u_src.xy + u_src.zw));
}
void main() {
    vec2 uv = u_src.xy + v_uv * u_src.zw;
    vec4 c = at(uv) * 0.2270270270;
    c += at(uv + u_dir * 1.3846153846) * 0.3162162162;
    c += at(uv - u_dir * 1.3846153846) * 0.3162162162;
    c += at(uv + u_dir * 3.2307692308) * 0.0702702703;
    c += at(uv - u_dir * 3.2307692308) * 0.0702702703;
    FRAG_COLOR = c;
}";

// Liquid glass, inside a rounded rectangle, antialiased at its edge, in
// premultiplied alpha (the window's own alpha kept, so a see-through
// window stays see-through behind it):
// - the inside shows what is behind, blurred, a little brighter and more
//   saturated;
// - towards the rim the glass bends light like a lens: it shows the less
//   blurred picture, pulled in from outside the edge, with the colours
//   parting very slightly;
// - its tint, lighter at the top;
// - light catches the rim, strongest where it faces the light (top left)
//   and again, softer, on the opposite edge, as in a real slab of glass.
const SHOW: &str = r"
uniform sampler2D u_tex;
uniform sampler2D u_sharp;
uniform vec4 u_src;
uniform vec2 u_size;
uniform float u_radius;
uniform vec2 u_texel;
uniform float u_ppp;
uniform vec4 u_tint_top;
uniform vec4 u_tint_bottom;
uniform float u_bend;
uniform float u_mask;
IN vec2 v_uv;
OUT_COLOR
float sdf(vec2 p) {
    vec2 q = abs(p - u_size * 0.5) - (u_size * 0.5 - vec2(u_radius));
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - u_radius;
}
void main() {
    vec2 p = v_uv * u_size;
    float d = sdf(p);
    float a = clamp(0.5 - d, 0.0, 1.0);
    vec2 h = vec2(0.75, 0.0);
    vec2 n = vec2(sdf(p + h.xy) - sdf(p - h.xy), sdf(p + h.yx) - sdf(p - h.yx));
    n = n / max(length(n), 1e-4);
    float band = max(u_radius * 0.9, 10.0 * u_ppp);
    float t = clamp(1.0 + d / band, 0.0, 1.0);
    float bend = t * t * t;
    vec2 uv = u_src.xy + v_uv * u_src.zw;
    vec2 off = n * bend * u_bend * band * u_texel;
    vec4 soft = TEXTURE(u_tex, uv + off * 0.7);
    vec4 sr = TEXTURE(u_sharp, uv + off * 1.08);
    vec4 sg = TEXTURE(u_sharp, uv + off);
    vec4 sb = TEXTURE(u_sharp, uv + off * 0.92);
    vec4 sharp = vec4(sr.r, sg.g, sb.b, sg.a);
    vec4 c = mix(soft, sharp, 0.15 + 0.6 * bend);
    float l = dot(c.rgb, vec3(0.2126, 0.7152, 0.0722));
    c.rgb = max(mix(vec3(l), c.rgb, 1.35), 0.0) * (1.04 + 0.08 * bend);
    vec4 tint = mix(u_tint_bottom, u_tint_top, v_uv.y);
    c = tint + c * (1.0 - tint.a);
    vec2 light = normalize(vec2(-0.55, 0.85));
    float facing = max(dot(n, light), 0.0);
    float away = max(dot(n, -light), 0.0);
    float inside = max(-d, 0.0);
    float rim = exp(-inside / (0.9 * u_ppp));
    float sheen = exp(-inside / (5.0 * u_ppp));
    float spec = rim * (0.06 + 0.42 * facing * facing + 0.16 * away * away) + sheen * (0.06 * facing + 0.025 * away);
    c = vec4(c.rgb + vec3(spec), min(c.a + spec, 1.0));
    // Two passes: the first takes the panel's shape out of what is there
    // (its coverage, antialiased), the second puts the glass in. The
    // backdrop it shows is what was there, so it must replace it, not be
    // laid over it a second time.
    FRAG_COLOR = u_mask > 0.5 ? vec4(a) : c * a;
}";

fn source(version: ShaderVersion, body: &str, fragment: bool) -> String {
    let new = version.is_new_shader_interface();
    let precision = if version.is_embedded() { "precision mediump float;\n" } else { "" };
    let body = body
        .replace("IN ", if new { "in " } else if fragment { "varying " } else { "attribute " })
        .replace("OUT vec2", if new { "out vec2" } else { "varying vec2" })
        .replace("OUT_COLOR", if new { "out vec4 o_color;" } else { "" })
        .replace("FRAG_COLOR", if new { "o_color" } else { "gl_FragColor" })
        .replace("TEXTURE(", if new { "texture(" } else { "texture2D(" });
    format!("{}{precision}{body}", version.version_declaration())
}

unsafe fn program(gl: &glow::Context, version: ShaderVersion, frag: &str) -> Result<glow::Program, String> {
    unsafe {
        let p = gl.create_program()?;
        let mut shaders = vec![];
        for (kind, src) in [(glow::VERTEX_SHADER, source(version, VERT, false)), (glow::FRAGMENT_SHADER, source(version, frag, true))] {
            let s = gl.create_shader(kind)?;
            gl.shader_source(s, &src);
            gl.compile_shader(s);
            if !gl.get_shader_compile_status(s) {
                return Err(format!("shader: {}", gl.get_shader_info_log(s)));
            }
            gl.attach_shader(p, s);
            shaders.push(s);
        }
        gl.bind_attrib_location(p, 0, "a_pos");
        gl.link_program(p);
        if !gl.get_program_link_status(p) {
            return Err(format!("link: {}", gl.get_program_info_log(p)));
        }
        for s in shaders {
            gl.detach_shader(p, s);
            gl.delete_shader(s);
        }
        Ok(p)
    }
}

/// A region of the window in pixels: x, y from the bottom, width, height.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Px {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl Px {
    /// The same region on a texture `k` times smaller, rounded outward.
    fn scaled(self, k: i32) -> Px {
        let (x0, y0) = (self.x.div_euclid(k), self.y.div_euclid(k));
        let (x1, y1) = ((self.x + self.w + k - 1) / k, (self.y + self.h + k - 1) / k);
        Px { x: x0, y: y0, w: (x1 - x0).max(1), h: (y1 - y0).max(1) }
    }
}

impl Res {
    unsafe fn new(gl: &glow::Context) -> Result<Self, String> {
        let version = ShaderVersion::get(gl);
        #[cfg(any(debug_assertions, feature = "design-evidence"))]
        if std::env::var_os("ONE_MK_APP_GL").is_some() {
            let (renderer, samples) = unsafe { (gl.get_parameter_string(glow::RENDERER), gl.get_parameter_i32(glow::SAMPLES)) };
            eprintln!("one-mk-app: glass on {renderer}, {samples} samples, {version:?}");
        }
        if !version.is_new_shader_interface() {
            // Framebuffer blits need OpenGL 3 / GLES 3.
            return Err(format!("OpenGL too old ({version:?})"));
        }
        unsafe {
            let blur = program(gl, version, BLUR)?;
            let show = program(gl, version, SHOW)?;
            let vbo = gl.create_buffer()?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let quad: [f32; 12] = [-1.0, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, -1.0, 1.0, 1.0, -1.0, 1.0];
            let bytes: Vec<u8> = quad.iter().flat_map(|f| f.to_ne_bytes()).collect();
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &bytes, glow::STATIC_DRAW);
            let vao = gl.create_vertex_array()?;
            gl.bind_vertex_array(Some(vao));
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);
            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            let fbo = gl.create_framebuffer()?;
            Ok(Self {
                u_blur_tex: gl.get_uniform_location(blur, "u_tex"),
                u_blur_dir: gl.get_uniform_location(blur, "u_dir"),
                u_blur_src: gl.get_uniform_location(blur, "u_src"),
                u_show_tex: gl.get_uniform_location(show, "u_tex"),
                u_show_src: gl.get_uniform_location(show, "u_src"),
                u_show_size: gl.get_uniform_location(show, "u_size"),
                u_show_radius: gl.get_uniform_location(show, "u_radius"),
                u_show_sharp: gl.get_uniform_location(show, "u_sharp"),
                u_show_texel: gl.get_uniform_location(show, "u_texel"),
                u_show_ppp: gl.get_uniform_location(show, "u_ppp"),
                u_show_tint_top: gl.get_uniform_location(show, "u_tint_top"),
                u_show_tint_bottom: gl.get_uniform_location(show, "u_tint_bottom"),
                u_show_bend: gl.get_uniform_location(show, "u_bend"),
                u_show_mask: gl.get_uniform_location(show, "u_mask"),
                blur,
                show,
                _vbo: vbo,
                vao,
                fbo,
                size: [0, 0],
                tex: vec![],
                kept: HashMap::new(),
                full: None,
                multisampled: false,
            })
        }
    }

    /// (Re)makes the textures when the window changes size.
    unsafe fn fit(&mut self, gl: &glow::Context, size: [i32; 2]) -> Result<(), String> {
        if self.size == size && !self.tex.is_empty() {
            return Ok(());
        }
        unsafe {
            for (t, _) in self.tex.drain(..) {
                gl.delete_texture(t);
            }
            for k in [4, 8, 8] {
                let s = [(size[0] + k - 1) / k, (size[1] + k - 1) / k];
                self.tex.push((Self::texture(gl, s)?, s));
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
        self.size = size;
        Ok(())
    }

    /// Lets go of the blurs of panels not drawn in the last two frames
    /// (gone, moved to a new key, scrolled away), so they never pile up.
    fn evict(&mut self, gl: &glow::Context, frame: u64) {
        if self.kept.len() <= 4 && self.kept.values().all(|k| k.used + 2 >= frame) {
            return;
        }
        self.kept.retain(|_, k| {
            let keep = k.used + 2 >= frame;
            if !keep {
                unsafe {
                    gl.delete_texture(k.tex);
                    gl.delete_texture(k.sharp);
                }
            }
            keep
        });
    }

    unsafe fn texture(gl: &glow::Context, s: [i32; 2]) -> Result<glow::Texture, String> {
        unsafe {
            let t = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(t));
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
            gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8 as i32, s[0], s[1], 0, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(None));
            Ok(t)
        }
    }

    unsafe fn target(&self, gl: &glow::Context, t: glow::Texture) {
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(t), 0);
        }
    }

    /// Draws work texture `from` into work texture `to` over `r` (in `to`'s
    /// pixels), blurring along `dir` (in `from`'s texels; zero: a copy).
    unsafe fn pass(&self, gl: &glow::Context, from: usize, to: usize, r: Px, src: [f32; 4], dir: [f32; 2]) {
        unsafe {
            self.target(gl, self.tex[to].0);
            gl.viewport(r.x, r.y, r.w, r.h);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.tex[from].0));
            let s = self.tex[from].1;
            gl.uniform_1_i32(self.u_blur_tex.as_ref(), 0);
            gl.uniform_2_f32(self.u_blur_dir.as_ref(), dir[0] / s[0] as f32, dir[1] / s[1] as f32);
            gl.uniform_4_f32(self.u_blur_src.as_ref(), src[0], src[1], src[2], src[3]);
            gl.draw_arrays(glow::TRIANGLES, 0, 6);
        }
    }

    /// Reads and blurs what is behind `region` into the panel's own texture.
    #[allow(clippy::too_many_arguments)]
    unsafe fn blur_into(&mut self, gl: &glow::Context, screen_fb: Option<glow::Framebuffer>, region: Px, screen: [i32; 2], key: u64, generation: u64, frame: u64) -> Result<(), String> {
        let [sw, sh] = screen;
        let uv = |r: Px| [r.x as f32 / sw as f32, r.y as f32 / sh as f32, r.w as f32 / sw as f32, r.h as f32 / sh as f32];
        let (q, e) = (region.scaled(4), region.scaled(8));
        unsafe {
            self.fit(gl, screen)?;
            let src = uv(region);
            // 1. The window's pixels behind the panel, at a quarter size.
            if !self.multisampled {
                self.target(gl, self.tex[0].0);
                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, screen_fb);
                gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(self.fbo));
                gl.blit_framebuffer(region.x, region.y, region.x + region.w, region.y + region.h, q.x, q.y, q.x + q.w, q.y + q.h, glow::COLOR_BUFFER_BIT, glow::LINEAR);
                // A multisampled window can't be scaled while copied.
                self.multisampled = gl.get_error() != glow::NO_ERROR;
            }
            if self.multisampled {
                if self.full.as_ref().is_none_or(|f| f.1 != screen) {
                    if let Some((t, _)) = self.full.take() {
                        gl.delete_texture(t);
                    }
                    self.full = Some((Self::texture(gl, screen)?, screen));
                }
                let full = self.full.as_ref().map(|f| f.0).ok_or("no texture")?;
                self.target(gl, full);
                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, screen_fb);
                gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(self.fbo));
                gl.blit_framebuffer(region.x, region.y, region.x + region.w, region.y + region.h, region.x, region.y, region.x + region.w, region.y + region.h, glow::COLOR_BUFFER_BIT, glow::NEAREST);
                let err = gl.get_error();
                if err != glow::NO_ERROR {
                    return Err(format!("blit failed (GL error {err:#x})"));
                }
                gl.use_program(Some(self.blur));
                gl.bind_vertex_array(Some(self.vao));
                self.target(gl, self.tex[0].0);
                gl.viewport(q.x, q.y, q.w, q.h);
                gl.active_texture(glow::TEXTURE0);
                gl.bind_texture(glow::TEXTURE_2D, Some(full));
                gl.uniform_1_i32(self.u_blur_tex.as_ref(), 0);
                gl.uniform_2_f32(self.u_blur_dir.as_ref(), 0.0, 0.0);
                gl.uniform_4_f32(self.u_blur_src.as_ref(), src[0], src[1], src[2], src[3]);
                gl.draw_arrays(glow::TRIANGLES, 0, 6);
            }
            // The unblurred quarter, for the lens at the rim.
            let sharp = match self.kept.get(&key).map(|k| (k.sharp, k.region)) {
                Some((t, r)) if r.scaled(4).w == q.w && r.scaled(4).h == q.h => t,
                Some((t, _)) => {
                    gl.delete_texture(t);
                    Self::texture(gl, [q.w, q.h])?
                }
                None => Self::texture(gl, [q.w, q.h])?,
            };
            let out = gl.create_framebuffer()?;
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(self.fbo));
            gl.framebuffer_texture_2d(glow::READ_FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(self.tex[0].0), 0);
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(out));
            gl.framebuffer_texture_2d(glow::DRAW_FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(sharp), 0);
            gl.blit_framebuffer(q.x, q.y, q.x + q.w, q.y + q.h, 0, 0, q.w, q.h, glow::COLOR_BUFFER_BIT, glow::NEAREST);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.delete_framebuffer(out);
            // 2. An eighth, then blurred twice each way.
            gl.use_program(Some(self.blur));
            gl.bind_vertex_array(Some(self.vao));
            self.pass(gl, 0, 1, e, src, [0.0, 0.0]);
            for _ in 0..2 {
                self.pass(gl, 1, 2, e, src, [1.0, 0.0]);
                self.pass(gl, 2, 1, e, src, [0.0, 1.0]);
            }
            // 3. Kept for this panel.
            let texels = [e.w, e.h];
            let kept = match self.kept.remove(&key) {
                Some(k) if k.texels == texels => k.tex,
                Some(k) => {
                    gl.delete_texture(k.tex);
                    Self::texture(gl, texels)?
                }
                None => Self::texture(gl, texels)?,
            };
            let out = gl.create_framebuffer()?;
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(self.fbo));
            gl.framebuffer_texture_2d(glow::READ_FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(self.tex[1].0), 0);
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(out));
            gl.framebuffer_texture_2d(glow::DRAW_FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(kept), 0);
            gl.blit_framebuffer(e.x, e.y, e.x + e.w, e.y + e.h, 0, 0, e.w, e.h, glow::COLOR_BUFFER_BIT, glow::NEAREST);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.delete_framebuffer(out);
            self.kept.insert(key, Kept { used: frame, tex: kept, sharp, texels, region, screen, generation });
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn paint(&mut self, gl: &glow::Context, info: &egui::PaintCallbackInfo, radius: f32, look: Look, key: u64, generation: u64, frame: u64) -> Result<(), String> {
        let screen = [info.screen_size_px[0] as i32, info.screen_size_px[1] as i32];
        let [sw, sh] = screen;
        if sw <= 0 || sh <= 0 {
            return Ok(());
        }
        let v = info.viewport_in_pixels();
        let clip = info.clip_rect_in_pixels();
        let panel = Px { x: v.left_px, y: v.from_bottom_px, w: v.width_px, h: v.height_px };
        // The part read: the panel and a margin round it, on whole eighths,
        // inside the window.
        let snap = |v: i32, up: bool| if up { (v + 7) / 8 * 8 } else { v.div_euclid(8) * 8 };
        let x0 = snap(panel.x - MARGIN_PX, false).max(0);
        let y0 = snap(panel.y - MARGIN_PX, false).max(0);
        let x1 = snap(panel.x + panel.w + MARGIN_PX, true).min(sw);
        let y1 = snap(panel.y + panel.h + MARGIN_PX, true).min(sh);
        if x1 <= x0 || y1 <= y0 {
            return Ok(());
        }
        let region = Px { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
        unsafe {
            let screen_fb = gl.get_parameter_framebuffer(glow::DRAW_FRAMEBUFFER_BINDING);
            let fresh = self.kept.get(&key).is_some_and(|k| k.region == region && k.screen == screen && k.generation == generation);
            if !fresh {
                gl.disable(glow::SCISSOR_TEST);
                gl.disable(glow::BLEND);
                let done = self.blur_into(gl, screen_fb, region, screen, key, generation, frame);
                gl.bind_framebuffer(glow::FRAMEBUFFER, screen_fb);
                done?;
            }
            let Some(k) = self.kept.get_mut(&key) else { return Ok(()) };
            k.used = frame;
            let k = &*k;
            // Back onto the window, inside the panel's shape and clip.
            let e = region.scaled(8);
            let (ex, ey, ew, eh) = ((e.x * 8) as f32, (e.y * 8) as f32, (e.w * 8) as f32, (e.h * 8) as f32);
            let src = [(panel.x as f32 - ex) / ew, (panel.y as f32 - ey) / eh, panel.w as f32 / ew, panel.h as f32 / eh];
            gl.bind_framebuffer(glow::FRAMEBUFFER, screen_fb);
            gl.viewport(panel.x, panel.y, panel.w, panel.h);
            gl.enable(glow::SCISSOR_TEST);
            gl.scissor(clip.left_px, clip.from_bottom_px, clip.width_px, clip.height_px);
            gl.enable(glow::BLEND);
            gl.blend_func_separate(glow::ONE, glow::ONE_MINUS_SRC_ALPHA, glow::ONE_MINUS_DST_ALPHA, glow::ONE);
            gl.use_program(Some(self.show));
            gl.bind_vertex_array(Some(self.vao));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(k.tex));
            gl.uniform_1_i32(self.u_show_tex.as_ref(), 0);
            gl.uniform_4_f32(self.u_show_src.as_ref(), src[0], src[1], src[2], src[3]);
            gl.uniform_2_f32(self.u_show_size.as_ref(), panel.w as f32, panel.h as f32);
            gl.uniform_1_f32(self.u_show_radius.as_ref(), radius * info.pixels_per_point);
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(k.sharp));
            gl.uniform_1_i32(self.u_show_sharp.as_ref(), 1);
            gl.uniform_2_f32(self.u_show_texel.as_ref(), 1.0 / region.w as f32, 1.0 / region.h as f32);
            gl.uniform_1_f32(self.u_show_ppp.as_ref(), info.pixels_per_point);
            // In gamma space, as the window's own pixels (and egui's
            // blending) are.
            let [t, b] = [look.top, look.bottom].map(|c| c.to_normalized_gamma_f32());
            gl.uniform_4_f32(self.u_show_tint_top.as_ref(), t[0], t[1], t[2], t[3]);
            gl.uniform_4_f32(self.u_show_tint_bottom.as_ref(), b[0], b[1], b[2], b[3]);
            gl.uniform_1_f32(self.u_show_bend.as_ref(), look.bend);
            // Out with what is under the panel (dst × (1 − coverage)), then
            // in with the glass (+ glass × coverage).
            gl.blend_func_separate(glow::ZERO, glow::ONE_MINUS_SRC_ALPHA, glow::ZERO, glow::ONE_MINUS_SRC_ALPHA);
            gl.uniform_1_f32(self.u_show_mask.as_ref(), 1.0);
            gl.draw_arrays(glow::TRIANGLES, 0, 6);
            gl.blend_func_separate(glow::ONE, glow::ONE, glow::ONE, glow::ONE);
            gl.uniform_1_f32(self.u_show_mask.as_ref(), 0.0);
            gl.draw_arrays(glow::TRIANGLES, 0, 6);
            gl.blend_func_separate(glow::ONE, glow::ONE_MINUS_SRC_ALPHA, glow::ONE_MINUS_DST_ALPHA, glow::ONE);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_vertex_array(None);
            gl.bind_texture(glow::TEXTURE_2D, None);
            let err = gl.get_error();
            if err != glow::NO_ERROR {
                return Err(format!("GL error {err:#x}"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regions_scale_outward_and_never_vanish() {
        let r = Px { x: 13, y: 7, w: 101, h: 3 };
        let s = r.scaled(8);
        assert_eq!((s.x, s.y), (1, 0));
        assert!(s.x * 8 <= r.x && (s.x + s.w) * 8 >= r.x + r.w);
        assert!(s.y * 8 <= r.y && (s.y + s.h) * 8 >= r.y + r.h);
        assert!(Px { x: 0, y: 0, w: 1, h: 1 }.scaled(8).w >= 1);
    }

    #[test]
    fn shaders_read_both_interfaces() {
        for v in [ShaderVersion::Gl140, ShaderVersion::Es300] {
            let f = source(v, SHOW, true);
            assert!(f.contains("in vec2 v_uv") && f.contains("out vec4 o_color") && f.contains("texture(") && !f.contains("TEXTURE("));
        }
        let old = source(ShaderVersion::Gl120, BLUR, true);
        assert!(old.contains("varying vec2 v_uv") && old.contains("gl_FragColor") && old.contains("texture2D("));
    }
}
