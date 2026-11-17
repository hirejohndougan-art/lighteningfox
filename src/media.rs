//! `<video>` and `<audio>` playback.
//!
//! Decoding is done by the system's `ffmpeg` and `ffprobe` programs (any build from the last
//! decade plays MP4/H.264, WebM/VP8/VP9/AV1, Ogg/Theora/Vorbis/Opus, MP3, FLAC, WAV, AAC…),
//! so Ironswift needs no media libraries at build time. `IRONSWIFT_FFMPEG` and
//! `IRONSWIFT_FFPROBE` point at other copies. Sound goes to the first of `pw-play`
//! (PipeWire), `pacat` (PulseAudio) or `aplay` (ALSA) that is installed.
//!
//! The JavaScript side (`media.js`) is HTMLMediaElement: the load algorithm, the playback
//! clock, events and the controls' clicks. It calls the `__md*` natives below, which keep each
//! element's decoder processes and pictures here, in the page's script engine. Before each
//! layout `publish` hands changed pictures to the page's images as `media:<node id>`: what the
//! element's box shows (the frame or poster, fitted, with the controls drawn over it), drawn
//! when it's painted at the box's size. Its size in the images is the video's own (the
//! poster's until that's known), which layout uses as the element's natural size.
//!
//! Media is downloaded whole before it plays (no streaming or Media Source Extensions yet).

use crate::dom::NodeId;
use crate::images::Images;
use boa_engine::object::builtins::JsArray;
use boa_engine::{Context, JsResult, JsString, JsValue, NativeFunction, Source};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, OnceLock};
use tiny_skia::{Color, FillRule, FilterQuality, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Transform};

/// Frames are decoded at most this wide (bigger videos are scaled down) and this often.
const MAX_FRAME_W: u32 = 1920;
const MAX_FPS: f64 = 30.0;
/// Height of the controls bar, in CSS px. `media.js` hit-tests the same layout.
pub const BAR_H: f32 = 32.0;
/// The biggest media file loaded (it's held in memory while it downloads).
const MAX_BYTES: usize = 1 << 30;
/// Sound is decoded to this format for the sound server.
const RATE: u32 = 48_000;

// ---------------------------------------------------------------- the decoder programs

fn find_tool(name: &str, env: &str) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os(env).filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    on_path(name)
}

fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| p.is_file())
}

fn ffmpeg() -> Option<&'static Path> {
    static F: OnceLock<Option<PathBuf>> = OnceLock::new();
    F.get_or_init(|| find_tool("ffmpeg", "IRONSWIFT_FFMPEG")).as_deref()
}

fn ffprobe() -> Option<&'static Path> {
    static F: OnceLock<Option<PathBuf>> = OnceLock::new();
    F.get_or_init(|| find_tool("ffprobe", "IRONSWIFT_FFPROBE")).as_deref()
}

/// Can media play at all here?
pub fn available() -> bool {
    ffmpeg().is_some() && ffprobe().is_some()
}

/// The program that takes raw 16-bit stereo sound on its standard input and plays it.
fn sound_sink() -> Option<Command> {
    static S: OnceLock<Option<(PathBuf, Vec<String>)>> = OnceLock::new();
    let (prog, args) = S
        .get_or_init(|| {
            let rate = RATE.to_string();
            let options: [(&str, Vec<String>); 3] = [
                ("pw-play", vec!["--format".into(), "s16".into(), format!("--rate={rate}"), "--channels=2".into(), "-".into()]),
                ("pacat", vec!["--raw".into(), "--format=s16le".into(), format!("--rate={rate}"), "--channels=2".into()]),
                ("aplay", vec!["-q".into(), "-t".into(), "raw".into(), "-f".into(), "S16_LE".into(), "-r".into(), rate.clone(), "-c".into(), "2".into(), "-".into()]),
            ];
            options.into_iter().find_map(|(name, args)| on_path(name).map(|p| (p, args)))
        })
        .as_ref()?;
    let mut c = Command::new(prog);
    c.args(args);
    Some(c)
}

fn quiet(c: &mut Command) -> &mut Command {
    c.stdin(Stdio::null()).stderr(Stdio::null())
}

// ---------------------------------------------------------------- probing

/// What ffprobe says about a media file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Info {
    /// Seconds (NaN if unknown, as for a live stream).
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub video: bool,
    pub audio: bool,
}

fn parse_rate(s: &str) -> Option<f64> {
    let (n, d) = s.split_once('/').unwrap_or((s, "1"));
    let (n, d) = (n.trim().parse::<f64>().ok()?, d.trim().parse::<f64>().ok()?);
    (d > 0.0 && n > 0.0).then_some(n / d)
}

