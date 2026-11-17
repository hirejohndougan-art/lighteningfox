//! Media Source Extensions at the level of coded frames (for mse.rs).
//!
//! Every media segment a SourceBuffer keeps carries the list of its coded frames: track,
//! presentation time, duration, whether it's a random access point, and where its data sits
//! in the segment. That gives per-track buffered ranges (intersected across tracks, as the
//! spec's `buffered` is), lets `remove()`, overlapping appends and the append window drop
//! single frames (and the frames that depend on them) instead of whole segments, and lets a
//! segment be rewritten with only the frames that are left:
//!
//! - MP4: a new `moof` with one `traf` per run of frames (its own `tfdt`, a `trun` that spells
//!   out every sample) and an `mdat` holding just those frames' data;
//! - WebM: the Cluster without the dropped (Simple)Block elements;
//! - raw ADTS AAC / MP3 (`audio/aac`, `audio/mpeg`): the kept frames' bytes, one after another.
//!
//! The raw audio formats have no timestamps of their own ("generate timestamps"): this file
//! also splits them into frames and gives each one its time from the frame's sample count.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::mse::{boxes, child, ebml_id, ebml_size};

/// The container a media segment is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Fmt {
    #[default]
    Mp4,
    WebM,
    Raw,
}

/// One coded frame of a media segment.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Frame {
    /// Track ID (MP4) or track number (WebM); 1 for raw audio.
    pub track: u64,
    /// Presentation time and duration, in seconds.
    pub pts: f64,
    pub dur: f64,
    /// A random access point (key frame): decoding can start here.
    pub key: bool,
    /// MP4 and raw: the byte offset and length of the frame's data in its segment (offset
    /// `u32::MAX` if it can't be located, so the segment can't be rewritten). WebM: the index
    /// of its block element among the Cluster's blocks.
    pub off: u32,
    pub len: u32,
    /// MP4: decode time, duration, sample flags and composition offset, in `scale` units.
    pub dts: u64,
    pub tdur: u32,
    pub flags: u32,
    pub cto: i32,
    pub scale: u32,
}

impl Frame {
    pub fn end(&self) -> f64 {
        self.pts + self.dur
    }
}

/// A media segment found in appended bytes, with its coded frames (in decode order).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaSeg {
    pub fmt: Fmt,
    pub bytes: Vec<u8>,
    pub frames: Vec<Frame>,
}

impl MediaSeg {
    /// The earliest presentation time, and the latest end, of its frames.
    pub fn span(&self) -> Option<(f64, f64)> {
        span(&self.frames)
    }
}

pub fn span(frames: &[Frame]) -> Option<(f64, f64)> {
    let start = frames.iter().map(|f| f.pts).fold(f64::INFINITY, f64::min);
    let end = frames.iter().map(|f| f.end()).fold(f64::NEG_INFINITY, f64::max);
    start.is_finite().then_some((start, end.max(start)))
}

/// Move frames `off` seconds. Their MP4 decode times move as `Parser::shift` moves `tfdt`:
/// by as much of `off` as keeps each track's earliest one at 0 or after.
pub fn shift_frames(frames: &mut [Frame], off: f64) {
    let mut first: HashMap<u64, u64> = HashMap::new();
    for f in frames.iter().filter(|f| f.scale > 0) {
        let e = first.entry(f.track).or_insert(f.dts);
        *e = (*e).min(f.dts);
    }
    for f in frames {
        f.pts += off;
        if f.scale > 0 {
            let delta = (off * f.scale as f64).round() as i64;
            let delta = delta.max(-(first[&f.track] as i64));
            f.dts = (f.dts as i64 + delta) as u64;
        }
    }
}

/// Which frames to keep, going through them in decode order: `drop` says which to take out,
/// and after a frame is taken out the frames of its track that follow it are taken out too,
/// up to the next random access point (they can't be decoded without it). `need_key` carries
/// that across segments.
pub fn keep_mask(frames: &[Frame], need_key: &mut HashSet<u64>, drop: impl Fn(&Frame) -> bool) -> Vec<bool> {
    frames
        .iter()
        .map(|f| {
            if drop(f) {
                need_key.insert(f.track);
                false
            } else if need_key.contains(&f.track) {
                if f.key {
                    need_key.remove(&f.track);
                    true
                } else {
                    false
                }
            } else {
                true
            }
        })
        .collect()
}

