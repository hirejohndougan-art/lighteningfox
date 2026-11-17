//! Media Source Extensions: `MediaSource` and `SourceBuffer` (with `mse.js`).
//!
//! A page's player (YouTube's, hls.js, dash.js, Shaka…) downloads media in pieces and hands
//! them to `SourceBuffer.appendBuffer`. Each SourceBuffer here splits what it's given into an
//! *initialization segment* (MP4 `ftyp`+`moov`, WebM EBML header + `Segment` up to the
//! first `Cluster`) and *media segments* (MP4 `moof`+`mdat`, WebM `Cluster`), reading each
//! media segment's start and end time from its headers (`tfdt`/`trun`, cluster and block
//! timecodes). The segments, kept in time order, are its buffered ranges.
//!
//! Playback reuses media.rs's ffmpeg decoders: instead of a file, a decoder reads a pipe
//! that a `pump` thread fills with the init segment and then the media segments from the
//! playback position on, in time order, waiting for more as the page appends them (the
//! "feed"). A seek starts a new decoder at the segment holding the new time, so seeking
//! doesn't decode from the start.

use boa_engine::object::builtins::{JsArray, JsUint8Array};
use boa_engine::{Context, JsResult, JsString, JsValue, NativeFunction, Source};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// Segments closer than this count as touching (in buffered ranges and in the feed).
const GAP: f64 = 0.15;
/// The most a SourceBuffer holds before appends are refused (QuotaExceededError).
const MAX_BYTES: usize = 600 << 20;

// ---------------------------------------------------------------- the feed

/// A media segment and the media time it covers.
#[derive(Clone)]
pub struct Seg {
    pub start: f64,
    pub end: f64,
    pub bytes: Arc<Vec<u8>>,
}

#[derive(Default)]
struct FeedState {
    init: Option<Arc<Vec<u8>>>,
    /// Bumped when a new (different) init segment arrives.
    init_gen: u64,
    /// Sorted by start time, not overlapping.
    segs: Vec<Seg>,
    ended: bool,
    bytes: usize,
}

/// One SourceBuffer's segments, shared with the threads feeding its decoders.
#[derive(Default)]
pub struct Feed {
    state: Mutex<FeedState>,
    more: Condvar,
}

impl Feed {
    fn lock(&self) -> std::sync::MutexGuard<'_, FeedState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set_init(&self, init: Vec<u8>) {
        let mut st = self.lock();
        if st.init.as_deref() != Some(&init) {
            st.init = Some(Arc::new(init));
            st.init_gen += 1;
        }
        self.more.notify_all();
    }

    /// Add a media segment, replacing what it overlaps (coded frame removal, by segment).
    fn add(&self, seg: Seg) {
        let mut st = self.lock();
        let (s, e) = (seg.start, seg.end.max(seg.start + 1e-3));
        let mut freed = 0;
        st.segs.retain(|o| {
            let overlap = o.start < e - 1e-3 && o.end.max(o.start + 1e-3) > s + 1e-3;
            if overlap {
                freed += o.bytes.len();
            }
            !overlap
        });
        st.bytes = st.bytes - freed + seg.bytes.len();
        let at = st.segs.partition_point(|o| o.start < seg.start);
        st.segs.insert(at, seg);
        st.ended = false;
        self.more.notify_all();
    }

    /// Drop the segments that start in `[start, end)`.
    fn remove(&self, start: f64, end: f64) {
        let mut st = self.lock();
        let mut freed = 0;
        st.segs.retain(|o| {
            let gone = o.start >= start - 1e-6 && o.start < end;
            if gone {
                freed += o.bytes.len();
            }
            !gone
        });
        st.bytes -= freed;
    }

    fn set_ended(&self, ended: bool) {
        self.lock().ended = ended;
        self.more.notify_all();
    }

    fn bytes(&self) -> usize {
        self.lock().bytes
    }

    /// The buffered ranges.
    pub fn buffered(&self) -> Vec<(f64, f64)> {
        let st = self.lock();
        let mut out: Vec<(f64, f64)> = Vec::new();
        for s in &st.segs {
            match out.last_mut() {
                Some(last) if s.start <= last.1 + GAP => last.1 = last.1.max(s.end),
                _ => out.push((s.start, s.end)),
            }
        }
        out
    }

    /// Where a decoder starting at media time `t` begins: the start of the segment holding
    /// `t` (or the next one). ffmpeg's `-ss` on a pipe counts from there.
    pub fn start_for(&self, t: f64) -> Option<f64> {
        let st = self.lock();
        let i = st.segs.partition_point(|s| s.start <= t + 1e-3);
        if i > 0 && st.segs[i - 1].end > t - GAP {
            return Some(st.segs[i - 1].start);
        }
        st.segs.get(i).map(|s| s.start)
    }

    /// Write the init segment and then the media segments from `from` on into a decoder's
    /// input, in time order, waiting for more until the stream ends, `stop` is set, the
    /// decoder goes away or a new init segment arrives.
    pub fn pump(self: Arc<Self>, from: Option<f64>, mut out: impl Write, stop: Arc<AtomicBool>) {
        let mut st = self.lock();
        let started = loop {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            if st.init.is_some() {
                break st.init_gen;
            }
            if st.ended {
                return;
            }
            st = self.more.wait_timeout(st, Duration::from_millis(250)).unwrap_or_else(|e| e.into_inner()).0;
        };
        let init = st.init.clone().unwrap_or_default();
        drop(st);
        if out.write_all(&init).is_err() {
            return;
        }
        // (start, end) of the last segment written.
        let mut last: Option<(f64, f64)> = None;
        let mut first = from;
        loop {
            let mut st = self.lock();
            let next = loop {
                if stop.load(Ordering::Relaxed) || st.init_gen != started {
                    return;
                }
                let found = match (last, first) {
                    (None, Some(f)) => st.segs.iter().find(|s| s.start >= f - 1e-3),
                    (None, None) => st.segs.first(),
                    (Some((ls, le)), _) => st.segs.iter().find(|s| s.start > ls + 1e-6 && s.start <= le + GAP),
                };
                if let Some(s) = found {
                    break s.clone();
                }
                if st.ended {
                    return;
                }
                st = self.more.wait_timeout(st, Duration::from_millis(250)).unwrap_or_else(|e| e.into_inner()).0;
            };
            drop(st);
            if out.write_all(&next.bytes).and_then(|_| out.flush()).is_err() {
                return;
            }
            first = None;
            last = Some((next.start, next.end));
        }
    }
}