/// Read `ffprobe -of compact` output (`stream|codec_type=video|width=…` lines).
pub fn parse_probe(out: &str) -> Option<Info> {
    let mut info = Info { duration: f64::NAN, ..Default::default() };
    let mut stream_duration = f64::NAN;
    for line in out.lines() {
        let mut parts = line.split('|');
        let section = parts.next().unwrap_or("");
        let kv: HashMap<&str, &str> = parts.filter_map(|p| p.split_once('=')).collect();
        match section {
            "stream" => match kv.get("codec_type").copied() {
                Some("video") if !info.video => {
                    // An album cover inside an MP3 is a one-picture "video" stream; it's shown
                    // like a poster, which is what other browsers do too.
                    let (w, h) = (kv.get("width").and_then(|v| v.parse().ok()).unwrap_or(0), kv.get("height").and_then(|v| v.parse().ok()).unwrap_or(0));
                    if w > 0 && h > 0 {
                        info.video = true;
                        info.width = w;
                        info.height = h;
                        info.fps = kv.get("avg_frame_rate").and_then(|r| parse_rate(r)).or_else(|| kv.get("r_frame_rate").and_then(|r| parse_rate(r))).unwrap_or(25.0);
                        if let Some(d) = kv.get("duration").and_then(|d| d.parse::<f64>().ok()) {
                            stream_duration = d;
                        }
                    }
                }
                Some("audio") => {
                    info.audio = true;
                    if let Some(d) = kv.get("duration").and_then(|d| d.parse::<f64>().ok()) {
                        stream_duration = if stream_duration.is_nan() { d } else { stream_duration.max(d) };
                    }
                }
                _ => {}
            },
            "format" => {
                if let Some(d) = kv.get("duration").and_then(|d| d.parse::<f64>().ok()) {
                    info.duration = d;
                }
            }
            _ => {}
        }
    }
    if info.duration.is_nan() {
        info.duration = stream_duration;
    }
    (info.video || info.audio).then_some(info)
}

fn probe(file: &Path) -> Result<Info, String> {
    let prog = ffprobe().ok_or("ffprobe isn't installed")?;
    let out = quiet(Command::new(prog).args(["-v", "error", "-show_entries", "format=duration:stream=codec_type,width,height,avg_frame_rate,r_frame_rate,duration", "-of", "compact"]).arg(file))
        .stdout(Stdio::piped())
        .output()
        .map_err(|e| format!("ffprobe: {e}"))?;
    parse_probe(&String::from_utf8_lossy(&out.stdout)).ok_or_else(|| "not a media file this browser can play".to_string())
}

/// The size frames are decoded at: the video's own, scaled down to at most MAX_FRAME_W wide.
pub fn frame_size(info: &Info) -> (u32, u32) {
    let (w, h) = (info.width.max(1), info.height.max(1));
    if w <= MAX_FRAME_W {
        return (w, h);
    }
    (MAX_FRAME_W, ((h as u64 * MAX_FRAME_W as u64 / w as u64) as u32).max(1))
}

/// ffmpeg's `atempo` takes 0.5–2.0, so other speeds are a chain of them.
pub fn atempo(rate: f64) -> Option<String> {
    if !(rate.is_finite() && rate > 0.0) || (rate - 1.0).abs() < 1e-6 {
        return None;
    }
    let mut parts = Vec::new();
    let mut r = rate.clamp(1.0 / 16.0, 16.0);
    while r > 2.0 {
        parts.push("atempo=2".to_string());
        r /= 2.0;
    }
    while r < 0.5 {
        parts.push("atempo=0.5".to_string());
        r /= 0.5;
    }
    parts.push(format!("atempo={r:.4}"));
    Some(parts.join(","))
}

// ---------------------------------------------------------------- decoder processes

/// A downloaded media file, deleted when the element lets go of it.
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn temp_path() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!("ironswift-media-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)))
}

fn kill(c: &mut Child) {
    let _ = c.kill();
    let _ = c.wait();
}

type Frame = (f64, image::RgbaImage);

/// An ffmpeg turning the video from time `start` on into RGBA frames, a few frames ahead.
struct VideoDecoder {
    child: Child,
    rx: Receiver<Frame>,
    pending: Option<Frame>,
    fps: f64,
}

impl VideoDecoder {
    fn start(file: &Path, info: &Info, start: f64) -> Option<VideoDecoder> {
        let (w, h) = frame_size(info);
        let fps = info.fps.clamp(1.0, MAX_FPS);
        let mut child = quiet(Command::new(ffmpeg()?).args(["-nostdin", "-loglevel", "error", "-ss", &format!("{:.3}", start.max(0.0)), "-i"]).arg(file).args([
            "-an",
            "-sn",
            "-vf",
            &format!("fps={fps},scale={w}:{h}"),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-",
        ]))
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
        let mut out = child.stdout.take()?;
        let (tx, rx) = mpsc::sync_channel::<Frame>(3);
        std::thread::Builder::new()
            .name("media-video".into())
            .spawn(move || {
                let size = (w * h * 4) as usize;
                for n in 0u64.. {
                    let mut buf = vec![0u8; size];
                    if out.read_exact(&mut buf).is_err() {
                        break;
                    }
                    let Some(img) = image::RgbaImage::from_raw(w, h, buf) else { break };
                    if tx.send((start + n as f64 / fps, img)).is_err() {
                        break;
                    }
                }
            })
            .ok()?;
        Some(VideoDecoder { child, rx, pending: None, fps })
    }

    /// The newest decoded frame due by media time `t` (late frames are skipped).
    fn take_due(&mut self, t: f64) -> Option<Frame> {
        let mut best = None;
        loop {
            let next = match self.pending.take() {
                Some(f) => f,
                None => match self.rx.try_recv() {
                    Ok(f) => f,
                    Err(_) => break,
                },
            };
            if next.0 <= t + 0.5 / self.fps {
                best = Some(next);
            } else {
                self.pending = Some(next);
                break;
            }
        }
        best
    }
}

impl Drop for VideoDecoder {
    fn drop(&mut self) {
        kill(&mut self.child);
    }
}

/// An ffmpeg decoding the sound from `start` on, piped (with the volume applied) into the
/// sound server's player.
struct SoundOut {
    decoder: Child,
    sink: Child,
}