/// A segment with only the frames in `kept` (a subsequence of its frames): the new bytes and
/// frames, or None if it can't be rewritten.
pub fn rebuild(fmt: Fmt, bytes: &[u8], kept: &[Frame]) -> Option<(Vec<u8>, Vec<Frame>)> {
    match fmt {
        Fmt::Mp4 => mp4_rebuild(bytes, kept),
        Fmt::WebM => webm_rebuild(bytes, kept),
        Fmt::Raw => raw_rebuild(bytes, kept),
    }
}

fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    v.extend_from_slice(kind);
    v.extend_from_slice(body);
    v
}

fn mp4_rebuild(bytes: &[u8], kept: &[Frame]) -> Option<(Vec<u8>, Vec<Frame>)> {
    if kept.iter().any(|f| f.off == u32::MAX || f.off as usize + f.len as usize > bytes.len()) {
        return None;
    }
    // Runs of frames of one track whose decode times follow on: one traf each.
    let mut runs: Vec<Vec<Frame>> = Vec::new();
    for f in kept {
        match runs.last_mut() {
            Some(r) if r.last().is_some_and(|p| p.track == f.track && p.dts + p.tdur as u64 == f.dts) => r.push(*f),
            _ => runs.push(vec![*f]),
        }
    }
    let mfhd = boxes(bytes)
        .into_iter()
        .find(|(k, _)| k == b"moof")
        .and_then(|(_, r)| child(&bytes[r], b"mfhd").map(|m| m.to_vec()))
        .unwrap_or_else(|| vec![0; 8]);
    let traf_size = |n: usize| 8 + 16 + 20 + 20 + 16 * n;
    let moof_size = 8 + 8 + mfhd.len() + runs.iter().map(|r| traf_size(r.len())).sum::<usize>();
    let mut moof_body = bx(b"mfhd", &mfhd);
    let mut mdat = Vec::with_capacity(kept.iter().map(|f| f.len as usize).sum());
    let mut out_frames = Vec::with_capacity(kept.len());
    for run in &runs {
        let data_at = moof_size + 8 + mdat.len();
        let mut tfhd = 0x0002_0000u32.to_be_bytes().to_vec(); // default-base-is-moof
        tfhd.extend_from_slice(&(run[0].track as u32).to_be_bytes());
        let mut tfdt = 0x0100_0000u32.to_be_bytes().to_vec(); // version 1
        tfdt.extend_from_slice(&run[0].dts.to_be_bytes());
        // version 1 (signed offsets); data offset, duration, size, flags, composition offset
        let mut trun = 0x0100_0f01u32.to_be_bytes().to_vec();
        trun.extend_from_slice(&(run.len() as u32).to_be_bytes());
        trun.extend_from_slice(&(data_at as i32).to_be_bytes());
        for f in run {
            trun.extend_from_slice(&f.tdur.to_be_bytes());
            trun.extend_from_slice(&f.len.to_be_bytes());
            trun.extend_from_slice(&f.flags.to_be_bytes());
            trun.extend_from_slice(&f.cto.to_be_bytes());
            let mut nf = *f;
            nf.off = (moof_size + 8 + mdat.len()) as u32;
            mdat.extend_from_slice(&bytes[f.off as usize..f.off as usize + f.len as usize]);
            out_frames.push(nf);
        }
        let mut traf = bx(b"tfhd", &tfhd);
        traf.extend(bx(b"tfdt", &tfdt));
        traf.extend(bx(b"trun", &trun));
        moof_body.extend(bx(b"traf", &traf));
    }
    let mut out = bx(b"moof", &moof_body);
    debug_assert_eq!(out.len(), moof_size);
    out.extend(bx(b"mdat", &mdat));
    Some((out, out_frames))
}

/// A Cluster's child elements: `(id, whole element range)`, stopping at anything malformed.
fn cluster_children(bytes: &[u8]) -> Option<(usize, Vec<(u32, std::ops::Range<usize>)>)> {
    let (id, il) = ebml_id(bytes, 0)?;
    if id != 0x1F43_B675 {
        return None;
    }
    let (size, sl) = ebml_size(bytes, il).ok()??;
    let head = il + sl;
    let end = size.map_or(bytes.len(), |s| (head + s as usize).min(bytes.len()));
    let mut out = Vec::new();
    let mut i = head;
    while i < end {
        let Some((cid, cl)) = ebml_id(bytes, i) else { break };
        let Ok(Some((Some(cs), csl))) = ebml_size(bytes, i + cl) else { break };
        let e = i + cl + csl + cs as usize;
        if e > end {
            break;
        }
        out.push((cid, i..e));
        i = e;
    }
    Some((head, out))
}