// ---------------------------------------------------------------- what an init segment says

/// What an initialization segment describes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InitInfo {
    /// Seconds; NaN if it doesn't say.
    pub duration: f64,
    pub video: Option<(u32, u32, f64)>,
    pub audio: bool,
}

/// A media segment found in appended bytes.
#[derive(Debug, PartialEq)]
pub enum Piece {
    Init(Vec<u8>, InitInfo),
    Media(f64, f64, Vec<u8>),
}

fn be(b: &[u8]) -> u64 {
    b.iter().fold(0u64, |a, &x| (a << 8) | x as u64)
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    b.get(i..i + 4).map(|s| be(s) as u32)
}

fn u64_at(b: &[u8], i: usize) -> Option<u64> {
    b.get(i..i + 8).map(be)
}

// ---------------------------------------------------------------- MP4 (ISO BMFF)

#[derive(Clone, Copy, Default, Debug)]
struct Track {
    timescale: u32,
    video: bool,
    audio: bool,
    default_dur: u32,
    w: u32,
    h: u32,
}

#[derive(Default)]
struct Mp4 {
    tracks: HashMap<u32, Track>,
    init: Vec<u8>,
    seg: Option<(Vec<u8>, f64, f64)>,
}

/// The boxes inside a box's payload: `(type, payload range)`.
fn boxes(b: &[u8]) -> Vec<([u8; 4], std::ops::Range<usize>)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 8 <= b.len() {
        let size = be(&b[i..i + 4]) as usize;
        let kind = [b[i + 4], b[i + 5], b[i + 6], b[i + 7]];
        let (head, size) = match size {
            1 => match u64_at(b, i + 8) {
                Some(s) => (16, s as usize),
                None => break,
            },
            0 => (8, b.len() - i),
            s => (8, s),
        };
        if size < head || i + size > b.len() {
            break;
        }
        out.push((kind, i + head..i + size));
        i += size;
    }
    out
}

fn child<'a>(b: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    boxes(b).into_iter().find(|(k, _)| k == kind).map(|(_, r)| &b[r])
}