impl SoundOut {
    fn start(file: &Path, start: f64, rate: f64, volume: Arc<AtomicU32>) -> Option<SoundOut> {
        let mut sink = sound_sink()?.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().ok()?;
        let mut cmd = Command::new(ffmpeg()?);
        cmd.args(["-nostdin", "-loglevel", "error", "-ss", &format!("{:.3}", start.max(0.0)), "-i"]).arg(file).args(["-vn", "-sn"]);
        if let Some(af) = atempo(rate) {
            cmd.args(["-af", &af]);
        }
        cmd.args(["-f", "s16le", "-ac", "2", "-ar", &RATE.to_string(), "-"]);
        let mut decoder = match quiet(&mut cmd).stdout(Stdio::piped()).spawn() {
            Ok(c) => c,
            Err(_) => {
                kill(&mut sink);
                return None;
            }
        };
        let (Some(mut pcm), Some(mut play)) = (decoder.stdout.take(), sink.stdin.take()) else {
            kill(&mut decoder);
            kill(&mut sink);
            return None;
        };
        let spawned = std::thread::Builder::new().name("media-sound".into()).spawn(move || {
            let mut buf = vec![0u8; 4096];
            loop {
                let n = match pcm.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n & !1,
                };
                apply_volume(&mut buf[..n], f32::from_bits(volume.load(Ordering::Relaxed)));
                if play.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
        });
        if spawned.is_err() {
            kill(&mut decoder);
            kill(&mut sink);
            return None;
        }
        Some(SoundOut { decoder, sink })
    }
}

impl Drop for SoundOut {
    fn drop(&mut self) {
        kill(&mut self.decoder);
        kill(&mut self.sink);
    }
}

/// Scale 16-bit little-endian samples by `v` (0–1).
pub fn apply_volume(pcm: &mut [u8], v: f32) {
    if v >= 0.999 {
        return;
    }
    for s in pcm.chunks_exact_mut(2) {
        let x = i16::from_le_bytes([s[0], s[1]]) as f32 * v.max(0.0);
        s.copy_from_slice(&(x as i16).to_le_bytes());
    }
}

// ---------------------------------------------------------------- per-element state

enum Loaded {
    Ok(TempFile, Info),
    Failed(u8, String),
}

/// What the element's box looked like when last drawn (it's redrawn when this changes).
#[derive(Clone, Copy, PartialEq, Default)]
struct Look {
    flags: u32,
    time: i64,
    duration: i64,
    frame: u64,
    poster: bool,
}

const F_CONTROLS: u32 = 1;
const F_PAUSED: u32 = 2;
const F_MUTED: u32 = 4;
const F_SHOW_POSTER: u32 = 8;
const F_AUDIO: u32 = 16;
const F_ERROR: u32 = 32;

#[derive(Default)]
struct Media {
    load: Option<Receiver<Loaded>>,
    poster_rx: Option<Receiver<Option<image::RgbaImage>>>,
    file: Option<TempFile>,
    info: Option<Info>,
    error: Option<(u8, String)>,
    poster: Option<Arc<image::RgbaImage>>,
    frame: Option<Arc<image::RgbaImage>>,
    frame_time: f64,
    frame_gen: u64,
    video: Option<VideoDecoder>,
    sound: Option<SoundOut>,
    volume: Arc<AtomicU32>,
    /// Waiting for the first frame at or after this time (after a seek or the first load).
    waiting_for: Option<f64>,
    look: Look,
    /// The look's numbers: current time and duration, and the screen's pixels per CSS px.
    shown: (f64, f64, f32),
    rendered: bool,
    version: u64,
    published: u64,
}

impl Media {
    fn new() -> Media {
        Media { volume: Arc::new(AtomicU32::new(1.0f32.to_bits())), ..Default::default() }
    }

    /// Pick up a finished download / probe and poster.
    fn poll(&mut self) {
        if let Some(rx) = &self.load {
            if let Ok(done) = rx.try_recv() {
                self.load = None;
                match done {
                    Loaded::Ok(file, info) => {
                        if info.video {
                            self.video = VideoDecoder::start(&file.0, &info, 0.0);
                            self.waiting_for = Some(0.0);
                        }
                        self.file = Some(file);
                        self.info = Some(info);
                    }
                    Loaded::Failed(code, msg) => self.error = Some((code, msg)),
                }
            }
        }
        if let Some(rx) = &self.poster_rx {
            if let Ok(p) = rx.try_recv() {
                self.poster_rx = None;
                self.poster = p.map(Arc::new);
                self.version += 1;
            }
        }
        // The first frame after a load or seek is shown as soon as it's decoded.
        if let (Some(t), Some(dec)) = (self.waiting_for, self.video.as_mut()) {
            if let Some((ft, img)) = dec.take_due(t) {
                self.set_frame(ft, img);
                self.waiting_for = None;
            } else if dec.pending.is_some() || matches!(dec.rx.try_recv(), Err(mpsc::TryRecvError::Disconnected)) {
                // Seeked past the last frame (or the decoder gave up): keep what we have.
                if let Some((ft, img)) = dec.pending.take() {
                    self.set_frame(ft, img);
                }
                self.waiting_for = None;
            }
        }
    }

    fn set_frame(&mut self, t: f64, img: image::RgbaImage) {
        self.frame = Some(Arc::new(img));
        self.frame_time = t;
        self.frame_gen += 1;
        self.version += 1;
    }
}

#[derive(Default)]
struct Store {
    media: HashMap<NodeId, Media>,
}