fn webm_rebuild(bytes: &[u8], kept: &[Frame]) -> Option<(Vec<u8>, Vec<Frame>)> {
    let (_, children) = cluster_children(bytes)?;
    let keep: HashSet<u32> = kept.iter().map(|f| f.off).collect();
    let mut body = Vec::with_capacity(bytes.len());
    let mut renumber = HashMap::new();
    let (mut old, mut new) = (0u32, 0u32);
    for (id, r) in children {
        if id == 0xA3 || id == 0xA0 {
            let i = old;
            old += 1;
            if !keep.contains(&i) {
                continue;
            }
            renumber.insert(i, new);
            new += 1;
        }
        body.extend_from_slice(&bytes[r]);
    }
    let mut out = vec![0x1F, 0x43, 0xB6, 0x75, 0x01];
    out.extend_from_slice(&(body.len() as u64).to_be_bytes()[1..]);
    out.extend(body);
    let frames = kept.iter().map(|f| Frame { off: renumber.get(&f.off).copied().unwrap_or(f.off), ..*f }).collect();
    Some((out, frames))
}

fn raw_rebuild(bytes: &[u8], kept: &[Frame]) -> Option<(Vec<u8>, Vec<Frame>)> {
    let mut out = Vec::new();
    let mut frames = Vec::with_capacity(kept.len());
    for f in kept {
        let r = f.off as usize..f.off as usize + f.len as usize;
        let data = bytes.get(r)?;
        frames.push(Frame { off: out.len() as u32, ..*f });
        out.extend_from_slice(data);
    }
    Some((out, frames))
}

/// Per-track buffered ranges of a set of frames, then the ranges every track in `tracks` has
/// (all the tracks seen, if `tracks` is empty). With `ended`, each track's last range runs to
/// the highest end time of any track (as the spec's `buffered` does after `endOfStream()`).
pub fn buffered<'a>(frames: impl Iterator<Item = &'a Frame>, tracks: &[u64], ended: bool) -> Vec<(f64, f64)> {
    // Per track: the frames' spans, and the shortest frame duration.
    let mut per: BTreeMap<u64, (Vec<(f64, f64)>, f64)> = BTreeMap::new();
    for f in frames {
        let e = per.entry(f.track).or_insert((Vec::new(), f64::INFINITY));
        e.0.push((f.pts, f.end()));
        if f.dur > 0.0 {
            e.1 = e.1.min(f.dur);
        }
    }
    if per.is_empty() {
        return Vec::new();
    }
    let wanted: Vec<u64> = if tracks.is_empty() { per.keys().copied().collect() } else { tracks.to_vec() };
    let mut lists = Vec::new();
    for t in &wanted {
        let Some((mut list, min_dur)) = per.remove(t) else { return Vec::new() };
        list.sort_by(|a, b| a.0.total_cmp(&b.0));
        // Frames closer than this count as touching: up to about one missing frame. (A frame's
        // duration is its decode duration, which with B-frames doesn't match the spacing of
        // presentation times; the usual spacing is the median step between them.)
        let mut steps: Vec<f64> = list.windows(2).map(|w| w[1].0 - w[0].0).filter(|&d| d > 1e-6).collect();
        steps.sort_by(|a, b| a.total_cmp(b));
        let typical = steps.get(steps.len() / 2).copied().unwrap_or(if min_dur.is_finite() { min_dur } else { 0.0 });
        let fudge = (2.0 * typical).clamp(1e-3, 0.25);
        let mut merged: Vec<(f64, f64)> = Vec::new();
        for (s, e) in list {
            match merged.last_mut() {
                Some(last) if s <= last.1 + fudge => last.1 = last.1.max(e),
                _ => merged.push((s, e)),
            }
        }
        lists.push(merged);
    }
    if ended {
        let highest = lists.iter().filter_map(|l| l.last().map(|r| r.1)).fold(f64::NEG_INFINITY, f64::max);
        for l in &mut lists {
            if let Some(last) = l.last_mut() {
                last.1 = highest;
            }
        }
    }
    let mut out = lists.pop().unwrap_or_default();
    for l in lists {
        out = intersect(&out, &l);
    }
    out
}

