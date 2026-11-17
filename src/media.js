// <video> and <audio>: HTMLMediaElement (the load algorithm, the playback clock, events,
// TimeRanges, MediaError, new Audio()) and the built-in controls. media.rs does the decoding
// (with ffmpeg) and draws the element's box; this file decides what happens when.
(function () {
  "use strict";
  const g = globalThis;
  if (g.__mediaInstalled) return;
  g.__mediaInstalled = true;

  const NETWORK_EMPTY = 0, NETWORK_IDLE = 1, NETWORK_LOADING = 2, NETWORK_NO_SOURCE = 3;
  const HAVE_NOTHING = 0, HAVE_METADATA = 1, HAVE_CURRENT_DATA = 2, HAVE_FUTURE_DATA = 3, HAVE_ENOUGH_DATA = 4;
  const CONSTS = { NETWORK_EMPTY, NETWORK_IDLE, NETWORK_LOADING, NETWORK_NO_SOURCE, HAVE_NOTHING, HAVE_METADATA, HAVE_CURRENT_DATA, HAVE_FUTURE_DATA, HAVE_ENOUGH_DATA };
  // Flags for __mdRender (media.rs).
  const F_CONTROLS = 1, F_PAUSED = 2, F_MUTED = 4, F_SHOW_POSTER = 8, F_AUDIO = 16, F_ERROR = 32;
  const BAR_H = 32;
  const AVAILABLE = __mdAvailable();
  const queue = (fn) => (g.__queueTask ? g.__queueTask(fn) : setTimeout(fn, 0));
  const now = () => (g.performance && g.performance.now ? g.performance.now() : Date.now());
  const isMedia = (el) => el != null && typeof el === "object" && el.nodeType === 1 && (el.localName === "video" || el.localName === "audio");

  // ---- MediaError, TimeRanges
  const ERR_NAMES = { MEDIA_ERR_ABORTED: 1, MEDIA_ERR_NETWORK: 2, MEDIA_ERR_DECODE: 3, MEDIA_ERR_SRC_NOT_SUPPORTED: 4 };
  class MediaError {
    constructor(code, message) {
      Object.defineProperty(this, "code", { value: code, enumerable: true });
      Object.defineProperty(this, "message", { value: String(message || ""), enumerable: true });
    }
  }
  Object.assign(MediaError, ERR_NAMES);
  Object.assign(MediaError.prototype, ERR_NAMES);
  class TimeRanges {
    constructor(ranges) { Object.defineProperty(this, "__r", { value: ranges }); }
    get length() { return this.__r.length; }
    start(i) { return this.__at(i)[0]; }
    end(i) { return this.__at(i)[1]; }
    __at(i) {
      i = Number(i) >>> 0;
      if (i >= this.__r.length) throw new DOMException("index out of range", "IndexSizeError");
      return this.__r[i];
    }
  }
  g.MediaError = MediaError;
  g.TimeRanges = TimeRanges;
  for (const C of [g.HTMLMediaElement, g.HTMLVideoElement, g.HTMLAudioElement]) if (C) Object.assign(C, CONSTS);

  // ---- per-element state, keyed by node id
  const states = new Map();
  function S(el) {
    let s = states.get(el.__id);
    if (!s) {
      s = {
        el, networkState: NETWORK_EMPTY, readyState: HAVE_NOTHING, paused: true, ended: false, seeking: false,
        time: 0, base: 0, anchor: 0, duration: NaN, vw: 0, vh: 0, rate: 1, defaultRate: 1, volume: 1,
        muted: el.hasAttribute("muted"), error: null, currentSrc: "", gen: 0, timer: null, lastUpdate: 0,
        showPoster: true, played: [], waitingPlays: [], pendingSeek: null, deferred: false, autoplayed: false,
        wasConnected: false, poster: null, preservesPitch: true, sinkId: "",
      };
      states.set(el.__id, s);
      // A parsed element starts loading as soon as anything looks at it (as if the parser
      // had started it), not only once the page has finished parsing.
      if (el.isConnected) discover(el);
    }
    return s;
  }
  const fire = (el, type) => el.dispatchEvent(new Event(type, { bubbles: false, cancelable: type === "invalid" }));
  // Events from before a new load are dropped, except these.
  const UNGATED = new Set(["emptied", "abort", "ratechange", "volumechange"]);
  const queueFire = (el, type) => { const gen = S(el).gen; queue(() => { if (S(el).gen === gen || UNGATED.has(type)) fire(el, type); }); };

  function resolve(u) {
    try { return new URL(String(u), document.baseURI || location.href).href; } catch (e) { return ""; }
  }

  // ---- what can play: anything ffmpeg can open; types we know get "maybe"/"probably".
  const CONTAINERS = /^(video|audio)\/(mp4|webm|ogg|mpeg|mp3|x-m4a|m4a|aac|flac|x-flac|wav|x-wav|wave|quicktime|x-matroska|matroska|3gpp|opus)$/;
  const CODECS = /^(avc1|avc3|hev1|hvc1|mp4a|vp8|vp08|vp9|vp09|av01|vorbis|opus|theora|flac|mp3|mp4v|alac|pcm|1)(\..*)?$/i;
  function canPlayType(type) {
    if (!AVAILABLE) return "";
    const parts = String(type).toLowerCase().split(";").map((x) => x.trim());
    if (!CONTAINERS.test(parts[0]) && parts[0] !== "application/ogg") return "";
    const codecs = parts.slice(1).find((p) => p.startsWith("codecs="));
    if (!codecs) return "maybe";
    const list = codecs.slice(7).replace(/^["']|["']$/g, "").split(",").map((c) => c.trim()).filter(Boolean);
    return list.length && list.every((c) => CODECS.test(c)) ? "probably" : "";
  }

  // The resource to load: the src attribute, else the first <source> child that could play.
  function selectSource(el) {
    if (el.hasAttribute("src")) return { url: resolve(el.getAttribute("src")), fromAttr: true };
    let any = false;
    for (const c of el.children) {
      if (c.localName !== "source") continue;
      any = true;
      if (!c.hasAttribute("src")) continue;
      const type = c.getAttribute("type");
      if (type && !canPlayType(type)) continue;
      const media = c.getAttribute("media");
      if (media && g.matchMedia && !g.matchMedia(media).matches) continue;
      return { url: resolve(c.getAttribute("src")), source: c };
    }
    return any ? { url: "", none: true } : null;
  }

  // ---- the load algorithm
  function rejectPlays(s, name, msg) {
    const list = s.waitingPlays.splice(0);
    for (const p of list) p.reject(new DOMException(msg, name));
  }
  function resolvePlays(s) {
    const list = s.waitingPlays.splice(0);
    if (list.length) queue(() => list.forEach((p) => p.resolve()));
  }
  // "Notify about playing": one task fires playing and resolves the waiting play() promises
  // (as the spec does), so no timeupdate can land between the two. The first timeupdate is
  // due a full interval after playback starts.
  function notifyPlaying(el, now_) {
    const s = S(el), gen = s.gen;
    const list = s.waitingPlays.splice(0);
    s.lastUpdate = now();
    const run = () => { if (S(el).gen === gen) fire(el, "playing"); list.forEach((p) => p.resolve()); };
    if (now_) run(); else queue(run);
  }
  function load(el, fromPlay) {
    const s = S(el);
    if (s.networkState === NETWORK_LOADING || s.networkState === NETWORK_IDLE) queueFire(el, "abort");
    if (s.mse) { s.mse.__detach(); s.mse = null; }
    if (s.networkState !== NETWORK_EMPTY) {
      queueFire(el, "emptied");
      __mdClose(el.__id);
      if (s.poster) __mdPoster(el.__id, s.poster, location.href);
      s.readyState = HAVE_NOTHING;
      if (!s.paused) { s.paused = true; rejectPlays(s, "AbortError", "The play() request was interrupted by a new load request."); }
      s.seeking = false;
      if (s.time !== 0) { s.time = 0; queueFire(el, "timeupdate"); }
      s.duration = NaN;
      s.vw = s.vh = 0;
    }
    s.gen++;
    s.error = null;
    s.ended = false;
    s.showPoster = true;
    s.autoplayed = false;
    s.pendingSeek = null;
    s.played = [];
    s.rate = s.defaultRate;
    const pick = selectSource(el);
    if (!pick) { s.networkState = NETWORK_EMPTY; render(el); return; }
    s.networkState = NETWORK_LOADING;
    queueFire(el, "loadstart");
    if (!pick.url) { fail(el, 4, pick.fromAttr ? "Empty or bad src" : "No playable <source>", pick); return; }
    s.currentSrc = pick.url;
    if (!AVAILABLE) { fail(el, 4, "Ironswift needs ffmpeg to play audio and video", pick); return; }
    // A MediaSource's URL: the page feeds the media (mse.js).
    const ms = internals.attachSource && internals.attachSource(el, pick.url);
    if (ms) { s.mse = ms; s.deferred = false; s.source = null; render(el); return; }
    const preload = (el.getAttribute("preload") || "").toLowerCase();
    if (preload === "none" && !el.hasAttribute("autoplay") && !fromPlay) {
      s.deferred = true;
      s.networkState = NETWORK_IDLE;
      queueFire(el, "suspend");
      render(el);
      return;
    }
    s.deferred = false;
    s.source = pick.source || null;
    __mdOpen(el.__id, pick.url, location.href);
    schedule(el, 10);
  }
  function fail(el, code, msg, pick) {
    const s = S(el);
    s.error = new MediaError(code, msg);
    s.networkState = code === 4 && pick && pick.none ? NETWORK_NO_SOURCE : NETWORK_IDLE;
    if (code === 4 && pick && pick.none) s.networkState = NETWORK_NO_SOURCE;
    rejectPlays(s, "NotSupportedError", "The element has no supported sources.");
    const src = pick && pick.source;
    queue(() => { if (src) src.dispatchEvent(new Event("error")); fire(el, "error"); });
    if (!s.paused) { s.paused = true; }
    render(el);
  }

  // ---- the playback clock
  function clock(s) {
    if (s.paused || s.seeking || s.readyState < HAVE_FUTURE_DATA || __mdFrozen()) return s.time;
    let t = s.base + ((now() - s.anchor) / 1000) * s.rate;
    if (isFinite(s.duration)) t = Math.min(t, s.duration);
    return Math.max(0, t);
  }
  function anchor(s) { s.time = clock(s); s.base = s.time; s.anchor = now(); }
  function startOutput(el) {
    const s = S(el);
    s.base = s.time;
    s.anchor = now();
    __mdVolume(el.__id, s.muted ? 0 : s.volume);
    __mdStart(el.__id, s.time, s.rate);
  }

  function schedule(el, ms) {
    const s = S(el);
    if (s.timer != null) return;
    s.timer = setTimeout(() => { s.timer = null; tick(el); }, ms);
  }

  // Everything that happens over time: the download finishing, frames, timeupdate, the end.
  function tick(el) {
    const s = S(el);
    const st = __mdStatus(el.__id);
    const [state, dur, vw, vh, code, msg, busy] = st;
    if (state === 3 && !s.error) { fail(el, code || 4, msg, s.source ? { source: s.source } : null); return; }
    if (state >= 1 && s.readyState < HAVE_METADATA) {
      s.duration = s.mse ? (isNaN(s.mse.duration) ? Infinity : s.mse.duration) : isFinite(dur) ? dur : Infinity;
      s.vw = vw; s.vh = vh;
      s.readyState = HAVE_METADATA;
      fire(el, "durationchange");
      if (el.localName === "video" && vw) fire(el, "resize");
      fire(el, "loadedmetadata");
      if (s.pendingSeek != null) { const t = s.pendingSeek; s.pendingSeek = null; seek(el, t); }
      else if (startFragment(el) != null) seek(el, startFragment(el));
    }
    if (state >= 2 && s.readyState < HAVE_CURRENT_DATA && !s.seeking && s.mse && !s.mse.__ready(s.time)) {
      // A MediaSource with its first frame but not enough after it yet.
      s.readyState = HAVE_CURRENT_DATA;
      fire(el, "loadeddata");
    }
    if (s.mse && s.readyState === HAVE_CURRENT_DATA && !s.seeking && s.mse.__ready(s.time)) {
      // Enough has been appended to play (again) from here.
      s.readyState = HAVE_ENOUGH_DATA;
      fire(el, "canplay");
      if (!s.paused) { startOutput(el); fire(el, "playing"); resolvePlays(s); }
      fire(el, "canplaythrough");
    }
    if (state >= 2 && s.readyState < HAVE_CURRENT_DATA && !s.seeking) {
      // The whole file is here, so there's enough data to play straight through.
      s.readyState = HAVE_ENOUGH_DATA;
      s.networkState = NETWORK_IDLE;
      fire(el, "loadeddata");
      fire(el, "progress");
      fire(el, "canplay");
      if (!s.paused) { startOutput(el); notifyPlaying(el, true); }
      fire(el, "canplaythrough");
      fire(el, "suspend");
      if (s.paused && !s.autoplayed && el.hasAttribute("autoplay") && autoplayAllowed(el)) {
        s.autoplayed = true;
        play.call(el).catch(() => {});
      }
    }
    if (s.seeking) {
      if (__mdAdvance(el.__id, s.time)) {
        s.seeking = false;
        fire(el, "timeupdate");
        fire(el, "seeked");
        if (!s.paused && s.readyState >= HAVE_FUTURE_DATA) startOutput(el);
      }
    } else if (!s.paused && s.readyState >= HAVE_FUTURE_DATA) {
      if (s.wasConnected && !el.isConnected) { pause.call(el); return; }
      const t = clock(s);
      __mdAdvance(el.__id, t);
      if (s.mse && t < s.duration && !s.mse.__ready(t)) {
        // Played up to the end of what's been appended: wait for more.
        anchor(s);
        s.readyState = HAVE_CURRENT_DATA;
        __mdStop(el.__id);
        fire(el, "timeupdate");
        fire(el, "waiting");
      } else if (t >= s.duration) { reachedEnd(el); }
      else if (now() - s.lastUpdate >= 250) { s.lastUpdate = now(); fire(el, "timeupdate"); }
    }
    if (el.isConnected) s.wasConnected = true;
    render(el);
    const playing = !s.paused && !s.seeking && s.readyState >= HAVE_FUTURE_DATA;
    if (busy || s.seeking || s.readyState < HAVE_CURRENT_DATA && s.networkState === NETWORK_LOADING) schedule(el, 15);
    else if (playing && !__mdFrozen()) schedule(el, 1000 / 30);
    else if (s.mse && s.readyState < HAVE_FUTURE_DATA && s.readyState >= HAVE_METADATA) schedule(el, 100);
  }

  function reachedEnd(el) {
    const s = S(el);
    if (el.hasAttribute("loop")) {
      s.played.push([0, s.duration]);
      seek(el, 0);
      return;
    }
    if (!s.paused) s.played.push([s.base, s.duration]);
    s.time = s.duration;
    s.ended = true;
    __mdStop(el.__id);
    fire(el, "timeupdate");
    if (!s.paused) { s.paused = true; fire(el, "pause"); }
    fire(el, "ended");
  }

  // A #t=10 or #t=10,20 media fragment in the URL: where playback starts.
  function startFragment(el) {
    const m = /#.*\bt=(?:npt:)?([\d.]+)/.exec(S(el).currentSrc);
    return m ? parseFloat(m[1]) : null;
  }

  // Sound that starts by itself needs to be muted (as in other browsers).
  function autoplayAllowed(el) { return S(el).muted; }

  function seek(el, t) {
    const s = S(el);
    t = Number(t);
    if (!isFinite(t)) throw new TypeError("The provided double value is non-finite.");
    if (s.readyState === HAVE_NOTHING) { s.pendingSeek = t; return; }
    if (isFinite(s.duration)) t = Math.min(t, s.duration);
    t = Math.max(0, t);
    if (!s.paused) s.played.push([s.base, clock(s)]);
    s.time = t;
    s.base = t;
    s.anchor = now();
    s.ended = false;
    s.showPoster = false;
    s.seeking = true;
    fire(el, "seeking");
    __mdSeek(el.__id, t);
    schedule(el, 0);
  }

  // ---- the methods
  function play() {
    const el = this, s = S(el);
    if (s.error && s.error.code === 4) return Promise.reject(new DOMException("The element has no supported sources.", "NotSupportedError"));
    if (s.networkState === NETWORK_EMPTY || s.deferred) load(el, true);
    if (s.ended || (isFinite(s.duration) && s.time >= s.duration && !s.seeking)) {
      if (s.readyState >= HAVE_METADATA) seek(el, 0); else s.time = 0;
      s.ended = false;
    }
    const p = new Promise((resolveP, rejectP) => s.waitingPlays.push({ resolve: resolveP, reject: rejectP }));
    if (s.paused) {
      s.paused = false;
      s.showPoster = false;
      queueFire(el, "play");
      if (s.readyState < HAVE_FUTURE_DATA) queueFire(el, "waiting");
      else {
        if (!s.seeking) startOutput(el);
        notifyPlaying(el);
      }
    } else if (s.readyState >= HAVE_FUTURE_DATA) resolvePlays(s);
    render(el);
    schedule(el, 0);
    return p;
  }
  function pause() {
    const el = this, s = S(el);
    if (s.networkState === NETWORK_EMPTY) load(el);
    if (!s.paused) {
      s.played.push([s.base, clock(s)]);
      anchor(s);
      s.paused = true;
      __mdStop(el.__id);
      queueFire(el, "timeupdate");
      queueFire(el, "pause");
      rejectPlays(s, "AbortError", "The play() request was interrupted by a call to pause().");
    }
    render(el);
  }

  // Draw the element's box (media.rs keeps the picture until something it shows changes).
  function render(el) {
    const s = S(el);
    const video = el.localName === "video";
    const controls = el.hasAttribute("controls");
    const flags = (controls ? F_CONTROLS : 0) | (s.paused ? F_PAUSED : 0) | (s.muted ? F_MUTED : 0) |
      (s.showPoster ? F_SHOW_POSTER : 0) | (video ? 0 : F_AUDIO) | (s.error ? F_ERROR : 0);
    __mdRender(el.__id, flags, clock(s), s.duration);
  }

  // ---- the built-in controls: play/pause, seek along the track, mute.
  function controlsClick(e) {
    const el = e.target;
    if (!isMedia(el) || !el.hasAttribute("controls") || e.defaultPrevented) return;
    const r = el.getBoundingClientRect();
    const x = e.clientX - r.left, y = e.clientY - r.top;
    const audio = el.localName === "audio";
    const top = audio ? Math.max(0, (r.height - BAR_H) / 2) : Math.max(0, r.height - BAR_H);
    const s = S(el);
    if (y >= top && y <= top + BAR_H) {
      const timeW = r.width >= 200 ? 96 : 4;
      const x0 = 32 + timeW, x1 = Math.max(x0, r.width - 40), muteX = Math.max(0, r.width - 32);
      if (x < 32) { el.paused ? el.play().catch(() => {}) : el.pause(); }
      else if (x >= muteX) { el.muted = !el.muted; }
      else if (x >= x0 - 6 && x <= x1 + 6 && isFinite(s.duration) && s.duration > 0) {
        seek(el, Math.min(1, Math.max(0, (x - x0) / Math.max(1, x1 - x0))) * s.duration);
      }
    } else if (!audio) {
      el.paused ? el.play().catch(() => {}) : el.pause();
    }
  }
  g.addEventListener("click", controlsClick);

  // ---- members on the shared element prototype (only <video>/<audio> see them)
  const proto = g.Element.prototype;
  function mediaProp(name, get, set, onlyTag) {
    const prev = Object.getOwnPropertyDescriptor(proto, name) || Object.getOwnPropertyDescriptor(g.HTMLElement.prototype, name);
    const mine = (el) => isMedia(el) && (!onlyTag || el.localName === onlyTag);
    Object.defineProperty(proto, name, {
      configurable: true,
      get() {
        if (mine(this)) return get.call(this);
        if (prev && prev.get) return prev.get.call(this);
        return prev ? prev.value : undefined;
      },
      set(v) {
        if (mine(this)) {
          if (set) set.call(this, v);
          return;
        }
        if (prev && prev.set) return prev.set.call(this, v);
        Object.defineProperty(this, name, { value: v, writable: true, configurable: true, enumerable: true });
      },
    });
  }
  const method = (name, fn, onlyTag) => mediaProp(name, function () { return fn; }, null, onlyTag);
  const boolAttr = (name, attr) => mediaProp(name, function () { return this.hasAttribute(attr); }, function (v) { v ? this.setAttribute(attr, "") : this.removeAttribute(attr); });
  const strAttr = (name, attr, onlyTag) => mediaProp(name, function () { return this.getAttribute(attr) || ""; }, function (v) { this.setAttribute(attr, String(v)); }, onlyTag);
  const urlAttr = (name, attr, onlyTag) => mediaProp(name, function () { return this.hasAttribute(attr) ? resolve(this.getAttribute(attr)) : ""; }, function (v) { this.setAttribute(attr, String(v)); }, onlyTag);

  for (const [k, v] of Object.entries(CONSTS)) mediaProp(k, () => v);
  method("play", play);
  method("pause", pause);
  method("load", function () { load(this); });
  method("canPlayType", canPlayType);
  method("fastSeek", function (t) { seek(this, t); });
  method("addTextTrack", function () { throw new DOMException("Text tracks aren't supported yet", "NotSupportedError"); });
  method("captureStream", function () { throw new DOMException("captureStream isn't supported", "NotSupportedError"); });
  method("setSinkId", function (id) { S(this).sinkId = String(id); return Promise.resolve(); });
  method("setMediaKeys", function () { return Promise.reject(new DOMException("Encrypted media isn't supported", "NotSupportedError")); });
  method("requestPictureInPicture", function () { return Promise.reject(new DOMException("Picture-in-picture isn't supported", "NotSupportedError")); }, "video");
  method("getVideoPlaybackQuality", function () { return { creationTime: now(), totalVideoFrames: 0, droppedVideoFrames: 0, corruptedVideoFrames: 0 }; }, "video");
  urlAttr("src", "src");
  urlAttr("poster", "poster", "video");
  strAttr("crossOrigin", "crossorigin");
  boolAttr("autoplay", "autoplay");
  boolAttr("loop", "loop");
  boolAttr("controls", "controls");
  boolAttr("defaultMuted", "muted");
  boolAttr("playsInline", "playsinline");
  mediaProp("preload", function () {
    const p = (this.getAttribute("preload") || "").toLowerCase();
    return p === "none" || p === "metadata" || p === "auto" ? p : p === "" && this.hasAttribute("preload") ? "auto" : "metadata";
  }, function (v) { this.setAttribute("preload", String(v)); });
  mediaProp("currentSrc", function () { return S(this).currentSrc; });
  mediaProp("networkState", function () { return S(this).networkState; });
  mediaProp("readyState", function () { return S(this).readyState; });
  mediaProp("error", function () { return S(this).error; });
  mediaProp("paused", function () { return S(this).paused; });
  mediaProp("ended", function () { const s = S(this); return s.ended && !this.hasAttribute("loop"); });
  mediaProp("seeking", function () { return S(this).seeking; });
  mediaProp("duration", function () { return S(this).duration; });
  mediaProp("videoWidth", function () { return S(this).vw; }, null, "video");
  mediaProp("videoHeight", function () { return S(this).vh; }, null, "video");
  mediaProp("currentTime", function () { return clock(S(this)); }, function (v) { seek(this, v); });
  mediaProp("srcObject", function () { return null; }, function (v) { if (v != null) throw new DOMException("MediaStream sources aren't supported", "NotSupportedError"); });
  mediaProp("volume", function () { return S(this).volume; }, function (v) {
    v = Number(v);
    if (!isFinite(v)) throw new TypeError("The provided double value is non-finite.");
    if (v < 0 || v > 1) throw new DOMException("The volume provided (" + v + ") is outside the range [0, 1].", "IndexSizeError");
    const s = S(this);
    if (s.volume === v) return;
    s.volume = v;
    __mdVolume(this.__id, s.muted ? 0 : v);
    queueFire(this, "volumechange");
  });
  mediaProp("muted", function () { return S(this).muted; }, function (v) {
    const s = S(this);
    v = !!v;
    if (s.muted === v) return;
    s.muted = v;
    __mdVolume(this.__id, v ? 0 : s.volume);
    queueFire(this, "volumechange");
    render(this);
  });
  mediaProp("playbackRate", function () { return S(this).rate; }, function (v) {
    v = Number(v);
    if (!isFinite(v)) throw new TypeError("The provided double value is non-finite.");
    if (v < 0 || v > 16) throw new DOMException("The provided playback rate (" + v + ") is not in the supported playback range.", "NotSupportedError");
    const s = S(this);
    if (s.rate === v) return;
    anchor(s);
    s.rate = v;
    if (!s.paused && !s.seeking && s.readyState >= HAVE_FUTURE_DATA) startOutput(this);
    queueFire(this, "ratechange");
  });
  mediaProp("defaultPlaybackRate", function () { return S(this).defaultRate; }, function (v) {
    v = Number(v);
    if (!isFinite(v)) throw new TypeError("The provided double value is non-finite.");
    S(this).defaultRate = v;
    queueFire(this, "ratechange");
  });
  mediaProp("preservesPitch", function () { return S(this).preservesPitch; }, function (v) { S(this).preservesPitch = !!v; });
  mediaProp("sinkId", function () { return S(this).sinkId; });
  mediaProp("buffered", function () { const s = S(this); if (s.mse) return new TimeRanges(s.mse.__buffered()); return new TimeRanges(s.readyState >= HAVE_CURRENT_DATA && isFinite(s.duration) ? [[0, s.duration]] : []); });
  mediaProp("seekable", function () { const s = S(this); if (s.mse) return new TimeRanges(s.readyState >= HAVE_METADATA ? s.mse.__seekable() : []); return new TimeRanges(s.readyState >= HAVE_METADATA && isFinite(s.duration) ? [[0, s.duration]] : []); });
  mediaProp("played", function () {
    const s = S(this);
    const r = s.played.concat(s.paused ? [] : [[s.base, clock(s)]]).filter((x) => x[1] > x[0]).sort((a, b) => a[0] - b[0]);
    const out = [];
    for (const x of r) { const last = out[out.length - 1]; if (last && x[0] <= last[1]) last[1] = Math.max(last[1], x[1]); else out.push(x.slice()); }
    return new TimeRanges(out);
  });
  const emptyTracks = () => Object.assign([], { onaddtrack: null, onremovetrack: null, onchange: null, getTrackById: () => null, addEventListener() {}, removeEventListener() {} });
  mediaProp("textTracks", function () { return S(this).textTracks || (S(this).textTracks = emptyTracks()); });
  mediaProp("audioTracks", function () { return S(this).audioTracks || (S(this).audioTracks = emptyTracks()); });
  mediaProp("videoTracks", function () { return S(this).videoTracks || (S(this).videoTracks = emptyTracks()); }, null, "video");
  mediaProp("mediaKeys", function () { return null; });

  // new Audio(src)
  g.Audio = function Audio(src) {
    if (!new.target) throw new TypeError("Failed to construct 'Audio': Please use the 'new' operator");
    const a = document.createElement("audio");
    a.setAttribute("preload", "auto");
    if (src !== undefined) a.setAttribute("src", String(src));
    return a;
  };
  if (g.HTMLAudioElement) g.Audio.prototype = g.HTMLAudioElement.prototype;

  // ---- noticing media elements: parsed ones, ones scripts add, and attribute changes
  function setPoster(el) {
    const s = S(el);
    const url = el.localName === "video" && el.hasAttribute("poster") ? resolve(el.getAttribute("poster")) : "";
    if (url === (s.poster || "")) return;
    s.poster = url || null;
    __mdPoster(el.__id, url, location.href);
    if (url) schedule(el, 10);
  }
  function discover(el) {
    const s = S(el);
    setPoster(el);
    if (s.networkState === NETWORK_EMPTY && selectSource(el)) load(el);
    else render(el);
    if (el.hasAttribute("controls") || s.poster) schedule(el, 10);
  }
  function scan(root) {
    if (!root || !root.querySelectorAll) return;
    if (isMedia(root)) discover(root);
    for (const el of root.querySelectorAll("video, audio")) discover(el);
  }
  function watch() {
    if (!g.MutationObserver || !document.documentElement) return;
    try {
      new MutationObserver((records) => {
        for (const r of records) {
          if (r.type === "childList") {
            for (const n of r.addedNodes) {
              if (n.nodeType !== 1) continue;
              scan(n);
              if (n.localName === "source" && isMedia(r.target) && S(r.target).networkState === NETWORK_EMPTY) load(r.target);
            }
          } else if (r.type === "attributes" && isMedia(r.target)) {
            const el = r.target;
            if (r.attributeName === "src" && el.hasAttribute("src")) load(el);
            else if (r.attributeName === "poster") setPoster(el);
            else render(el);
          }
        }
      }).observe(document.documentElement, { childList: true, subtree: true, attributes: true, attributeFilter: ["src", "poster", "controls", "muted"] });
    } catch (e) {}
  }
  // A src set on an element that isn't in the page yet (new Audio(), createElement) loads
  // too: setAttribute on a media element starts the load algorithm.
  const setAttr = proto.setAttribute;
  proto.setAttribute = function (name, value) {
    const r = setAttr.call(this, name, value);
    if (isMedia(this) && !this.isConnected) {
      const n = String(name).toLowerCase();
      if (n === "src") load(this);
      else if (n === "poster") setPoster(this);
    }
    return r;
  };
  // For mse.js (Media Source Extensions).
  const internals = {
    available: AVAILABLE, schedule, fail: (el, code, msg) => fail(el, code, msg, null),
    setDuration(el, d) {
      const s = S(el);
      if (s.readyState < HAVE_METADATA || s.duration === d) return;
      s.duration = d;
      queueFire(el, "durationchange");
      if (clock(s) > d) seek(el, d);
    },
  };
  Object.defineProperty(g, "__mediaInternals", { value: internals, configurable: true });
  function start() { scan(document); watch(); }
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", start);
  else start();
})();