fn store(ctx: &Context) -> std::cell::RefMut<'_, Store> {
    ctx.get_data::<RefCell<Store>>().expect("media store").borrow_mut()
}

fn with<R>(ctx: &Context, id: NodeId, f: impl FnOnce(&mut Media) -> R) -> R {
    let mut s = store(ctx);
    f(s.media.entry(id).or_insert_with(Media::new))
}

/// Set up `<video>`/`<audio>` support in a page's script engine.
pub fn install(ctx: &mut Context) {
    ctx.insert_data(RefCell::new(Store::default()));
    let natives: &[(&str, usize, fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>)] = &[
        ("__mdAvailable", 0, n_available),
        ("__mdFrozen", 0, n_frozen),
        ("__mdOpen", 3, n_open),
        ("__mdPoster", 3, n_poster),
        ("__mdStatus", 1, n_status),
        ("__mdStart", 3, n_start),
        ("__mdStop", 1, n_stop),
        ("__mdSeek", 2, n_seek),
        ("__mdAdvance", 2, n_advance),
        ("__mdVolume", 2, n_volume),
        ("__mdRender", 4, n_render),
        ("__mdClose", 1, n_close),
    ];
    for (name, len, f) in natives {
        ctx.register_global_callable(JsString::from(*name), *len, NativeFunction::from_fn_ptr(*f)).expect("register media native");
    }
    if let Err(e) = ctx.eval(Source::from_bytes(include_str!("media.js").as_bytes())) {
        log!("[ironswift] media setup failed: {e}");
    }
}

/// Hand media pictures that changed since the last call to the page's images.
pub fn publish(ctx: &Context, images: &mut Images) {
    let Some(store) = ctx.get_data::<RefCell<Store>>() else { return };
    let mut store = store.borrow_mut();
    for (id, m) in store.media.iter_mut() {
        if m.version == m.published {
            continue;
        }
        m.published = m.version;
        let key = format!("media:{id}");
        let (time, duration, scale) = m.shown;
        let snap = Snap { frame: m.frame.clone(), poster: m.poster.clone(), flags: m.look.flags, time, duration, scale };
        // The natural size: the video's, else the poster's.
        let natural = match (&m.info, &m.poster) {
            (Some(i), _) if i.video => (i.width, i.height),
            (_, Some(p)) => p.dimensions(),
            _ => (0, 0),
        };
        if snap.frame.is_none() && snap.poster.is_none() && snap.flags & F_CONTROLS == 0 {
            images.insert_drawn(&key, natural.0, natural.1, Arc::new(|_, _| None));
            continue;
        }
        images.insert_drawn(&key, natural.0, natural.1, Arc::new(move |w, h| draw(&snap, w, h)));
    }
}

// ---------------------------------------------------------------- natives

fn num(a: &[JsValue], i: usize, ctx: &mut Context) -> f64 {
    a.get(i).map_or(Ok(f64::NAN), |v| v.to_number(ctx)).unwrap_or(f64::NAN)
}

fn string(a: &[JsValue], i: usize, ctx: &mut Context) -> String {
    a.get(i).and_then(|v| v.to_string(ctx).ok()).map(|s| s.to_std_string_escaped()).unwrap_or_default()
}

fn id_of(a: &[JsValue], ctx: &mut Context) -> NodeId {
    num(a, 0, ctx) as NodeId
}

fn n_available(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(available().into())
}

fn n_frozen(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(crate::images::animations_frozen().into())
}

/// Download `url` (for page `page`) and work out what's in it, in the background.
fn n_open(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = id_of(a, ctx);
    let (url, page) = (string(a, 1, ctx), string(a, 2, ctx));
    let (tx, rx) = mpsc::channel();
    with(ctx, id, |m| {
        let poster = (m.poster.take(), m.poster_rx.take());
        *m = Media { poster: poster.0, poster_rx: poster.1, volume: m.volume.clone(), version: m.version + 1, ..Media::new() };
        m.load = Some(rx);
    });
    std::thread::Builder::new()
        .name("media-load".into())
        .spawn(move || {
            let done = match crate::net::fetch_sub_bytes(&url, &page, crate::security::Dest::Media) {
                Err(e) => Loaded::Failed(2, e),
                Ok((_, bytes)) if bytes.len() > MAX_BYTES => Loaded::Failed(2, "too big".into()),
                Ok((_, bytes)) => {
                    let file = TempFile(temp_path());
                    match std::fs::write(&file.0, &bytes) {
                        Err(e) => Loaded::Failed(2, e.to_string()),
                        Ok(()) => match probe(&file.0) {
                            Ok(info) => Loaded::Ok(file, info),
                            Err(e) => Loaded::Failed(4, e),
                        },
                    }
                }
            };
            let _ = tx.send(done);
        })
        .ok();
    Ok(JsValue::undefined())
}

/// Load a `<video poster>` picture in the background ("" forgets it).
fn n_poster(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = id_of(a, ctx);
    let (url, page) = (string(a, 1, ctx), string(a, 2, ctx));
    if url.is_empty() {
        with(ctx, id, |m| {
            m.poster = None;
            m.poster_rx = None;
            m.version += 1;
        });
        return Ok(JsValue::undefined());
    }
    let (tx, rx) = mpsc::channel();
    with(ctx, id, |m| m.poster_rx = Some(rx));
    std::thread::Builder::new()
        .name("media-poster".into())
        .spawn(move || {
            let img = crate::net::fetch_sub_bytes(&url, &page, crate::security::Dest::Image).ok().and_then(|(_, b)| image::load_from_memory(&b).ok()).map(|i| i.to_rgba8());
            let _ = tx.send(img);
        })
        .ok();
    Ok(JsValue::undefined())
}