pub fn intersect(a: &[(f64, f64)], b: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    for &(s1, e1) in a {
        for &(s2, e2) in b {
            let (s, e) = (s1.max(s2), e1.min(e2));
            if e > s {
                out.push((s, e));
            }
        }
    }
    out.sort_by(|x, y| x.0.total_cmp(&y.0));
    out
}

// ---------------------------------------------------------------- raw ADTS AAC and MP3

/// Splits an ADTS (`audio/aac`) or MPEG audio (`audio/mpeg`) byte stream into frames.
#[derive(Default)]
pub struct RawAudio {
    pub mp3: bool,
    /// Sample rate and channel count of the first frame (0 before one is seen).
    pub rate: u32,
    pub channels: u32,
}

/// A frame header at `b[0..]`: `(frame length, samples, sample rate, channels)`.
fn adts_header(b: &[u8]) -> Option<(usize, u32, u32, u32)> {
    const RATES: [u32; 13] = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350];
    if b.len() < 7 || b[0] != 0xFF || b[1] & 0xF6 != 0xF0 {
        return None;
    }
    let rate = *RATES.get(((b[2] >> 2) & 0xF) as usize)?;
    let len = ((b[3] as usize & 3) << 11) | ((b[4] as usize) << 3) | (b[5] as usize >> 5);
    let blocks = (b[6] & 3) as u32 + 1;
    let channels = (((b[2] & 1) << 2) | (b[3] >> 6)) as u32;
    (len >= 7).then_some((len, 1024 * blocks, rate, channels.max(1)))
}

fn mp3_header(b: &[u8]) -> Option<(usize, u32, u32, u32)> {
    if b.len() < 4 || b[0] != 0xFF || b[1] & 0xE0 != 0xE0 {
        return None;
    }
    let version = (b[1] >> 3) & 3; // 3: MPEG-1, 2: MPEG-2, 0: MPEG-2.5
    let layer = (b[1] >> 1) & 3; // 3: I, 2: II, 1: III
    let (bi, si, pad) = ((b[2] >> 4) as usize, ((b[2] >> 2) & 3) as usize, ((b[2] >> 1) & 1) as usize);
    if version == 1 || layer == 0 || bi == 0 || bi == 15 || si == 3 {
        return None;
    }
    let rate = [44100, 48000, 32000][si] >> if version == 3 { 0 } else if version == 2 { 1 } else { 2 };
    const V1: [[u32; 15]; 3] = [
        [0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448],
        [0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384],
        [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320],
    ];
    const V2: [[u32; 15]; 2] = [[0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256], [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160]];
    let li = (3 - layer) as usize; // 0: I, 1: II, 2: III
    let kbps = if version == 3 { V1[li][bi] } else { V2[li.min(1)][bi] } as usize;
    let samples = match (li, version) {
        (0, _) => 384,
        (1, _) | (2, 3) => 1152,
        _ => 576,
    };
    let len = if li == 0 { (12 * kbps * 1000 / rate as usize + pad) * 4 } else { samples / 8 * kbps * 1000 / rate as usize + pad };
    let channels = if b[3] >> 6 == 3 { 1 } else { 2 };
    (len >= 4).then_some((len, samples as u32, rate, channels))
}

impl RawAudio {
    pub fn new(mp3: bool) -> RawAudio {
        RawAudio { mp3, ..Default::default() }
    }

