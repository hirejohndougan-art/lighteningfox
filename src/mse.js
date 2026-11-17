// Media Source Extensions: MediaSource, SourceBuffer, SourceBufferList. mse.rs splits what
// pages append into segments and feeds them to the decoders; this file is the API and its
// events, and joins a MediaSource to the <video>/<audio> playing it (through the hooks
// media.js leaves in __mediaInternals).
(function () {
  "use strict";
  const g = globalThis;
  if (g.MediaSource || typeof __mseNew !== "function") return;
  const M = g.__mediaInternals;
  if (!M) return;
  const queue = (fn) => (g.__queueTask ? g.__queueTask(fn) : setTimeout(fn, 0));
  const DOMEx = (msg, name) => new g.DOMException(msg, name);
  const emit = (target, type) => target.dispatchEvent(new Event(type));

  // ---- which types can be appended: MP4 and WebM, with codecs ffmpeg decodes, at sizes
  // and rates we play (players probe with width/height/framerate/eotf to pick a stream).
  // [pattern, video?, containers it may be in]
  const CODECS = [
    [/^avc[13](\.[0-9a-f]{6})?$/i, true, "mp4"],
    [/^(hev1|hvc1)(\..+)?$/i, true, "mp4"],
    [/^(vp8|vp8\.0)$/i, true, "webm"],
    [/^(vp9|vp9\.\d+|vp09\.[\d.]+)$/i, true, "webm mp4"],
    [/^av01\.[\w.]+$/i, true, "webm mp4"],
    [/^mp4a\.(40\.\d+|66|67|68|69|6b)$/i, false, "mp4"],
    [/^opus$/i, false, "webm mp4"],
    [/^vorbis$/i, false, "webm"],
    [/^flac$/i, false, "mp4"],
  ];
  // Byte streams without timestamps (they're generated: the "generate timestamps flag").
  const RAW = /^audio\/(mpeg|mp3|aac)$/;
  const generatesTimestamps = (type) => RAW.test((String(type).split(";")[0] || "").trim().toLowerCase());
  function isTypeSupported(type) {
    if (!M.available || type == null) return false;
    const parts = String(type).split(";").map((x) => x.trim());
    const base = (parts.shift() || "").toLowerCase();
    const raw = RAW.exec(base);
    if (raw) {
      for (const p of parts) {
        if (!p) continue;
        const eq = p.indexOf("=");
        if (eq < 0) return false;
        if (p.slice(0, eq).trim().toLowerCase() !== "codecs") continue;
        const list = p.slice(eq + 1).trim().replace(/^"|"$/g, "").split(",").map((c) => c.trim());
        // MSE's MPEG audio byte streams take no codecs parameter at all.
        return false;
      }
      return true;
    }
    const m = /^(video|audio)\/(mp4|webm)$/.exec(base);
    if (!m) return false;
    let sawCodecs = false;
    for (const p of parts) {
      if (!p) continue;
      const eq = p.indexOf("=");
      if (eq < 0) return false;
      const k = p.slice(0, eq).trim().toLowerCase(), v = p.slice(eq + 1).trim().replace(/^"|"$/g, "");
      if (k === "codecs") {
        sawCodecs = true;
        const list = v.split(",").map((c) => c.trim());
        if (!list.length || list.some((c) => !c)) return false;
        for (const c of list) {
          const known = CODECS.find(([re]) => re.test(c));
          if (!known || !known[2].split(" ").includes(m[2]) || (known[1] && m[1] === "audio")) return false;
        }
      } else if (k === "width" && +v > 1920) return false;
      else if (k === "height" && +v > 1080) return false;
      else if (k === "framerate" && +v > 60) return false;
      else if (k === "eotf" && !/^(bt709|srgb)$/i.test(v)) return false;
      else if (k === "channels" && +v > 2) return false;
    }
    // WebM needs to say its codecs.
    return sawCodecs || m[2] === "mp4";
  }

  // ---- TimeRanges helpers
  const pairs = (flat) => { const out = []; for (let i = 0; i + 1 < flat.length; i += 2) out.push([flat[i], flat[i + 1]]); return out; };
  function intersect(a, b) {
    const out = [];
    for (const [s1, e1] of a) for (const [s2, e2] of b) {
      const s = Math.max(s1, s2), e = Math.min(e1, e2);
      if (e > s) out.push([s, e]);
    }
    return out.sort((x, y) => x[0] - y[0]);
  }

  // ---- AudioTrack, VideoTrack and their lists (SourceBuffer's, and the media element's)
  if (!g.TrackEvent) {
    g.TrackEvent = class TrackEvent extends Event {
      constructor(type, init) {
        super(type, init);
        Object.defineProperty(this, "track", { value: (init && init.track) || null, enumerable: true });
      }
    };
  }
  class MediaTrack {
    constructor(id, kind, language, sb) {
      Object.defineProperty(this, "__t", { value: { id, kind, language, sb, on: false, lists: [] } });
    }
    get id() { return this.__t.id; }
    get kind() { return this.__t.kind; }
    get label() { return ""; }
    get language() { return this.__t.language; }
    get sourceBuffer() { return this.__t.sb; }
    __set(on) {
      on = !!on;
      if (on === this.__t.on) return;
      this.__t.on = on;
      for (const l of this.__t.lists) queue(() => emit(l, "change"));
      const sb = this.__t.sb;
      if (sb && !sb.__s.removed) sb.__s.ms.__refreshActive();
    }
  }
  class AudioTrack extends MediaTrack {
    get [Symbol.toStringTag]() { return "AudioTrack"; }
    get enabled() { return this.__t.on; }
    set enabled(v) { this.__set(v); }
  }
  class VideoTrack extends MediaTrack {
    get [Symbol.toStringTag]() { return "VideoTrack"; }
    get selected() { return this.__t.on; }
    set selected(v) {
      // Only one video track is selected at a time.
      if (v) {
        for (const l of this.__t.lists) for (const o of l.__l) if (o !== this && o.__t.on) { o.__t.on = false; if (o.__t.sb) o.__t.sb.__s.ms.__refreshActive(); }
      }
      this.__set(v);
    }
  }
  class TrackList extends EventTarget {
    constructor() {
      super();
      this.onchange = this.onaddtrack = this.onremovetrack = null;
      Object.defineProperty(this, "__l", { value: [] });
    }
    get length() { return this.__l.length; }
    getTrackById(id) { return this.__l.find((t) => t.id === String(id)) || null; }
    [Symbol.iterator]() { return this.__l[Symbol.iterator](); }
    __sync() {
      for (let i = 0; i < this.__l.length + 1; i++) delete this[i];
      this.__l.forEach((t, i) => Object.defineProperty(this, i, { value: t, configurable: true, enumerable: true }));
    }
    __add(t) {
      if (this.__l.includes(t)) return;
      this.__l.push(t);
      t.__t.lists.push(this);
      this.__sync();
      this.dispatchEvent(new g.TrackEvent("addtrack", { track: t }));
    }
    __remove(t) {
      const i = this.__l.indexOf(t);
      if (i < 0) return;
      this.__l.splice(i, 1);
      t.__t.lists = t.__t.lists.filter((l) => l !== this);
      this.__sync();
      this.dispatchEvent(new g.TrackEvent("removetrack", { track: t }));
    }
  }
  class AudioTrackList extends TrackList { get [Symbol.toStringTag]() { return "AudioTrackList"; } }
  class VideoTrackList extends TrackList {
    get [Symbol.toStringTag]() { return "VideoTrackList"; }
    get selectedIndex() { return this.__l.findIndex((t) => t.selected); }
  }
  // media.js: an element's audioTracks/videoTracks.
  M.trackList = (kind) => (kind === "video" ? new VideoTrackList() : new AudioTrackList());
  const elementTracks = (el, video) => {
    const l = el && (video ? el.videoTracks : el.audioTracks);
    return l instanceof TrackList ? l : null;
  };

  // ---- SourceBufferList
  class SourceBufferList extends EventTarget {
    constructor() { super(); this.onaddsourcebuffer = null; this.onremovesourcebuffer = null; Object.defineProperty(this, "__l", { value: [] }); }
    get [Symbol.toStringTag]() { return "SourceBufferList"; }
    get length() { return this.__l.length; }
    [Symbol.iterator]() { return this.__l[Symbol.iterator](); }
    __sync() {
      for (let i = 0; i < this.__l.length + 1; i++) delete this[i];
      this.__l.forEach((sb, i) => Object.defineProperty(this, i, { value: sb, configurable: true, enumerable: true }));
    }
    __add(sb, order) {
      this.__l.push(sb);
      // activeSourceBuffers keeps the order of sourceBuffers.
      if (order) this.__l.sort((a, b) => order.indexOf(a) - order.indexOf(b));
      this.__sync();
      queue(() => emit(this, "addsourcebuffer"));
    }
    __remove(sb) {
      const i = this.__l.indexOf(sb);
      if (i < 0) return;
      this.__l.splice(i, 1);
      this.__sync();
      queue(() => emit(this, "removesourcebuffer"));
    }
  }

  // ---- SourceBuffer
  class SourceBuffer extends EventTarget {
    constructor(ms, type, id) {
      super();
      this.onupdatestart = this.onupdate = this.onupdateend = this.onerror = this.onabort = null;
      Object.defineProperty(this, "__s", { value: {
        ms, id, type, updating: false, mode: generatesTimestamps(type) ? "sequence" : "segments", tsOffset: 0, winStart: 0, winEnd: Infinity,
        hasInit: false, info: null, pending: null, removed: false, genTs: generatesTimestamps(type),
        audio: new AudioTrackList(), video: new VideoTrackList(), tracksMade: false,
      } });
    }
    get [Symbol.toStringTag]() { return "SourceBuffer"; }
    get mode() { return this.__s.mode; }
    set mode(v) {
      v = String(v);
      if (v !== "segments" && v !== "sequence") return;
      this.__check();
      const s = this.__s;
      if (s.genTs && v === "segments") throw new TypeError("This SourceBuffer's byte stream has no timestamps, so its mode must be 'sequence'.");
      if (s.ms.readyState === "ended") s.ms.__reopen();
      this.__checkMid();
      s.mode = v;
      __mseSetMode(s.id, v === "sequence");
    }
    get updating() { return this.__s.updating; }
    get buffered() {
      if (this.__s.removed) throw DOMEx("This SourceBuffer has been removed from its MediaSource.", "InvalidStateError");
      return new g.TimeRanges(pairs(__mseBuffered(this.__s.id)));
    }
    get timestampOffset() { return this.__s.tsOffset; }
    set timestampOffset(v) {
      v = Number(v);
      if (!isFinite(v)) throw new TypeError("The provided double value is non-finite.");
      this.__check();
      const s = this.__s;
      if (s.ms.readyState === "ended") s.ms.__reopen();
      this.__checkMid();
      s.tsOffset = v;
      __mseSetOffset(s.id, v);
    }
    get appendWindowStart() { return this.__s.winStart; }
    set appendWindowStart(v) {
      this.__check();
      v = Number(v);
      if (!(v >= 0) || v >= this.__s.winEnd) throw new TypeError("appendWindowStart is out of range");
      this.__s.winStart = v;
    }
    get appendWindowEnd() { return this.__s.winEnd; }
    set appendWindowEnd(v) {
      this.__check();
      v = Number(v);
      if (isNaN(v) || v <= this.__s.winStart) throw new TypeError("appendWindowEnd is out of range");
      this.__s.winEnd = v;
    }
    get audioTracks() { return this.__s.audio; }
    get videoTracks() { return this.__s.video; }
    get textTracks() { return []; }
    __check() {
      const s = this.__s;
      if (s.removed) throw DOMEx("This SourceBuffer has been removed from its MediaSource.", "InvalidStateError");
      if (s.updating) throw DOMEx("This SourceBuffer is still processing an append or remove.", "InvalidStateError");
    }
    // mode and timestampOffset can't change in the middle of a media segment.
    __checkMid() {
      if (this.__s.midMedia) throw DOMEx("A media segment is still being parsed.", "InvalidStateError");
    }
    // The first init segment: make its tracks, in this SourceBuffer's lists and the element's.
    __makeTracks(flat) {
      const s = this.__s;
      if (s.tracksMade) return;
      s.tracksMade = true;
      const el = s.ms.__s.el;
      for (let i = 0; i + 2 < flat.length; i += 3) {
        const video = !!flat[i + 1];
        const elList = elementTracks(el, video);
        // IDs are unique within the element (two SourceBuffers may both have a track 1).
        let id = String(flat[i]), n = 1;
        while ((elList && elList.getTrackById(id)) || (video ? s.video : s.audio).getTrackById(id)) id = flat[i] + "-" + ++n;
        const t = video ? new VideoTrack(id, "main", String(flat[i + 2]), this) : new AudioTrack(id, "main", String(flat[i + 2]), this);
        // The first audio track is enabled, the first video track selected.
        const others = elList ? elList.__l : (video ? s.video : s.audio).__l;
        t.__t.on = !others.some((o) => o.__t.on);
        (video ? s.video : s.audio).__add(t);
        if (elList) elList.__add(t);
      }
    }
    // The SourceBuffer is going away: so are its tracks.
    __dropTracks(el) {
      const s = this.__s;
      for (const list of [s.audio, s.video]) {
        for (const t of list.__l.slice()) {
          const elList = elementTracks(el, list === s.video);
          list.__remove(t);
          if (elList && elList.__l.includes(t)) {
            elList.__remove(t);
            if (t.__t.on) queue(() => emit(elList, "change"));
          }
          t.__t.sb = null;
        }
      }
    }
    __begin() {
      const s = this.__s;
      this.__check();
      if (s.ms.readyState === "ended") s.ms.__reopen();
      s.updating = true;
      queue(() => emit(this, "updatestart"));
    }
    appendBuffer(data) {
      let bytes;
      if (data instanceof ArrayBuffer) bytes = new Uint8Array(data);
      else if (data && ArrayBuffer.isView(data)) bytes = new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
      else throw new TypeError("Failed to execute 'appendBuffer' on 'SourceBuffer': The provided value is not an ArrayBuffer or ArrayBufferView.");
      this.__check();
      const s = this.__s;
      const r = __mseAppend(s.id, new Uint8Array(bytes), s.tsOffset, s.winStart, s.winEnd);
      if (r[0] === "quota") throw DOMEx("The SourceBuffer is full, and cannot free space to append additional buffers.", "QuotaExceededError");
      this.__begin();
      const token = (s.pending = {});
      queue(() => {
        if (s.pending !== token) return;
        s.pending = null;
        if (r[0]) {
          s.updating = false;
          emit(this, "error");
          emit(this, "updateend");
          s.ms.endOfStream("decode");
          return;
        }
        s.tsOffset = r[7];
        s.midMedia = !!r[9];
        if (r[1]) {
          s.hasInit = true;
          s.info = { duration: r[2], video: r[3], audio: r[4], width: r[5], height: r[6] };
          this.__makeTracks(r[8] || []);
          s.ms.__initReceived(this);
        }
        s.updating = false;
        emit(this, "update");
        emit(this, "updateend");
        s.ms.__appended();
      });
    }
    abort() {
      const s = this.__s;
      if (s.removed) throw DOMEx("This SourceBuffer has been removed from its MediaSource.", "InvalidStateError");
      if (s.ms.readyState !== "open") throw DOMEx("The MediaSource's readyState is not 'open'.", "InvalidStateError");
      if (s.removing) throw DOMEx("abort() can't stop a remove() in progress.", "InvalidStateError");
      if (s.updating) {
        s.pending = null;
        s.updating = false;
        queue(() => { emit(this, "abort"); emit(this, "updateend"); });
      }
      __mseAbort(s.id);
      s.midMedia = false;
      s.winStart = 0;
      s.winEnd = Infinity;
    }
    remove(start, end) {
      const s = this.__s;
      start = Number(start); end = Number(end);
      const d = s.ms.duration;
      if (isNaN(d) || !(start >= 0) || start > d || !(end > start)) throw new TypeError("The start/end of remove() is out of range.");
      this.__begin();
      s.removing = true;
      const token = (s.pending = {});
      queue(() => {
        if (s.pending !== token) return;
        s.pending = null;
        s.removing = false;
        __mseRemove(s.id, start, end);
        s.updating = false;
        emit(this, "update");
        emit(this, "updateend");
      });
    }
    changeType(type) {
      type = String(type);
      if (!type) throw new TypeError("The type provided is empty.");
      this.__check();
      if (!isTypeSupported(type) || !__mseChangeType(this.__s.id, type)) throw DOMEx("The type provided ('" + type + "') is not supported.", "NotSupportedError");
      const s = this.__s;
      s.type = type;
      s.genTs = generatesTimestamps(type);
      if (s.genTs) { s.mode = "sequence"; __mseSetMode(s.id, true); }
    }
  }

  // ---- MediaSource
  const byUrl = new Map();
  class MediaSource extends EventTarget {
    constructor() {
      super();
      this.onsourceopen = this.onsourceended = this.onsourceclose = null;
      Object.defineProperty(this, "__s", { value: {
        readyState: "closed", duration: NaN, el: null, attached: false, live: null,
        list: new SourceBufferList(), active: new SourceBufferList(),
      } });
    }
    get [Symbol.toStringTag]() { return "MediaSource"; }
    static isTypeSupported(type) { return isTypeSupported(type); }
    static get canConstructInDedicatedWorker() { return false; }
    get readyState() { return this.__s.readyState; }
    get sourceBuffers() { return this.__s.list; }
    get activeSourceBuffers() { return this.__s.active; }
    get duration() { return this.__s.readyState === "closed" ? NaN : this.__s.duration; }
    set duration(v) {
      v = Number(v);
      if (isNaN(v) || v < 0) throw new TypeError("The duration provided is negative or NaN.");
      if (this.__s.readyState !== "open") throw DOMEx("The MediaSource's readyState is not 'open'.", "InvalidStateError");
      if (this.__s.list.__l.some((sb) => sb.__s.updating)) throw DOMEx("A SourceBuffer is still updating.", "InvalidStateError");
      this.__setDuration(v);
    }
    get handle() { return this; }
    addSourceBuffer(type) {
      type = String(type);
      if (!type) throw new TypeError("The type provided is empty.");
      if (!isTypeSupported(type)) throw DOMEx("The type provided ('" + type + "') is unsupported.", "NotSupportedError");
      if (this.__s.readyState !== "open") throw DOMEx("The MediaSource's readyState is not 'open'.", "InvalidStateError");
      if (this.__s.list.length >= 4) throw DOMEx("This MediaSource has reached the limit of SourceBuffer objects it can handle.", "QuotaExceededError");
      const id = __mseNew(type);
      if (id < 0) throw DOMEx("The type provided ('" + type + "') is unsupported.", "NotSupportedError");
      const sb = new SourceBuffer(this, type, id);
      this.__s.list.__add(sb);
      return sb;
    }
    removeSourceBuffer(sb) {
      if (!(sb instanceof SourceBuffer)) throw new TypeError("The provided value is not a SourceBuffer.");
      if (!this.__s.list.__l.includes(sb)) throw DOMEx("The SourceBuffer provided is not contained in this MediaSource.", "NotFoundError");
      const s = sb.__s;
      if (s.updating) {
        s.pending = null;
        s.updating = false;
        queue(() => { emit(sb, "abort"); emit(sb, "updateend"); });
      }
      s.removed = true;
      __mseFree(s.id);
      sb.__dropTracks(this.__s.el);
      this.__s.active.__remove(sb);
      this.__s.list.__remove(sb);
    }
    endOfStream(error) {
      const st = this.__s;
      if (st.readyState !== "open") throw DOMEx("The MediaSource's readyState is not 'open'.", "InvalidStateError");
      if (st.list.__l.some((sb) => sb.__s.updating)) throw DOMEx("A SourceBuffer is still updating.", "InvalidStateError");
      if (error !== undefined && error !== "network" && error !== "decode") throw new TypeError("The provided value '" + error + "' is not a valid enum value of type EndOfStreamError.");
      st.readyState = "ended";
      queue(() => emit(this, "sourceended"));
      if (error === undefined) {
        for (const sb of st.list.__l) __mseEnd(sb.__s.id, true);
        // The duration becomes the end of what's buffered.
        let end = 0;
        for (const sb of st.list.__l) { const r = pairs(__mseBuffered(sb.__s.id)); if (r.length) end = Math.max(end, r[r.length - 1][1]); }
        if (end !== st.duration) this.__setDuration(end);
        if (st.el) M.schedule(st.el, 0);
      } else if (st.el) {
        M.fail(st.el, error === "network" ? 2 : 3, error === "network" ? "The media stream failed to download." : "The media couldn't be decoded.");
      }
    }
    setLiveSeekableRange(start, end) {
      if (this.__s.readyState !== "open") throw DOMEx("The MediaSource's readyState is not 'open'.", "InvalidStateError");
      start = Number(start); end = Number(end);
      if (!(start >= 0) || !(end >= start)) throw new TypeError("The start/end provided is out of range.");
      this.__s.live = [start, end];
    }
    clearLiveSeekableRange() {
      if (this.__s.readyState !== "open") throw DOMEx("The MediaSource's readyState is not 'open'.", "InvalidStateError");
      this.__s.live = null;
    }

    // -- inside: the element side
    __setDuration(v) {
      const st = this.__s;
      if (st.duration === v) return;
      st.duration = v;
      if (st.el) M.setDuration(st.el, v);
    }
    __reopen() {
      this.__s.readyState = "open";
      for (const sb of this.__s.list.__l) __mseEnd(sb.__s.id, false);
      queue(() => emit(this, "sourceopen"));
    }
    __attach(el) {
      const st = this.__s;
      if (st.readyState !== "closed") return false;
      st.el = el;
      st.readyState = "open";
      queue(() => emit(this, "sourceopen"));
      return true;
    }
    __detach() {
      const st = this.__s;
      if (!st.el) return;
      for (const sb of st.list.__l) sb.__dropTracks(st.el);
      st.el = null;
      st.attached = false;
      st.readyState = "closed";
      st.duration = NaN;
      for (const sb of st.list.__l.slice()) { sb.__s.removed = true; __mseFree(sb.__s.id); st.active.__remove(sb); st.list.__remove(sb); }
      queue(() => emit(this, "sourceclose"));
    }
    // Every SourceBuffer has its init segment: the element has its metadata.
    __initReceived(sb) {
      const st = this.__s;
      this.__refreshActive();
      if (isNaN(st.duration)) st.duration = isFinite(sb.__s.info.duration) && sb.__s.info.duration > 0 ? sb.__s.info.duration : Infinity;
      if (st.attached || !st.el || !st.list.__l.every((b) => b.__s.hasInit)) return;
      st.attached = true;
      __mseAttach(st.el.__id, st.list.__l.map((b) => b.__s.id), st.duration);
      // Every init segment is in: the element has its metadata now (not when the decoder starts).
      const v = st.list.__l.map((b) => b.__s.info).find((i) => i.video);
      if (M.metadata) M.metadata(st.el, st.duration, v ? v.width : 0, v ? v.height : 0);
      M.schedule(st.el, 0);
    }
    // activeSourceBuffers: those with an enabled audio track or a selected video track (or,
    // before their tracks are known, any with an init segment).
    __refreshActive() {
      const st = this.__s;
      for (const sb of st.list.__l) {
        const s = sb.__s;
        const tracks = s.audio.__l.concat(s.video.__l);
        const want = s.hasInit && (!tracks.length || tracks.some((t) => t.__t.on));
        const has = st.active.__l.includes(sb);
        if (want && !has) st.active.__add(sb, st.list.__l);
        else if (!want && has) st.active.__remove(sb);
      }
    }
    __appended() { if (this.__s.el) M.schedule(this.__s.el, 0); }
    // For the element: what's buffered in every SourceBuffer, and is playback at `t` ready?
    __buffered() {
      const l = this.__s.list.__l;
      if (!l.length) return [];
      const lists = l.map((sb) => pairs(__mseBuffered(sb.__s.id)));
      // When the stream has ended, each one's last range runs to the end of the longest.
      if (this.__s.readyState === "ended") {
        const highest = Math.max(...lists.map((r) => (r.length ? r[r.length - 1][1] : -Infinity)));
        for (const r of lists) if (r.length) r[r.length - 1][1] = highest;
      }
      let r = lists[0];
      for (const x of lists.slice(1)) r = intersect(r, x);
      return r;
    }
    __ready(t) {
      const st = this.__s;
      if (st.readyState === "ended") return true;
      for (const [s, e] of this.__buffered()) if (t >= s - 0.15 && t < e - 0.1) return true;
      return false;
    }
    __seekable() {
      const st = this.__s;
      if (isFinite(st.duration)) return [[0, st.duration]];
      if (st.live) return [st.live];
      const b = this.__buffered();
      return b.length ? [[0, b[b.length - 1][1]]] : [];
    }
  }

  // URL.createObjectURL(mediaSource) (channels.js keeps blob: URLs; it may load later).
  g.__mseUrlFor = function (ms) {
    if (!(ms instanceof MediaSource)) return null;
    const o = g.location && g.location.origin && g.location.origin !== "null" ? g.location.origin : "null";
    const u = "blob:" + o + "/" + "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, (c) => { const r = (Math.random() * 16) | 0; return (c === "x" ? r : (r & 3) | 8).toString(16); });
    byUrl.set(u, ms);
    return u;
  };
  g.__mseRevoke = (u) => byUrl.delete(String(u));
  // media.js: is this element's new src a MediaSource? Then it plays that.
  M.attachSource = function (el, url) {
    const ms = byUrl.get(String(url).split("#")[0]);
    if (!ms || !ms.__attach(el)) return null;
    return ms;
  };

  // media.js: srcObject = mediaSource.
  M.isMediaSource = (v) => v instanceof MediaSource;
  M.attachObject = (el, ms) => (ms instanceof MediaSource && ms.__attach(el) ? ms : null);

  g.MediaSource = MediaSource;
  g.AudioTrack = AudioTrack;
  g.VideoTrack = VideoTrack;
  g.AudioTrackList = AudioTrackList;
  g.VideoTrackList = VideoTrackList;
  g.SourceBuffer = SourceBuffer;
  g.SourceBufferList = SourceBufferList;
})();