/// `[state, duration, videoWidth, videoHeight, errorCode, errorMessage, busy]`:
/// state 0 = nothing yet, 1 = metadata, 2 = current frame (or sound only), 3 = error;
/// busy = a download, poster or first frame is still on its way.
fn n_status(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = id_of(a, ctx);
    let (state, dur, w, h, code, msg, busy) = with(ctx, id, |m| {
        m.poll();
        let busy = m.load.is_some() || m.poster_rx.is_some() || m.waiting_for.is_some();
        match (&m.error, &m.info) {
            (Some((c, msg)), _) => (3, f64::NAN, 0, 0, *c, msg.clone(), busy),
            (None, Some(i)) => {
                let state = if !i.video || m.frame.is_some() || m.waiting_for.is_none() { 2 } else { 1 };
                (state, i.duration, i.width, i.height, 0, String::new(), busy)
            }
            (None, None) => (0, f64::NAN, 0, 0, 0, String::new(), busy),
        }
    });
    let vals = [JsValue::from(state), JsValue::from(dur), JsValue::from(w), JsValue::from(h), JsValue::from(code), JsValue::from(JsString::from(msg)), JsValue::from(busy)];
    Ok(JsArray::from_iter(vals, ctx).into())
}

/// Playback starts (or restarts, at a new speed) at media time `t`: sound from there; the
/// video decoder keeps going if it's already there.
fn n_start(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = id_of(a, ctx);
    let (t, rate) = (num(a, 1, ctx), num(a, 2, ctx));
    let frozen = crate::images::animations_frozen();
    with(ctx, id, |m| {
        m.sound = None;
        let (Some(file), Some(info)) = (&m.file, &m.info) else { return };
        if info.video {
            // Restart the decoder only if the frames it has are for somewhere else.
            let near = m.video.is_some() && (t - m.frame_time).abs() < 1.0;
            if !near {
                m.video = VideoDecoder::start(&file.0, info, t);
            }
        }
        if info.audio && !frozen && rate > 0.0 {
            m.sound = SoundOut::start(&file.0, t, rate, m.volume.clone());
        }
    });
    Ok(JsValue::undefined())
}

fn n_stop(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = id_of(a, ctx);
    with(ctx, id, |m| m.sound = None);
    Ok(JsValue::undefined())
}

/// Jump to media time `t`: the frame there is decoded (see `__mdAdvance`).
fn n_seek(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = id_of(a, ctx);
    let t = num(a, 1, ctx);
    with(ctx, id, |m| {
        m.sound = None;
        if let (Some(file), Some(info)) = (&m.file, &m.info) {
            if info.video {
                m.video = VideoDecoder::start(&file.0, info, t);
                m.waiting_for = Some(t);
            }
        }
    });
    Ok(JsValue::undefined())
}

/// Show the frame for media time `t`. True once no frame is still awaited (a seek has its
/// picture).
fn n_advance(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = id_of(a, ctx);
    let t = num(a, 1, ctx);
    let ready = with(ctx, id, |m| {
        m.poll();
        if m.waiting_for.is_none() {
            if let Some(dec) = m.video.as_mut() {
                if let Some((ft, img)) = dec.take_due(t) {
                    m.set_frame(ft, img);
                }
            }
        }
        m.waiting_for.is_none()
    });
    Ok(ready.into())
}

fn n_volume(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = id_of(a, ctx);
    let v = num(a, 1, ctx);
    let v = if v.is_finite() { v.clamp(0.0, 1.0) as f32 } else { 1.0 };
    with(ctx, id, |m| m.volume.store(v.to_bits(), Ordering::Relaxed));
    Ok(JsValue::undefined())
}

/// What the element's box shows: `(id, flags, currentTime, duration)`. Marks the page for
/// repainting when that changed.
fn n_render(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = id_of(a, ctx);
    let flags = num(a, 1, ctx) as u32;
    let (time, duration) = (num(a, 2, ctx), num(a, 3, ctx));
    let scale = crate::zoom::pixel_ratio().clamp(0.25, 4.0);
    let changed = with(ctx, id, |m| {
        m.poll();
        let look = Look {
            flags,
            time: if time.is_finite() { time.floor() as i64 } else { -1 },
            duration: if duration.is_finite() { duration.floor() as i64 } else { -1 },
            frame: m.frame_gen,
            poster: m.poster.is_some(),
        };
        // The progress bar moves smoothly, so a playing element with controls redraws each frame.
        let progress = flags & F_CONTROLS != 0 && flags & F_PAUSED == 0;
        if look == m.look && m.rendered && !progress && m.shown.2 == scale {
            return false;
        }
        m.look = look;
        m.rendered = true;
        m.shown = (time, duration, scale);
        m.version += 1;
        true
    });
    if changed {
        crate::js::mark_dirty();
    }
    Ok(changed.into())
}

/// The element is gone or loads something else: stop its decoders and free its pictures.
fn n_close(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = id_of(a, ctx);
    store(ctx).media.remove(&id);
    Ok(JsValue::undefined())
}

// ---------------------------------------------------------------- drawing the box

fn to_pixmap(img: &image::RgbaImage) -> Option<Pixmap> {
    let mut data = img.as_raw().clone();
    for p in data.chunks_exact_mut(4) {
        let a = p[3] as u16;
        if a < 255 {
            for c in &mut p[..3] {
                *c = ((*c as u16 * a + 127) / 255) as u8;
            }
        }
    }
    Pixmap::from_vec(data, tiny_skia::IntSize::from_wh(img.width(), img.height())?)
}