    /// Take the complete frames off the front of `buf` as one segment, timed from 0.
    pub fn push(&mut self, buf: &mut Vec<u8>, flush: bool) -> Option<MediaSeg> {
        let mut i = 0;
        let mut seg = MediaSeg { fmt: Fmt::Raw, ..Default::default() };
        let mut t = 0.0;
        loop {
            let rest = &buf[i..];
            // An ID3v2 tag (MP3 files often start with one): skip it.
            if rest.len() >= 10 && &rest[..3] == b"ID3" {
                let size = rest[6..10].iter().fold(0usize, |a, &x| (a << 7) | (x & 0x7f) as usize) + 10 + if rest[5] & 0x10 != 0 { 10 } else { 0 };
                if rest.len() < size {
                    break;
                }
                i += size;
                continue;
            }
            // Wait for a whole header (and an ID3 tag's), unless the stream has ended.
            if rest.len() < if self.mp3 { 4 } else { 7 } || (!flush && rest.len() < 10 && rest.starts_with(&b"ID3"[..rest.len().min(3)])) {
                break;
            }
            let header = if self.mp3 { mp3_header(rest) } else { adts_header(rest) };
            let Some((len, samples, rate, channels)) = header else {
                i += 1; // not at a frame: look for the next sync word
                continue;
            };
            if rest.len() < len {
                break;
            }
            if self.rate == 0 {
                self.rate = rate;
                self.channels = channels;
            }
            let dur = samples as f64 / rate as f64;
            seg.frames.push(Frame { track: 1, pts: t, dur, key: true, off: seg.bytes.len() as u32, len: len as u32, ..Default::default() });
            seg.bytes.extend_from_slice(&rest[..len]);
            t += dur;
            i += len;
        }
        buf.drain(..i.min(buf.len()));
        (!seg.frames.is_empty()).then_some(seg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(track: u64, pts: f64, key: bool) -> Frame {
        Frame { track, pts, dur: 0.1, key, ..Default::default() }
    }

    #[test]
    fn dropping_a_frame_drops_its_dependents_until_the_next_key_frame() {
        let frames = [f(1, 0.0, true), f(1, 0.1, false), f(1, 0.2, false), f(2, 0.0, true), f(1, 0.3, true), f(1, 0.4, false)];
        let mut need = HashSet::new();
        let mask = keep_mask(&frames, &mut need, |fr| fr.track == 1 && (0.1..0.15).contains(&fr.pts));
        assert_eq!(mask, [true, false, false, true, true, true]);
    }

    #[test]
    fn buffered_intersects_tracks_and_stretches_the_last_range_after_end_of_stream() {
        let frames: Vec<Frame> = (0..10).map(|i| f(1, i as f64 * 0.1, true)).chain((2..12).map(|i| f(2, i as f64 * 0.1, true))).collect();
        let r = buffered(frames.iter(), &[1, 2], false);
        assert_eq!(r.len(), 1);
        assert!((r[0].0 - 0.2).abs() < 1e-9 && (r[0].1 - 1.0).abs() < 1e-9, "{r:?}");
        let r = buffered(frames.iter(), &[1, 2], true);
        assert!((r[0].1 - 1.2).abs() < 1e-9, "{r:?}");
        // A missing track: nothing is buffered.
        assert!(buffered(frames.iter(), &[1, 3], false).is_empty());
        // A gap of several frames splits a range.
        let gappy = [f(1, 0.0, true), f(1, 0.1, true), f(1, 0.9, true)];
        assert_eq!(buffered(gappy.iter(), &[], false).len(), 2);
    }

    #[test]
    fn splits_adts_and_mp3_streams_into_timed_frames() {
        // ADTS: 44.1 kHz, mono, 10-byte frames.
        let mut frame = vec![0xFF, 0xF1, 0x50, 0x40, 0x01, 0x40, 0xFC];
        frame.extend_from_slice(&[1, 2, 3]);
        let len = frame.len();
        frame[3] = 0x40 | ((len >> 11) & 3) as u8;
        frame[4] = (len >> 3) as u8;
        frame[5] = ((len & 7) << 5) as u8 | 0x1F;
        let mut buf: Vec<u8> = frame.iter().chain(frame.iter()).chain(frame[..4].iter()).copied().collect();
        let mut p = RawAudio::new(false);
        let seg = p.push(&mut buf, false).unwrap();
        assert_eq!(seg.frames.len(), 2);
        assert!((seg.frames[1].pts - 1024.0 / 44100.0).abs() < 1e-9);
        assert_eq!(buf.len(), 4, "half a frame waits for the rest");
        assert_eq!(p.rate, 44100);
        // MP3: MPEG-1 layer III, 128 kbps, 44.1 kHz: 417-byte frames of 1152 samples.
        let mut mp3 = vec![0xFF, 0xFB, 0x90, 0x00];
        mp3.resize(417, 0);
        let mut buf: Vec<u8> = b"ID3\x04\x00\x00\x00\x00\x00\x02xx".iter().chain(mp3.iter()).chain(mp3.iter()).copied().collect();
        let seg = RawAudio::new(true).push(&mut buf, false).unwrap();
        assert_eq!(seg.frames.len(), 2);
        assert!((seg.frames[1].pts - 1152.0 / 44100.0).abs() < 1e-9);
        let (bytes, frames) = rebuild(Fmt::Raw, &seg.bytes, &seg.frames[1..]).unwrap();
        assert_eq!(bytes.len(), 417);
        assert_eq!(frames[0].off, 0);
    }
}