impl Mp4 {
    fn read_moov(&mut self, moov: &[u8]) -> InitInfo {
        let mut info = InitInfo { duration: f64::NAN, ..Default::default() };
        let mut movie_scale = 0;
        self.tracks.clear();
        for (k, r) in boxes(moov) {
            let b = &moov[r];
            match &k {
                b"mvhd" => {
                    let v1 = b.first() == Some(&1);
                    let (scale, dur) = if v1 { (u32_at(b, 20), u64_at(b, 24)) } else { (u32_at(b, 12), u32_at(b, 16).map(|d| d as u64)) };
                    movie_scale = scale.unwrap_or(0);
                    if let (Some(d), true) = (dur, movie_scale > 0) {
                        if d > 0 && d != u32::MAX as u64 && d != u64::MAX {
                            info.duration = d as f64 / movie_scale as f64;
                        }
                    }
                }
                b"trak" => {
                    let mut t = Track::default();
                    let mut id = 0;
                    if let Some(tkhd) = child(b, b"tkhd") {
                        let v1 = tkhd.first() == Some(&1);
                        id = u32_at(tkhd, if v1 { 20 } else { 12 }).unwrap_or(0);
                        let n = tkhd.len();
                        if n >= 8 {
                            t.w = u32_at(tkhd, n - 8).unwrap_or(0) >> 16;
                            t.h = u32_at(tkhd, n - 4).unwrap_or(0) >> 16;
                        }
                    }
                    if let Some(mdia) = child(b, b"mdia") {
                        if let Some(mdhd) = child(mdia, b"mdhd") {
                            let v1 = mdhd.first() == Some(&1);
                            t.timescale = u32_at(mdhd, if v1 { 20 } else { 12 }).unwrap_or(0);
                        }
                        if let Some(hdlr) = child(mdia, b"hdlr") {
                            match hdlr.get(8..12) {
                                Some(b"vide") => t.video = true,
                                Some(b"soun") => t.audio = true,
                                _ => {}
                            }
                        }
                    }
                    if t.timescale > 0 {
                        self.tracks.insert(id, t);
                    }
                }
                b"mvex" => {
                    for (k2, r2) in boxes(b) {
                        let c = &b[r2];
                        if &k2 == b"trex" {
                            if let (Some(id), Some(d)) = (u32_at(c, 4), u32_at(c, 12)) {
                                self.tracks.entry(id).or_default().default_dur = d;
                            }
                        } else if &k2 == b"mehd" && movie_scale > 0 {
                            let d = if c.first() == Some(&1) { u64_at(c, 4) } else { u32_at(c, 4).map(|d| d as u64) };
                            if let Some(d) = d.filter(|&d| d > 0) {
                                info.duration = d as f64 / movie_scale as f64;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        // trex may come before the trak it describes: drop tracks that never got a timescale.
        self.tracks.retain(|_, t| t.timescale > 0);
        for t in self.tracks.values() {
            if t.video && info.video.is_none() {
                let fps = if t.default_dur > 0 { t.timescale as f64 / t.default_dur as f64 } else { 30.0 };
                info.video = Some((t.w, t.h, fps));
            }
            info.audio |= t.audio;
        }
        info
    }

    /// The media time a `moof` covers.
    fn read_moof(&self, moof: &[u8]) -> Option<(f64, f64)> {
        let (mut start, mut end) = (f64::INFINITY, f64::NEG_INFINITY);
        for (k, r) in boxes(moof) {
            if &k != b"traf" {
                continue;
            }
            let traf = &moof[r];
            let tfhd = child(traf, b"tfhd")?;
            let flags = u32_at(tfhd, 0)? & 0xff_ffff;
            let id = u32_at(tfhd, 4)?;
            let Some(track) = self.tracks.get(&id) else { continue };
            let mut i = 8;
            if flags & 1 != 0 {
                i += 8;
            }
            if flags & 2 != 0 {
                i += 4;
            }
            let mut default_dur = track.default_dur;
            if flags & 8 != 0 {
                default_dur = u32_at(tfhd, i)?;
            }
            let base = match child(traf, b"tfdt") {
                Some(t) if t.first() == Some(&1) => u64_at(t, 4)?,
                Some(t) => u32_at(t, 4)? as u64,
                None => 0,
            };
            let mut dur: u64 = 0;
            for (k2, r2) in boxes(traf) {
                if &k2 != b"trun" {
                    continue;
                }
                let trun = &traf[r2];
                let tflags = u32_at(trun, 0)? & 0xff_ffff;
                let count = u32_at(trun, 4)? as usize;
                let mut j = 8;
                if tflags & 1 != 0 {
                    j += 4;
                }
                if tflags & 4 != 0 {
                    j += 4;
                }
                let per = [0x100, 0x200, 0x400, 0x800].iter().filter(|&&f| tflags & f != 0).count() * 4;
                for n in 0..count.min(1 << 20) {
                    let d = if tflags & 0x100 != 0 { u32_at(trun, j + n * per)? } else { default_dur };
                    dur += d as u64;
                }
            }
            let scale = track.timescale as f64;
            start = start.min(base as f64 / scale);
            end = end.max((base + dur) as f64 / scale);
        }
        start.is_finite().then_some((start, end.max(start)))
    }

    /// Add `off` seconds to every `tfdt` in a moof+mdat segment.
    fn shift(&self, seg: &mut [u8], off: f64) -> bool {
        let Some((_, moof)) = boxes(seg).into_iter().find(|(k, _)| k == b"moof") else { return false };
        let mut edits = Vec::new();
        for (k, traf) in boxes(&seg[moof.clone()]) {
            if &k != b"traf" {
                continue;
            }
            let traf = moof.start + traf.start..moof.start + traf.end;
            let parts = boxes(&seg[traf.clone()]);
            let id = parts.iter().find(|(k, _)| k == b"tfhd").and_then(|(_, r)| u32_at(&seg[traf.start + r.start..], 4));
            let Some(track) = id.and_then(|id| self.tracks.get(&id)) else { continue };
            if let Some((_, r)) = parts.iter().find(|(k, _)| k == b"tfdt") {
                let at = traf.start + r.start;
                let v1 = seg[at] == 1;
                let old = if v1 { u64_at(seg, at + 4) } else { u32_at(seg, at + 4).map(|v| v as u64) };
                let new = old.unwrap_or(0) as f64 + (off * track.timescale as f64).round();
                if new < 0.0 || (!v1 && new > u32::MAX as f64) {
                    return false;
                }
                edits.push((at + 4, v1, new as u64));
            }
        }
        for (at, v1, v) in edits {
            if v1 {
                seg[at..at + 8].copy_from_slice(&v.to_be_bytes());
            } else {
                seg[at..at + 4].copy_from_slice(&(v as u32).to_be_bytes());
            }
        }
        true
    }

    /// Handle one complete top-level box.
    fn top_box(&mut self, kind: &[u8; 4], bytes: Vec<u8>, payload: std::ops::Range<usize>, out: &mut Vec<Piece>) -> Result<(), String> {
        match kind {
            b"ftyp" => {
                self.init = bytes;
                self.seg = None;
            }
            b"moov" => {
                let info = self.read_moov(&bytes[payload]);
                if self.tracks.is_empty() {
                    return Err("the initialization segment has no tracks".into());
                }
                let mut init = std::mem::take(&mut self.init);
                init.extend_from_slice(&bytes);
                out.push(Piece::Init(init, info));
            }
            b"moof" => {
                if self.tracks.is_empty() {
                    return Err("a media segment came before the initialization segment".into());
                }
                let (s, e) = self.read_moof(&bytes[payload]).ok_or("unreadable media segment")?;
                self.seg = Some((bytes, s, e));
            }
            b"mdat" => {
                if let Some((mut seg, s, e)) = self.seg.take() {
                    seg.extend_from_slice(&bytes);
                    out.push(Piece::Media(s, e, seg));
                }
            }
            // styp, sidx, emsg, prft, free, skip, mfra, uuid…: not needed to play.
            _ => {}
        }
        Ok(())
    }

    fn push(&mut self, buf: &mut Vec<u8>, out: &mut Vec<Piece>) -> Result<(), String> {
        loop {
            if buf.len() < 8 {
                return Ok(());
            }
            let size = be(&buf[..4]) as usize;
            let kind = [buf[4], buf[5], buf[6], buf[7]];
            if !kind.iter().all(|c| c.is_ascii_graphic() || *c == b' ') {
                return Err("not an MP4 stream".into());
            }
            let (head, size) = match size {
                1 => {
                    if buf.len() < 16 {
                        return Ok(());
                    }
                    (16, be(&buf[8..16]) as usize)
                }
                // A box to the end of the stream: only an mdat does that; wait for all of it.
                0 => return Ok(()),
                s => (8, s),
            };
            if size < head {
                return Err("bad MP4 box size".into());
            }
            if buf.len() < size {
                return Ok(());
            }
            let rest = buf.split_off(size);
            let bytes = std::mem::replace(buf, rest);
            self.top_box(&kind, bytes, head..size, out)?;
        }
    }
}

// ---------------------------------------------------------------- WebM (Matroska)

const EBML: u32 = 0x1A45_DFA3;
const SEGMENT: u32 = 0x1853_8067;
const CLUSTER: u32 = 0x1F43_B675;
const INFO: u32 = 0x1549_A966;
const TRACKS: u32 = 0x1654_AE6B;
/// Elements that sit directly in the Segment: one of these ends an unknown-size Cluster.
const LEVEL1: &[u32] = &[CLUSTER, INFO, TRACKS, 0x114D_9B74, 0x1C53_BB6B, 0x1254_C367, 0x1043_A770, 0x1941_A469, EBML, SEGMENT];

/// An EBML element ID (with its length marker) at `b[i..]`: `(id, length)`.
fn ebml_id(b: &[u8], i: usize) -> Option<(u32, usize)> {
    let first = *b.get(i)?;
    let len = first.leading_zeros() as usize + 1;
    if len > 4 {
        return None;
    }
    Some((be(b.get(i..i + len)?) as u32, len))
}

/// An EBML size at `b[i..]`: `(size or None if unknown, length)`. Err if malformed.
fn ebml_size(b: &[u8], i: usize) -> Result<Option<(Option<u64>, usize)>, String> {
    let Some(&first) = b.get(i) else { return Ok(None) };
    let len = first.leading_zeros() as usize + 1;
    if len > 8 {
        return Err("bad WebM element size".into());
    }
    let Some(raw) = b.get(i..i + len) else { return Ok(None) };
    let mask = if len == 8 { 0 } else { 0xffu8 >> len };
    let mut v = (raw[0] & mask) as u64;
    let mut all_ones = raw[0] & mask == mask;
    for &x in &raw[1..] {
        v = (v << 8) | x as u64;
        all_ones &= x == 0xff;
    }
    Ok(Some((if all_ones { None } else { Some(v) }, len)))
}

/// The child elements of an element's payload: `(id, payload)`.
fn ebml_children(b: &[u8]) -> Vec<(u32, &[u8])> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some((id, il)) = ebml_id(b, i) {
        let Ok(Some((Some(size), sl))) = ebml_size(b, i + il) else { break };
        let start = i + il + sl;
        let Some(end) = start.checked_add(size as usize).filter(|&e| e <= b.len()) else { break };
        out.push((id, &b[start..end]));
        i = end;
    }
    out
}

fn ebml_float(b: &[u8]) -> f64 {
    match b.len() {
        4 => f32::from_bits(be(b) as u32) as f64,
        8 => f64::from_bits(be(b)),
        _ => f64::NAN,
    }
}

#[derive(Default)]
struct WebM {
    in_segment: bool,
    init: Vec<u8>,
    info_seen: bool,
    scale: f64,
    duration: f64,
    /// Track number → (video size and fps, audio, default duration in seconds).
    tracks: HashMap<u64, (Option<(u32, u32)>, bool, f64)>,
    have_init: bool,
}

impl WebM {
    fn new() -> WebM {
        WebM { scale: 1e-3, duration: f64::NAN, ..Default::default() }
    }

    fn read_info(&mut self, b: &[u8]) {
        self.scale = 1e-3;
        self.duration = f64::NAN;
        let mut dur = f64::NAN;
        for (id, c) in ebml_children(b) {
            match id {
                0x2A_D7B1 => self.scale = be(c) as f64 / 1e9,
                0x4489 => dur = ebml_float(c),
                _ => {}
            }
        }
        if dur.is_finite() && dur > 0.0 {
            self.duration = dur * self.scale;
        }
    }

    fn read_tracks(&mut self, b: &[u8]) -> InitInfo {
        self.tracks.clear();
        for (id, entry) in ebml_children(b) {
            if id != 0xAE {
                continue;
            }
            let (mut num, mut kind, mut dd, mut size) = (0, 0, 0.0, None);
            for (cid, c) in ebml_children(entry) {
                match cid {
                    0xD7 => num = be(c),
                    0x83 => kind = be(c),
                    0x23_E383 => dd = be(c) as f64 / 1e9,
                    0xE0 => {
                        let (mut w, mut h) = (0, 0);
                        for (vid, v) in ebml_children(c) {
                            match vid {
                                0xB0 => w = be(v) as u32,
                                0xBA => h = be(v) as u32,
                                _ => {}
                            }
                        }
                        size = Some((w, h));
                    }
                    _ => {}
                }
            }
            self.tracks.insert(num, (if kind == 1 { Some(size.unwrap_or((0, 0))) } else { None }, kind == 2, dd));
        }
        let mut info = InitInfo { duration: self.duration, ..Default::default() };
        for (v, a, dd) in self.tracks.values() {
            if let (Some((w, h)), None) = (v, &info.video) {
                info.video = Some((*w, *h, if *dd > 0.0 { 1.0 / dd } else { 30.0 }));
            }
            info.audio |= a;
        }
        info
    }

    /// Add `off` seconds to a Cluster's timecode (rewriting the Cluster with an 8-byte
    /// timecode, and an 8-byte size if it has one).
    fn shift(&self, seg: &mut Vec<u8>, off: f64) -> bool {
        let Some((CLUSTER, il)) = ebml_id(seg, 0) else { return false };
        let Ok(Some((size, sl))) = ebml_size(seg, il) else { return false };
        let body = &seg[il + sl..];
        let mut out_body = Vec::with_capacity(body.len() + 8);
        let mut i = 0;
        let mut done = false;
        while let Some((id, cl)) = ebml_id(body, i) {
            let Ok(Some((Some(cs), csl))) = ebml_size(body, i + cl) else { break };
            let end = (i + cl + csl + cs as usize).min(body.len());
            if id == 0xE7 && !done {
                let new = be(&body[i + cl + csl..end]) as f64 + (off / self.scale).round();
                if new < 0.0 {
                    return false;
                }
                out_body.extend_from_slice(&[0xE7, 0x88]);
                out_body.extend_from_slice(&(new as u64).to_be_bytes());
                done = true;
            } else {
                out_body.extend_from_slice(&body[i..end]);
            }
            i = end;
        }
        out_body.extend_from_slice(&body[i.min(body.len())..]);
        if !done {
            return false;
        }
        let mut out = vec![0x1F, 0x43, 0xB6, 0x75];
        match size {
            Some(_) => {
                out.push(0x01);
                out.extend_from_slice(&(out_body.len() as u64).to_be_bytes()[1..]);
            }
            None => out.push(0xFF),
        }
        out.extend(out_body);
        *seg = out;
        true
    }

    /// The media time a Cluster's payload covers.
    fn cluster_times(&self, b: &[u8]) -> Option<(f64, f64)> {
        let mut tc: Option<u64> = None;
        let mut blocks: Vec<(u64, i64, f64)> = Vec::new();
        for (id, c) in ebml_children(b) {
            match id {
                0xE7 => tc = Some(be(c)),
                0xA3 => blocks.extend(self.block(c, None)),
                0xA0 => {
                    let mut dur = None;
                    let mut blk = None;
                    for (gid, g) in ebml_children(c) {
                        match gid {
                            0xA1 => blk = Some(g),
                            0x9B => dur = Some(be(g) as f64 * self.scale),
                            _ => {}
                        }
                    }
                    if let Some(g) = blk {
                        blocks.extend(self.block(g, dur));
                    }
                }
                _ => {}
            }
        }
        let tc = tc? as f64;
        if blocks.is_empty() {
            let t = tc * self.scale;
            return Some((t, t));
        }
        let (mut start, mut end) = (f64::INFINITY, f64::NEG_INFINITY);
        // Per track, a block without a known duration lasts until the track's next block,
        // and the last one as long as the smallest step seen.
        let mut by_track: HashMap<u64, Vec<(f64, f64)>> = HashMap::new();
        for (track, rel, d) in blocks {
            by_track.entry(track).or_default().push(((tc + rel as f64) * self.scale, d));
        }
        for (_, mut list) in by_track {
            list.sort_by(|a, b| a.0.total_cmp(&b.0));
            let step = list.windows(2).map(|w| w[1].0 - w[0].0).filter(|&d| d > 0.0).fold(f64::INFINITY, f64::min);
            let step = if step.is_finite() { step } else { 0.0 };
            for (t, d) in list {
                start = start.min(t);
                end = end.max(t + if d > 0.0 { d } else { step });
            }
        }
        Some((start, end.max(start)))
    }

    /// A (Simple)Block's track, relative timecode and duration (known duration or the
    /// track's default; 0 if unknown).
    fn block(&self, b: &[u8], dur: Option<f64>) -> Option<(u64, i64, f64)> {
        let (track, l) = match ebml_size(b, 0) {
            Ok(Some((Some(t), l))) => (t, l),
            _ => return None,
        };
        let rel = i16::from_be_bytes([*b.get(l)?, *b.get(l + 1)?]) as i64;
        let d = dur.unwrap_or_else(|| self.tracks.get(&track).map_or(0.0, |t| t.2));
        Some((track, rel, d))
    }

    fn push(&mut self, buf: &mut Vec<u8>, out: &mut Vec<Piece>, flush: bool) -> Result<(), String> {
        loop {
            let Some((id, il)) = ebml_id(buf, 0) else {
                if buf.len() >= 4 {
                    return Err("not a WebM stream".into());
                }
                return Ok(());
            };
            let Some((size, sl)) = ebml_size(buf, il)? else { return Ok(()) };
            let head = il + sl;
            if !self.in_segment {
                match id {
                    EBML => {
                        let Some(size) = size else { return Err("bad WebM header".into()) };
                        let end = head + size as usize;
                        if buf.len() < end {
                            return Ok(());
                        }
                        let rest = buf.split_off(end);
                        self.init = std::mem::replace(buf, rest);
                        self.info_seen = false;
                    }
                    SEGMENT => {
                        let rest = buf.split_off(head);
                        let h = std::mem::replace(buf, rest);
                        self.init.extend_from_slice(&h);
                        self.in_segment = true;
                    }
                    _ => return Err("not a WebM stream".into()),
                }
                continue;
            }
            match id {
                EBML => {
                    self.in_segment = false;
                    continue;
                }
                CLUSTER => {
                    if !self.have_init {
                        return Err("a media segment came before the initialization segment".into());
                    }
                    let end = match size {
                        Some(s) => {
                            let end = head + s as usize;
                            if buf.len() < end {
                                return Ok(());
                            }
                            end
                        }
                        None => {
                            // Runs until the next top-level element (or the end of the stream).
                            let mut i = head;
                            loop {
                                match ebml_id(buf, i) {
                                    Some((cid, _)) if LEVEL1.contains(&cid) => break i,
                                    Some((_, cl)) => match ebml_size(buf, i + cl)? {
                                        Some((Some(cs), csl)) if buf.len() >= i + cl + csl + cs as usize => i += cl + csl + cs as usize,
                                        Some((None, _)) => return Err("bad WebM cluster".into()),
                                        _ if flush => break buf.len(),
                                        _ => return Ok(()),
                                    },
                                    None if flush && i >= buf.len() => break i,
                                    None => return Ok(()),
                                }
                            }
                        }
                    };
                    let rest = buf.split_off(end);
                    let bytes = std::mem::replace(buf, rest);
                    if let Some((s, e)) = self.cluster_times(&bytes[head..]) {
                        out.push(Piece::Media(s, e, bytes));
                    }
                }
                _ => {
                    let Some(size) = size else { return Err("bad WebM element".into()) };
                    let end = head + size as usize;
                    if buf.len() < end {
                        return Ok(());
                    }
                    let rest = buf.split_off(end);
                    let bytes = std::mem::replace(buf, rest);
                    match id {
                        INFO => {
                            self.read_info(&bytes[head..]);
                            self.init.extend_from_slice(&bytes);
                            self.info_seen = true;
                        }
                        TRACKS => {
                            let info = self.read_tracks(&bytes[head..]);
                            self.init.extend_from_slice(&bytes);
                            self.have_init = true;
                            out.push(Piece::Init(self.init.clone(), info));
                        }
                        // SeekHead, Cues, Tags, Chapters, Void…: not needed to play.
                        _ => {}
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------- a SourceBuffer's parser

enum Kind {
    Mp4(Mp4),
    WebM(WebM),
}

/// Turns appended bytes into init and media segments.
pub struct Parser {
    kind: Kind,
    buf: Vec<u8>,
}

impl Parser {
    /// For a MIME type (`video/mp4; codecs=…`, `audio/webm`…); None if it's neither.
    pub fn for_type(mime: &str) -> Option<Parser> {
        let base = mime.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
        let kind = match base.as_str() {
            "video/mp4" | "audio/mp4" => Kind::Mp4(Mp4::default()),
            "video/webm" | "audio/webm" => Kind::WebM(WebM::new()),
            _ => return None,
        };
        Some(Parser { kind, buf: Vec::new() })
    }

    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Piece>, String> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        match &mut self.kind {
            Kind::Mp4(m) => m.push(&mut self.buf, &mut out)?,
            Kind::WebM(w) => w.push(&mut self.buf, &mut out, false)?,
        }
        Ok(out)
    }

    /// The stream has ended: finish a WebM cluster of unknown size.
    pub fn flush(&mut self) -> Vec<Piece> {
        let mut out = Vec::new();
        if let Kind::WebM(w) = &mut self.kind {
            let _ = w.push(&mut self.buf, &mut out, true);
        }
        out
    }

    /// `timestampOffset`: move a media segment `off` seconds later (earlier if negative) by
    /// rewriting its timestamps. False if it can't (the times would go below zero).
    pub fn shift(&self, seg: &mut Vec<u8>, off: f64) -> bool {
        match &self.kind {
            Kind::Mp4(m) => m.shift(seg, off),
            Kind::WebM(w) => w.shift(seg, off),
        }
    }

    /// `abort()`: drop a half-parsed segment.
    pub fn reset(&mut self) {
        self.buf.clear();
        if let Kind::Mp4(m) = &mut self.kind {
            m.seg = None;
        }
    }
}

// ---------------------------------------------------------------- per-page state and natives

struct Buffer {
    parser: Parser,
    feed: Arc<Feed>,
    info: Option<InitInfo>,
}

#[derive(Default)]
struct Store {
    next: u32,
    buffers: HashMap<u32, Buffer>,
}

fn store(ctx: &Context) -> std::cell::RefMut<'_, Store> {
    ctx.get_data::<RefCell<Store>>().expect("mse store").borrow_mut()
}

fn num(a: &[JsValue], i: usize, ctx: &mut Context) -> f64 {
    a.get(i).map_or(Ok(f64::NAN), |v| v.to_number(ctx)).unwrap_or(f64::NAN)
}

fn string(a: &[JsValue], i: usize, ctx: &mut Context) -> String {
    a.get(i).and_then(|v| v.to_string(ctx).ok()).map(|s| s.to_std_string_escaped()).unwrap_or_default()
}

fn arr(vals: Vec<JsValue>, ctx: &mut Context) -> JsValue {
    JsArray::from_iter(vals, ctx).into()
}

/// Set up MediaSource/SourceBuffer in a page's script engine (after media.js).
pub fn install(ctx: &mut Context) {
    ctx.insert_data(RefCell::new(Store::default()));
    let natives: &[(&str, usize, fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>)] = &[
        ("__mseNew", 1, n_new),
        ("__mseAppend", 3, n_append),
        ("__mseBuffered", 1, n_buffered),
        ("__mseRemove", 3, n_remove),
        ("__mseAbort", 1, n_abort),
        ("__mseChangeType", 2, n_change_type),
        ("__mseEnd", 2, n_end),
        ("__mseFree", 1, n_free),
        ("__mseAttach", 4, n_attach),
    ];
    for (name, len, f) in natives {
        ctx.register_global_callable(JsString::from(*name), *len, NativeFunction::from_fn_ptr(*f)).expect("register mse native");
    }
    if let Err(e) = ctx.eval(Source::from_bytes(include_str!("mse.js").as_bytes())) {
        log!("[ironswift] Media Source Extensions setup failed: {e}");
    }
}

/// A new SourceBuffer for a MIME type: its id, or -1 if the type isn't MP4 or WebM.
fn n_new(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let mime = string(a, 0, ctx);
    let Some(parser) = Parser::for_type(&mime) else { return Ok(JsValue::from(-1)) };
    let mut s = store(ctx);
    s.next += 1;
    let id = s.next;
    s.buffers.insert(id, Buffer { parser, feed: Arc::default(), info: None });
    Ok(JsValue::from(id))
}

/// Append bytes, moving their media segments `timestampOffset` seconds. Returns
/// `[error, sawInit, duration, hasVideo, hasAudio, width, height]`: error is "" on success,
/// "quota" if the buffer is full, else the parse error.
fn n_append(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = num(a, 0, ctx) as u32;
    let offset = num(a, 2, ctx);
    let offset = if offset.is_finite() { offset } else { 0.0 };
    let bytes = match a.get(1).and_then(|v| v.as_object()) {
        Some(o) => JsUint8Array::from_object(o.clone()).ok().and_then(|u| u.to_vec(ctx).ok()).unwrap_or_default(),
        None => Vec::new(),
    };
    let result = {
        let mut s = store(ctx);
        match s.buffers.get_mut(&id) {
            None => Err("this SourceBuffer is gone".to_string()),
            Some(b) if b.feed.bytes() + bytes.len() > MAX_BYTES => Err("quota".to_string()),
            Some(b) => b.parser.push(&bytes).map(|pieces| {
                let mut saw_init = false;
                for p in pieces {
                    match p {
                        Piece::Init(init, info) => {
                            b.feed.set_init(init);
                            b.info = Some(info);
                            saw_init = true;
                        }
                        Piece::Media(start, end, mut bytes) => {
                            if offset != 0.0 && !b.parser.shift(&mut bytes, offset) {
                                continue;
                            }
                            b.feed.add(Seg { start: start + offset, end: end + offset, bytes: Arc::new(bytes) });
                        }
                    }
                }
                (saw_init, b.info.clone().unwrap_or_default())
            }),
        }
    };
    let vals = match result {
        Err(e) => vec![JsValue::from(JsString::from(e))],
        Ok((saw, info)) => {
            let (w, h) = info.video.map_or((0, 0), |v| (v.0, v.1));
            vec![
                JsValue::from(JsString::from("")),
                saw.into(),
                info.duration.into(),
                info.video.is_some().into(),
                info.audio.into(),
                w.into(),
                h.into(),
            ]
        }
    };
    Ok(arr(vals, ctx))
}

/// The buffered ranges as `[start0, end0, start1, end1, …]`.
fn n_buffered(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = num(a, 0, ctx) as u32;
    let ranges = store(ctx).buffers.get(&id).map(|b| b.feed.buffered()).unwrap_or_default();
    let vals = ranges.into_iter().flat_map(|(s, e)| [JsValue::from(s), JsValue::from(e)]).collect();
    Ok(arr(vals, ctx))
}

fn n_remove(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (id, start, end) = (num(a, 0, ctx) as u32, num(a, 1, ctx), num(a, 2, ctx));
    if let Some(b) = store(ctx).buffers.get(&id) {
        b.feed.remove(start, end);
    }
    Ok(JsValue::undefined())
}

fn n_abort(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = num(a, 0, ctx) as u32;
    if let Some(b) = store(ctx).buffers.get_mut(&id) {
        b.parser.reset();
    }
    Ok(JsValue::undefined())
}

/// `changeType(type)`: parse what comes next as another container. False if unsupported.
fn n_change_type(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = num(a, 0, ctx) as u32;
    let mime = string(a, 1, ctx);
    let Some(parser) = Parser::for_type(&mime) else { return Ok(false.into()) };
    if let Some(b) = store(ctx).buffers.get_mut(&id) {
        b.parser = parser;
    }
    Ok(true.into())
}

/// The stream has ended (or reopened, with `false`).
fn n_end(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = num(a, 0, ctx) as u32;
    let ended = a.get(1).is_some_and(|v| v.to_boolean());
    if let Some(b) = store(ctx).buffers.get_mut(&id) {
        if ended {
            for p in b.parser.flush() {
                if let Piece::Media(start, end, bytes) = p {
                    b.feed.add(Seg { start, end, bytes: Arc::new(bytes) });
                }
            }
        }
        b.feed.set_ended(ended);
    }
    Ok(JsValue::undefined())
}

fn n_free(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = num(a, 0, ctx) as u32;
    if let Some(b) = store(ctx).buffers.remove(&id) {
        b.feed.set_ended(true);
    }
    Ok(JsValue::undefined())
}

/// `__mseAttach(elementId, [sourceBufferIds], duration)`: the element plays these buffers:
/// the video from the first with a video track, the sound from the first with sound.
fn n_attach(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let el = num(a, 0, ctx) as crate::dom::NodeId;
    let ids: Vec<u32> = match a.get(1).and_then(|v| v.as_object()).and_then(|o| JsArray::from_object(o.clone()).ok()) {
        Some(list) => {
            let n = list.length(ctx).unwrap_or(0);
            (0..n).filter_map(|i| list.get(i, ctx).ok().and_then(|v| v.to_number(ctx).ok())).map(|n| n as u32).collect()
        }
        None => Vec::new(),
    };
    let duration = num(a, 2, ctx);
    let (video, audio) = {
        let s = store(ctx);
        let find = |want_video: bool| {
            ids.iter().filter_map(|i| s.buffers.get(i)).find(|b| b.info.as_ref().is_some_and(|i| if want_video { i.video.is_some() } else { i.audio })).map(|b| (b.feed.clone(), b.info.clone().unwrap_or_default()))
        };
        (find(true), find(false))
    };
    let mut info = crate::media::Info { duration, ..Default::default() };
    if let Some((_, i)) = &video {
        let (w, h, fps) = i.video.unwrap_or((0, 0, 30.0));
        info.video = true;
        info.width = w;
        info.height = h;
        info.fps = fps;
    }
    info.audio = audio.is_some();
    crate::media::attach_feeds(ctx, el, video.map(|v| v.0), audio.map(|a| a.0), info);
    Ok(JsValue::undefined())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fragmented MP4: ftyp, moov (one video track, timescale 1000, trex default 40),
    /// then two moof+mdat fragments at 0 s and 1 s of 25 samples each.
    fn mp4() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
            let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
            v.extend_from_slice(kind);
            v.extend_from_slice(body);
            v
        }
        let ftyp = bx(b"ftyp", b"isom\0\0\0\x01isomiso6");
        let mut tkhd = vec![0u8; 84];
        tkhd[12..16].copy_from_slice(&1u32.to_be_bytes());
        tkhd[76..80].copy_from_slice(&(320u32 << 16).to_be_bytes());
        tkhd[80..84].copy_from_slice(&(240u32 << 16).to_be_bytes());
        let mut mdhd = vec![0u8; 24];
        mdhd[12..16].copy_from_slice(&1000u32.to_be_bytes());
        let mut hdlr = vec![0u8; 24];
        hdlr[8..12].copy_from_slice(b"vide");
        let trak = bx(b"trak", &[bx(b"tkhd", &tkhd), bx(b"mdia", &[bx(b"mdhd", &mdhd), bx(b"hdlr", &hdlr)].concat())].concat());
        let mut trex = vec![0u8; 24];
        trex[4..8].copy_from_slice(&1u32.to_be_bytes());
        trex[12..16].copy_from_slice(&40u32.to_be_bytes());
        let moov = bx(b"moov", &[trak, bx(b"mvex", &bx(b"trex", &trex))].concat());
        let frag = |t: u32| {
            let mut tfhd = vec![0u8; 8];
            tfhd[4..8].copy_from_slice(&1u32.to_be_bytes());
            let mut tfdt = vec![0u8; 8];
            tfdt[4..8].copy_from_slice(&t.to_be_bytes());
            let mut trun = vec![0u8; 8];
            trun[4..8].copy_from_slice(&25u32.to_be_bytes());
            let traf = bx(b"traf", &[bx(b"tfhd", &tfhd), bx(b"tfdt", &tfdt), bx(b"trun", &trun)].concat());
            [bx(b"moof", &traf), bx(b"mdat", &[7u8; 100])].concat()
        };
        ([ftyp, moov].concat(), frag(0), frag(1000))
    }

    #[test]
    fn splits_mp4_into_segments_with_times() {
        let (init, f1, f2) = mp4();
        let mut p = Parser::for_type("video/mp4; codecs=\"avc1.42E01E\"").unwrap();
        let all = [init.clone(), f1.clone(), f2.clone()].concat();
        // Appended in awkward pieces, as networks deliver them.
        let mut pieces = Vec::new();
        for chunk in all.chunks(37) {
            pieces.extend(p.push(chunk).unwrap());
        }
        assert_eq!(pieces.len(), 3);
        match &pieces[0] {
            Piece::Init(b, info) => {
                assert_eq!(b, &init);
                assert_eq!(info.video, Some((320, 240, 25.0)));
            }
            _ => panic!("init first"),
        }
        assert_eq!(pieces[1], Piece::Media(0.0, 1.0, f1.clone()));
        assert_eq!(pieces[2], Piece::Media(1.0, 2.0, f2));
        // timestampOffset rewrites the tfdt: re-parsing the shifted segment gives the new times.
        let mut moved = f1;
        assert!(p.shift(&mut moved, 10.0));
        let mut q = Parser::for_type("video/mp4").unwrap();
        let again = q.push(&[init, moved].concat()).unwrap();
        assert_eq!(again.len(), 2);
        assert!(matches!(again[1], Piece::Media(s, e, _) if s == 10.0 && e == 11.0));
        assert!(Parser::for_type("video/mp4").unwrap().push(&[0, 0, 0, 9, 0xff, 0, 1, 2, 3]).is_err());
    }

    #[test]
    fn feed_keeps_segments_in_order_and_replaces_overlaps() {
        let f = Feed::default();
        let seg = |s: f64, e: f64, b: u8| Seg { start: s, end: e, bytes: Arc::new(vec![b; 10]) };
        f.add(seg(2.0, 3.0, 3));
        f.add(seg(0.0, 1.0, 1));
        f.add(seg(1.0, 2.0, 2));
        f.add(seg(5.0, 6.0, 5));
        assert_eq!(f.buffered(), vec![(0.0, 3.0), (5.0, 6.0)]);
        f.add(seg(1.0, 2.0, 9));
        assert_eq!(f.bytes(), 40);
        assert_eq!(f.start_for(1.5), Some(1.0));
        assert_eq!(f.start_for(4.0), Some(5.0));
        f.remove(0.0, 2.0);
        assert_eq!(f.buffered(), vec![(2.0, 3.0), (5.0, 6.0)]);
        // The pump writes init then contiguous segments from the start point, and stops at the gap
        // once the stream has ended.
        f.set_init(vec![0xAA]);
        f.add(seg(3.0, 4.0, 4));
        f.set_ended(true);
        let mut out = Vec::new();
        Arc::new(f).pump(Some(2.0), &mut out, Arc::new(AtomicBool::new(false)));
        assert_eq!(out, [vec![0xAA], vec![3; 10], vec![4; 10]].concat());
    }

    #[test]
    fn reads_webm_clusters() {
        // EBML header, Segment (unknown size), Info (scale 1 ms), Tracks (video 64×48), two
        // unknown-size Clusters at 0 and 500 ms with blocks every 50 ms.
        fn el(id: &[u8], body: &[u8]) -> Vec<u8> {
            let mut v = id.to_vec();
            v.push(0x01);
            v.extend_from_slice(&(body.len() as u64).to_be_bytes()[1..]);
            v.extend_from_slice(body);
            v
        }
        let header = el(&[0x1A, 0x45, 0xDF, 0xA3], &el(&[0x42, 0x82], b"webm"));
        let seg_head = [0x18, 0x53, 0x80, 0x67, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF].to_vec();
        let info = el(&[0x15, 0x49, 0xA9, 0x66], &el(&[0x2A, 0xD7, 0xB1], &[0x0F, 0x42, 0x40]));
        let video = el(&[0xE0], &[el(&[0xB0], &[64]), el(&[0xBA], &[48])].concat());
        let tracks = el(&[0x16, 0x54, 0xAE, 0x6B], &el(&[0xAE], &[el(&[0xD7], &[1]), el(&[0x83], &[1]), video].concat()));
        let cluster = |tc: u16| {
            let mut v = vec![0x1F, 0x43, 0xB6, 0x75, 0xFF];
            v.extend(el(&[0xE7], &tc.to_be_bytes()));
            for n in 0..5u8 {
                v.extend(el(&[0xA3], &[0x81, 0, n * 50, 0x80, 1, 2, 3]));
            }
            v
        };
        let mut p = Parser::for_type("video/webm").unwrap();
        let mut pieces = p.push(&[header, seg_head, info, tracks].concat()).unwrap();
        assert!(matches!(&pieces[0], Piece::Init(_, i) if i.video == Some((64, 48, 30.0))));
        pieces = p.push(&cluster(0)).unwrap();
        assert!(pieces.is_empty(), "an unknown-size cluster isn't done until the next one starts");
        // A cluster moved by timestampOffset parses at its new time.
        let mut known = vec![0x1F, 0x43, 0xB6, 0x75, 0x01, 0, 0, 0, 0, 0, 0];
        let body = cluster(0)[5..].to_vec();
        known.push(body.len() as u8);
        known.extend_from_slice(&body);
        assert!(p.shift(&mut known, 2.5));
        let mut w = WebM::new();
        w.read_tracks(&[]);
        assert_eq!(w.cluster_times(&known[12..]), Some((2.5, 2.75)));
        // The next cluster's ID finishes it.
        pieces = p.push(&[0x1F, 0x43, 0xB6, 0x75]).unwrap();
        assert!(matches!(pieces.as_slice(), [Piece::Media(s, e, _)] if *s == 0.0 && (*e - 0.25).abs() < 1e-9), "{:?}", pieces.iter().map(|p| match p { Piece::Media(s, e, b) => (*s, *e, b.len()), _ => (-1.0, -1.0, 0) }).collect::<Vec<_>>());
        // At the end of the stream, a last unknown-size cluster is finished by flush().
        pieces = p.push(&cluster(500)[4..]).unwrap();
        assert!(pieces.is_empty());
        pieces = p.flush();
        assert!(matches!(pieces.as_slice(), [Piece::Media(s, _, _)] if (*s - 0.5).abs() < 1e-9), "{pieces:?}");
    }
}