fn rgba(r: u8, g: u8, b: u8, a: u8) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(Color::from_rgba8(r, g, b, a));
    p.anti_alias = true;
    p
}

/// `m:ss` (or `h:mm:ss`).
pub fn clock_text(t: f64) -> String {
    if !t.is_finite() || t < 0.0 {
        return "0:00".into();
    }
    let s = t.floor() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Where the controls bar starts (CSS px from the box's top): the bottom of a video, the
/// middle of an audio player. `media.js` hit-tests with the same numbers.
pub fn bar_top(h: f32, audio: bool) -> f32 {
    if audio { ((h - BAR_H) / 2.0).max(0.0) } else { (h - BAR_H).max(0.0) }
}

/// Where the controls' parts are along the bar (CSS px), for a box `w` wide:
/// (play button end, progress track start, progress track end, mute button start).
/// `media.js` hit-tests with the same numbers.
pub fn bar_layout(w: f32) -> (f32, f32, f32, f32) {
    let time_w = if w >= 200.0 { 96.0 } else { 4.0 };
    (32.0, 32.0 + time_w, (w - 40.0).max(32.0 + time_w), (w - 32.0).max(0.0))
}

/// What's needed to draw an element's box, kept by the page's images.
struct Snap {
    frame: Option<Arc<image::RgbaImage>>,
    poster: Option<Arc<image::RgbaImage>>,
    flags: u32,
    time: f64,
    duration: f64,
    /// Device pixels per CSS px.
    scale: f32,
}

fn draw(m: &Snap, pw: u32, ph: u32) -> Option<image::RgbaImage> {
    let (flags, scale, time, duration) = (m.flags, m.scale, m.time, m.duration);
    let mut pm = Pixmap::new(pw.max(1), ph.max(1))?;
    let (bw, bh) = (pw as f32, ph as f32);
    // The picture: the poster until playback or a seek, else the current frame (the poster
    // again if there's no frame); fitted inside the box keeping its shape.
    let pic = if flags & F_AUDIO != 0 {
        None
    } else if flags & F_SHOW_POSTER != 0 && m.poster.is_some() {
        m.poster.as_ref()
    } else {
        m.frame.as_ref().or(m.poster.as_ref())
    };
    if let Some(img) = pic.and_then(|p| to_pixmap(p)) {
        let (iw, ih) = (img.width() as f32, img.height() as f32);
        let s = (bw / iw).min(bh / ih);
        let (dw, dh) = (iw * s, ih * s);
        let t = Transform::from_row(s, 0.0, 0.0, s, (bw - dw) / 2.0, (bh - dh) / 2.0);
        pm.draw_pixmap(0, 0, img.as_ref(), &PixmapPaint { quality: FilterQuality::Bilinear, ..Default::default() }, t, None);
    }
    if flags & F_CONTROLS != 0 {
        draw_controls(&mut pm, scale, flags, time, duration);
    }
    let mut out = Vec::with_capacity((pw * ph * 4) as usize);
    for p in pm.pixels() {
        let c = p.demultiply();
        out.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    image::RgbaImage::from_raw(pm.width(), pm.height(), out)
}

fn draw_controls(pm: &mut Pixmap, s: f32, flags: u32, time: f64, duration: f64) {
    let (bw, bh) = (pm.width() as f32 / s, pm.height() as f32 / s);
    let audio = flags & F_AUDIO != 0;
    let top = bar_top(bh, audio);
    let tf = Transform::from_scale(s, s);
    // Video: a dark translucent bar along the bottom; audio: a light rounded panel filling
    // the box, the bar across its middle.
    let (bg, fg, track) = if audio { (rgba(241, 243, 244, 255), (32, 33, 36), (190, 193, 198)) } else { (rgba(0, 0, 0, 150), (255, 255, 255), (150, 150, 150)) };
    let panel = if audio { rounded(0.0, 0.0, bw, bh, (bh / 2.0).min(27.0)) } else { rounded(0.0, top, bw, BAR_H.min(bh), 0.0) };
    if let Some(r) = panel {
        pm.fill_path(&r, &bg, FillRule::Winding, tf, None);
    }
    let fgp = rgba(fg.0, fg.1, fg.2, 255);
    let cy = top + BAR_H / 2.0;
    let (play_end, track_x0, track_x1, mute_x) = bar_layout(bw);
    // Play (a triangle) or pause (two bars).
    if flags & F_PAUSED != 0 {
        let mut pb = PathBuilder::new();
        pb.move_to(11.0, cy - 7.0);
        pb.line_to(23.0, cy);
        pb.line_to(11.0, cy + 7.0);
        pb.close();
        if let Some(p) = pb.finish() {
            pm.fill_path(&p, &fgp, FillRule::Winding, tf, None);
        }
    } else {
        for x in [11.0, 18.0] {
            if let Some(r) = Rect::from_xywh(x, cy - 7.0, 4.0, 14.0) {
                pm.fill_rect(r, &fgp, tf, None);
            }
        }
    }
    let _ = play_end;
    // Elapsed / total time.
    if track_x0 - 32.0 > 20.0 {
        let text = format!("{} / {}", clock_text(time), clock_text(duration));
        draw_text(pm, s, 36.0, cy + 4.5, &text, 12.0, fg);
    }
    // The progress track and how far along it playback is.
    let len = track_x1 - track_x0;
    if len > 4.0 {
        if let Some(r) = rounded(track_x0, cy - 2.0, len, 4.0, 2.0) {
            pm.fill_path(&r, &rgba(track.0, track.1, track.2, 255), FillRule::Winding, tf, None);
        }
        let frac = if duration.is_finite() && duration > 0.0 && time.is_finite() { (time / duration).clamp(0.0, 1.0) as f32 } else { 0.0 };
        if frac > 0.0 {
            if let Some(r) = rounded(track_x0, cy - 2.0, len * frac, 4.0, 2.0) {
                pm.fill_path(&r, &fgp, FillRule::Winding, tf, None);
            }
        }
        let mut pb = PathBuilder::new();
        pb.push_circle(track_x0 + len * frac, cy, 5.0);
        if let Some(p) = pb.finish() {
            pm.fill_path(&p, &fgp, FillRule::Winding, tf, None);
        }
    }
    // The speaker, crossed out when muted.
    if mute_x > track_x1 {
        let x = mute_x + 8.0;
        let mut pb = PathBuilder::new();
        pb.move_to(x, cy - 3.0);
        pb.line_to(x + 4.0, cy - 3.0);
        pb.line_to(x + 9.0, cy - 7.0);
        pb.line_to(x + 9.0, cy + 7.0);
        pb.line_to(x + 4.0, cy + 3.0);
        pb.line_to(x, cy + 3.0);
        pb.close();
        if let Some(p) = pb.finish() {
            pm.fill_path(&p, &fgp, FillRule::Winding, tf, None);
        }
        let mut stroke = tiny_skia::Stroke { width: 1.6, ..Default::default() };
        stroke.line_cap = tiny_skia::LineCap::Round;
        let mut pb = PathBuilder::new();
        if flags & F_MUTED != 0 {
            pb.move_to(x + 12.0, cy - 4.0);
            pb.line_to(x + 18.0, cy + 4.0);
            pb.move_to(x + 18.0, cy - 4.0);
            pb.line_to(x + 12.0, cy + 4.0);
        } else {
            pb.move_to(x + 12.0, cy - 4.0);
            pb.quad_to(x + 15.0, cy, x + 12.0, cy + 4.0);
        }
        if let Some(p) = pb.finish() {
            pm.stroke_path(&p, &fgp, &stroke, tf, None);
        }
    }
    if flags & F_ERROR != 0 && !audio && top > 24.0 {
        draw_text(pm, s, 12.0, top / 2.0, "This video can't be played", 13.0, (255, 255, 255));
    }
}

fn rounded(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let r = r.min(w / 2.0).min(h / 2.0);
    if r <= 0.0 {
        return Some(PathBuilder::from_rect(Rect::from_xywh(x, y, w, h)?));
    }
    let k = 0.552_284_8 * r;
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish()
}

/// Text in the controls, with its baseline at (x, baseline) in CSS px.
fn draw_text(pm: &mut Pixmap, s: f32, x: f32, baseline: f32, text: &str, size: f32, color: (u8, u8, u8)) {
    let Some(fonts) = crate::load_fonts() else { return };
    let dsize = size * s;
    let shaped = fonts.shape(crate::font::REGULAR, text, dsize);
    let (pw, ph) = (pm.width() as i32, pm.height() as i32);
    let (ox, oy) = (x * s, baseline * s);
    let data = pm.data_mut();
    for gl in &shaped.glyphs {
        fonts.with_glyph_id(gl.face, gl.id, dsize, 0, |mt, bmp| {
            let gx = (ox + gl.x).round() as i32 + mt.xmin;
            let gy = (oy - gl.y).round() as i32 - mt.height as i32 - mt.ymin;
            for row in 0..mt.height {
                for col in 0..mt.width {
                    let (px, py) = (gx + col as i32, gy + row as i32);
                    if px < 0 || py < 0 || px >= pw || py >= ph {
                        continue;
                    }
                    let cov = if mt.color { bmp.get((row * mt.width + col) * 4 + 3).copied().unwrap_or(0) } else { bmp[row * mt.width + col] } as u32;
                    if cov == 0 {
                        continue;
                    }
                    // Premultiplied source-over.
                    let i = (py * pw + px) as usize * 4;
                    let inv = 255 - cov;
                    for (k, c) in [color.0, color.1, color.2].into_iter().enumerate() {
                        data[i + k] = ((c as u32 * cov + data[i + k] as u32 * inv) / 255) as u8;
                    }
                    data[i + 3] = (cov + data[i + 3] as u32 * inv / 255).min(255) as u8;
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_ffprobe_output() {
        let out = "stream|codec_type=video|width=320|height=240|r_frame_rate=25/1|avg_frame_rate=25/1|duration=3.000000\nstream|codec_type=audio|r_frame_rate=0/0|avg_frame_rate=0/0|duration=3.003000\nformat|duration=3.003000\n";
        let i = parse_probe(out).unwrap();
        assert!(i.video && i.audio);
        assert_eq!((i.width, i.height), (320, 240));
        assert_eq!(i.fps, 25.0);
        assert!((i.duration - 3.003).abs() < 1e-9);
        // Sound only, duration only on the stream; NTSC frame rates.
        let a = parse_probe("stream|codec_type=audio|duration=12.5\nformat|duration=N/A\n").unwrap();
        assert!(!a.video && a.audio && (a.duration - 12.5).abs() < 1e-9);
        let n = parse_probe("stream|codec_type=video|width=640|height=360|avg_frame_rate=30000/1001\n").unwrap();
        assert!((n.fps - 29.97).abs() < 0.01);
        assert!(parse_probe("format|duration=1.0\n").is_none());
    }

    #[test]
    fn sizes_speeds_and_clock() {
        assert_eq!(frame_size(&Info { width: 3840, height: 2160, ..Default::default() }), (1920, 1080));
        assert_eq!(frame_size(&Info { width: 320, height: 240, ..Default::default() }), (320, 240));
        assert_eq!(atempo(1.0), None);
        assert_eq!(atempo(1.5).as_deref(), Some("atempo=1.5000"));
        assert_eq!(atempo(4.0).as_deref(), Some("atempo=2,atempo=2.0000"));
        assert_eq!(atempo(0.25).as_deref(), Some("atempo=0.5,atempo=0.5000"));
        assert_eq!(clock_text(0.0), "0:00");
        assert_eq!(clock_text(65.9), "1:05");
        assert_eq!(clock_text(3725.0), "1:02:05");
        let mut pcm = 1000i16.to_le_bytes().to_vec();
        apply_volume(&mut pcm, 0.5);
        assert_eq!(i16::from_le_bytes([pcm[0], pcm[1]]), 500);
    }

    /// With ffmpeg installed: make a short clip, probe it and decode frames from a seek point.
    #[test]
    fn decodes_frames_with_ffmpeg() {
        if !available() {
            eprintln!("ffmpeg not installed; skipping");
            return;
        }
        let file = TempFile(temp_path().with_extension("webm"));
        let made = Command::new(ffmpeg().unwrap())
            .args(["-nostdin", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "color=c=red:size=64x48:rate=10:duration=2", "-c:v", "libvpx"])
            .arg(&file.0)
            .status();
        if !made.is_ok_and(|s| s.success()) {
            eprintln!("this ffmpeg can't encode VP8; skipping");
            return;
        }
        let info = probe(&file.0).expect("probe");
        assert!(info.video && !info.audio);
        assert_eq!((info.width, info.height), (64, 48));
        assert!((info.duration - 2.0).abs() < 0.2, "{}", info.duration);
        let mut dec = VideoDecoder::start(&file.0, &info, 1.0).expect("decoder");
        let t0 = std::time::Instant::now();
        let frame = loop {
            if let Some(f) = dec.take_due(1.0) {
                break f;
            }
            assert!(t0.elapsed().as_secs() < 20, "no frame");
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        assert!((frame.0 - 1.0).abs() < 1e-9);
        let p = frame.1.get_pixel(32, 24);
        assert!(p[0] > 200 && p[1] < 60 && p[2] < 60, "{p:?}");
    }

    #[test]
    fn draws_controls_over_the_frame() {
        let frame = Some(Arc::new(image::RgbaImage::from_pixel(40, 30, image::Rgba([0, 0, 255, 255]))));
        let m = Snap { frame, poster: None, flags: F_CONTROLS | F_PAUSED, time: 0.0, duration: 10.0, scale: 1.0 };
        let img = draw(&m, 300, 150).unwrap();
        // 4:3 frame in a 2:1 box: letterboxed sideways, blue in the middle, clear at the edge.
        assert_eq!(img.get_pixel(150, 40).0, [0, 0, 255, 255]);
        assert_eq!(img.get_pixel(5, 40).0[3], 0);
        // The bar is along the bottom, and the play triangle is drawn in it.
        assert!(img.get_pixel(150, 140).0[3] > 100);
        let tri = img.get_pixel(15, 150 - 16);
        assert!(tri.0[0] > 200 && tri.0[1] > 200, "{tri:?}");
    }
}

#[cfg(test)]
mod page_tests {
    /// Play a muted clip at 4× to its end, from a page's script, and check the events and
    /// state a page sees.
    #[test]
    fn playback_from_script() {
        if !super::available() {
            eprintln!("ffmpeg not installed; skipping");
            return;
        }
        let profile = std::env::temp_dir().join(format!("ironswift-media-test-{}", std::process::id()));
        // SAFETY: tests only read this variable; every test sets the same kind of value.
        unsafe { std::env::set_var("IRONSWIFT_PROFILE", &profile) };
        crate::load_fonts().expect("fonts");
        let url = crate::net::normalize_input("tests/media-api.html");
        let mut page = crate::load(&url);
        let js = page.js.as_mut().expect("scripts");
        let t = std::time::Instant::now();
        let text = |id: &str| {
            crate::js::with_dom(|d| {
                let n = d.descendants(crate::dom::Dom::ROOT).into_iter().find(|&n| d.attr(n, "id") == Some(id));
                n.map(|n| d.text_content(n)).unwrap_or_default()
            })
        };
        while text("out").is_empty() || text("errs").is_empty() {
            assert!(t.elapsed().as_secs() < 20, "didn't finish: out={:?} errs={:?}", text("out"), text("errs"));
            js.settle(std::time::Duration::from_millis(200));
        }
        assert_eq!(text("checks"), "IndexSizeError,TypeError,4,2,2,metadata,,true,true,true");
        let out = text("out");
        assert!(out.starts_with("ratechange,play,playing,resolved,timeupdate,"), "{out}");
        assert!(out.ends_with("timeupdate,pause,ended t=2.0 paused=true ended=true played=1:2.0"), "{out}");
        assert_eq!(text("errs"), "error 2 1 true");
        assert_eq!(text("perr"), "NotSupportedError");
    }
}
