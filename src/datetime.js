// Dates as browsers read and write them. From the New York Times (2026-11): dayjs's timezone
// plugin formats a date for "America/New_York" with Intl.DateTimeFormat#formatToParts and
// parses toLocaleString() back with new Date("11/17/2026, 5:33:56 PM"). Boa's formatter
// ignored the field options (year: "numeric"…), wrote two-digit years and "GMT+?" for zone
// names, and its Date parser only takes ISO and RFC 2822 strings, so every timestamp became
// "Invalid time value" and React's error boundary took the masthead away.
//
// 1. Intl.DateTimeFormat for English locales, computed from the time zone database Temporal
//    carries: format, formatToParts, resolvedOptions; and Date's toLocale*String through it.
//    Other languages keep the formatter they had.
// 2. new Date(string) and Date.parse: when the built-in parser gives NaN, the forms browsers
//    also accept (11/17/2026 5:33 PM, Nov 17 2026, 2026-11-17 13:05, +0000 offsets, more than
//    three fractional digits, Date#toString output, EST/PDT…).
(function () {
  "use strict";
  const g = globalThis;
  const NativeDate = g.Date;
  if (typeof NativeDate !== "function") return;

  // ------------------------------------------------------------ parsing
  const MONTHS = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
  const ZONES = { UT: 0, UTC: 0, GMT: 0, Z: 0, EST: -300, EDT: -240, CST: -360, CDT: -300, MST: -420, MDT: -360, PST: -480, PDT: -420 };
  const monthOf = (w) => { const i = MONTHS.indexOf(String(w).slice(0, 3).toLowerCase()); return i; };

  // Minutes east of UTC for a zone suffix, or undefined when there is none, or NaN when bad.
  function zoneOffset(z) {
    if (!z) return undefined;
    z = z.trim();
    if (!z) return undefined;
    const m = /^(?:(?:UTC|GMT|UT)\s*)?([+-])(\d{1,2})(?::?(\d{2}))?$/i.exec(z);
    if (m) return (m[1] === "-" ? -1 : 1) * (Number(m[2]) * 60 + Number(m[3] || 0));
    const n = ZONES[z.toUpperCase()];
    return n === undefined ? NaN : n;
  }

  function build(y, mo, d, h, mi, s, ms, off) {
    if ([y, mo, d, h, mi, s, ms].some((v) => !Number.isFinite(v))) return NaN;
    if (mo < 0 || mo > 11 || d < 1 || d > 31 || h > 24 || mi > 59 || s > 60) return NaN;
    if (off === undefined) return new NativeDate(y, mo, d, h, mi, s, ms).getTime();
    if (Number.isNaN(off)) return NaN;
    return NativeDate.UTC(y, mo, d, h, mi, s, ms) - off * 60000;
  }

  // "5:33", "5:33:56.123", "5:33 PM", "17:33:56 GMT-0600 (Central Standard Time)".
  function time(rest) {
    rest = rest.replace(/\([^)]*\)\s*$/, "").trim();
    if (!rest) return [0, 0, 0, 0, undefined];
    const m = /^(?:at\s+)?(\d{1,2})(?::(\d{2}))?(?::(\d{2})(?:[.,](\d+))?)?\s*([ap]\.?m\.?)?\s*(.*)$/i.exec(rest);
    if (!m || (m[2] === undefined && !m[5])) {
      const z = zoneOffset(rest);
      return z === undefined || Number.isNaN(z) ? null : [0, 0, 0, 0, z];
    }
    let h = Number(m[1]);
    if (m[5]) {
      if (h < 1 || h > 12) return null;
      h = (h % 12) + (/^p/i.test(m[5]) ? 12 : 0);
    }
    const ms = m[4] ? Number((m[4] + "00").slice(0, 3)) : 0;
    const z = zoneOffset(m[6]);
    if (Number.isNaN(z)) return null;
    return [h, Number(m[2] || 0), Number(m[3] || 0), ms, z];
  }

  function fallbackParse(str) {
    let s = String(str).trim().replace(/\s+/g, " ");
    let m;
    // ISO-like: 2026-11-17[ T]13:05[:56[.123456]][Z|+hh[:mm]|+hhmm]
    if ((m = /^([+-]\d{6}|\d{4})-(\d{1,2})-(\d{1,2})(?:[T ](\d{1,2}):(\d{2})(?::(\d{2})(?:[.,](\d+))?)?)?\s*(Z|[+-]\d{2}(?::?\d{2})?|UTC|GMT)?$/i.exec(s))) {
      const ms = m[7] ? Number((m[7] + "00").slice(0, 3)) : 0;
      // Date-only forms are UTC; date-time forms without a zone are local (as in ES).
      let off = m[8] ? zoneOffset(m[8]) : m[4] === undefined ? 0 : undefined;
      return build(Number(m[1]), Number(m[2]) - 1, Number(m[3]), Number(m[4] || 0), Number(m[5] || 0), Number(m[6] || 0), ms, off);
    }
    // 2026/11/17 [time]
    if ((m = /^(\d{4})[/.](\d{1,2})[/.](\d{1,2}),?\s*(.*)$/.exec(s))) {
      const t = time(m[4]);
      return t ? build(Number(m[1]), Number(m[2]) - 1, Number(m[3]), t[0], t[1], t[2], t[3], t[4]) : NaN;
    }
    // 11/17/2026 [time] (US order, as browsers read it)
    if ((m = /^(\d{1,2})[/-](\d{1,2})[/-](\d{4}|\d{2})(?!\d),?\s*(.*)$/.exec(s))) {
      let y = Number(m[3]);
      if (m[3].length === 2) y += y < 50 ? 2000 : 1900;
      const t = time(m[4]);
      return t ? build(y, Number(m[1]) - 1, Number(m[2]), t[0], t[1], t[2], t[3], t[4]) : NaN;
    }
    // Strip a leading weekday ("Tue", "Tuesday,").
    s = s.replace(/^(mon|tue|wed|thu|fri|sat|sun)[a-z]*\.?,?\s+/i, "");
    // Nov 17 2026, November 17, 2026 [time]
    if ((m = /^([a-z]{3,})\.?\s+(\d{1,2})(?:st|nd|rd|th)?,?\s+(\d{4})(?:,?\s+(.*))?$/i.exec(s)) && monthOf(m[1]) >= 0) {
      const t = time(m[4] || "");
      return t ? build(Number(m[3]), monthOf(m[1]), Number(m[2]), t[0], t[1], t[2], t[3], t[4]) : NaN;
    }
    // 17 Nov 2026, 17 November, 2026 [time]
    if ((m = /^(\d{1,2})\s+([a-z]{3,})\.?,?\s+(\d{4})(?:,?\s+(.*))?$/i.exec(s)) && monthOf(m[2]) >= 0) {
      const t = time(m[4] || "");
      return t ? build(Number(m[3]), monthOf(m[2]), Number(m[1]), t[0], t[1], t[2], t[3], t[4]) : NaN;
    }
    // Nov 2026, November 2026
    if ((m = /^([a-z]{3,})\.?\s+(\d{4})$/i.exec(s)) && monthOf(m[1]) >= 0) return build(Number(m[2]), monthOf(m[1]), 1, 0, 0, 0, 0, undefined);
    return NaN;
  }

  const nativeParse = NativeDate.parse;
  function parse(str) {
    const t = nativeParse.call(NativeDate, str);
    if (!Number.isNaN(t)) return t;
    try { return fallbackParse(str); } catch (e) { return NaN; }
  }

  function Date(...args) {
    if (!new.target) return NativeDate();
    if (args.length === 1 && typeof args[0] === "string") return Reflect.construct(NativeDate, [parse(args[0])], new.target);
    return Reflect.construct(NativeDate, args, new.target);
  }
  Object.defineProperty(Date, "length", { value: 7 });
  Date.prototype = NativeDate.prototype;
  Object.defineProperty(NativeDate.prototype, "constructor", { value: Date, writable: true, configurable: true, enumerable: false });
  Date.UTC = NativeDate.UTC;
  Date.now = NativeDate.now;
  Date.parse = function parse_(s) { return parse(String(s)); };
  Object.defineProperty(Date.parse, "name", { value: "parse" });
  Object.setPrototypeOf(Date, Object.getPrototypeOf(NativeDate));
  g.Date = Date;

  // ------------------------------------------------------------ formatting (English)
  const Intl = g.Intl;
  const Temporal = g.Temporal;
  if (!Intl || !Intl.DateTimeFormat || !Temporal) return;
  const PrevDTF = Intl.DateTimeFormat;

  const MONTH_NAMES = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
  const DAY_NAMES = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
  const ZONE_NAMES = {
    "America/New_York": ["E", "Eastern"], "America/Detroit": ["E", "Eastern"], "America/Toronto": ["E", "Eastern"],
    "America/Chicago": ["C", "Central"], "America/Winnipeg": ["C", "Central"], "America/Mexico_City": ["C", "Central"],
    "America/Denver": ["M", "Mountain"], "America/Phoenix": ["M", "Mountain"], "America/Edmonton": ["M", "Mountain"],
    "America/Los_Angeles": ["P", "Pacific"], "America/Vancouver": ["P", "Pacific"],
    "America/Anchorage": ["AK", "Alaska"], "Pacific/Honolulu": ["H", "Hawaii-Aleutian"],
  };
  const isEnglish = (loc) => /^en(-|$)/i.test(loc);

  function localZone() {
    try { const z = new PrevDTF().resolvedOptions().timeZone; if (z) return z; } catch (e) { /* none */ }
    return "UTC";
  }
  function canonicalZone(z) {
    if (z === undefined) return undefined;
    const s = String(z);
    if (/^(utc|gmt|etc\/utc|etc\/gmt|z)$/i.test(s)) return "UTC";
    try { return Temporal.Now.zonedDateTimeISO(s).timeZoneId; } catch (e) { throw new RangeError("Invalid time zone specified: " + s); }
  }

  // The wall-clock fields of `ms` in a zone (undefined: the Date's own local time).
  function fields(ms, zone) {
    if (zone === undefined) {
      const d = new NativeDate(ms);
      return { y: d.getFullYear(), mo: d.getMonth() + 1, d: d.getDate(), h: d.getHours(), mi: d.getMinutes(), s: d.getSeconds(), ms: d.getMilliseconds(), wd: d.getDay(), off: -d.getTimezoneOffset() };
    }
    const z = Temporal.Instant.fromEpochMilliseconds(ms).toZonedDateTimeISO(zone);
    return { y: z.year, mo: z.month, d: z.day, h: z.hour, mi: z.minute, s: z.second, ms: z.millisecond, wd: z.dayOfWeek % 7, off: Math.round(z.offsetNanoseconds / 6e10) };
  }

  const pad2 = (n) => String(n).padStart(2, "0");
  function offsetText(off, long) {
    if (off === 0) return "GMT";
    const sign = off < 0 ? "-" : "+", a = Math.abs(off), h = Math.floor(a / 60), m = a % 60;
    return long ? `GMT${sign}${pad2(h)}:${pad2(m)}` : `GMT${sign}${h}${m ? ":" + pad2(m) : ""}`;
  }
  function zoneText(style, zoneId, f) {
    if (zoneId === "UTC") return style === "long" ? "Coordinated Universal Time" : style === "longOffset" ? "GMT" : "UTC";
    if (style === "shortOffset") return offsetText(f.off, false);
    if (style === "longOffset") return offsetText(f.off, true);
    const n = ZONE_NAMES[zoneId];
    if (n) {
      // Standard time is the smaller of the January and July offsets.
      const std = Math.min(fields(NativeDate.UTC(f.y, 0, 1), zoneId).off, fields(NativeDate.UTC(f.y, 6, 1), zoneId).off);
      const dst = f.off !== std;
      if (style === "long" || style === "longGeneric") return `${n[1]} ${style === "longGeneric" ? "Time" : dst ? "Daylight Time" : "Standard Time"}`;
      return style === "shortGeneric" ? n[0] + "T" : n[0] + (dst ? "DT" : "ST");
    }
    return offsetText(f.off, style === "long");
  }

  const DATE_STYLES = {
    full: { weekday: "long", year: "numeric", month: "long", day: "numeric" },
    long: { year: "numeric", month: "long", day: "numeric" },
    medium: { year: "numeric", month: "short", day: "numeric" },
    short: { year: "2-digit", month: "numeric", day: "numeric" },
  };
  const TIME_STYLES = {
    full: { hour: "numeric", minute: "2-digit", second: "2-digit", timeZoneName: "long" },
    long: { hour: "numeric", minute: "2-digit", second: "2-digit", timeZoneName: "short" },
    medium: { hour: "numeric", minute: "2-digit", second: "2-digit" },
    short: { hour: "numeric", minute: "2-digit" },
  };
  const FIELD_NAMES = ["weekday", "era", "year", "month", "day", "dayPeriod", "hour", "minute", "second", "fractionalSecondDigits", "timeZoneName"];

  // The fields a formatter shows: `required`/`defaults` as in ECMA-402's ToDateTimeOptions.
  function resolve(options, required, defaults) {
    const o = options === undefined ? {} : Object(options);
    const r = {};
    for (const k of FIELD_NAMES) if (o[k] !== undefined) r[k] = String(o[k]);
    if (o.fractionalSecondDigits !== undefined) r.fractionalSecondDigits = Number(o.fractionalSecondDigits);
    const ds = o.dateStyle !== undefined ? String(o.dateStyle) : undefined;
    const ts = o.timeStyle !== undefined ? String(o.timeStyle) : undefined;
    if ((ds || ts) && FIELD_NAMES.some((k) => r[k] !== undefined && k !== "timeZoneName")) throw new TypeError("Can't set option " + FIELD_NAMES.find((k) => r[k] !== undefined) + " when dateStyle or timeStyle is used");
    if (ds) { if (!DATE_STYLES[ds]) throw new RangeError("Value " + ds + " out of range for Intl.DateTimeFormat options property dateStyle"); Object.assign(r, DATE_STYLES[ds]); }
    if (ts) { if (!TIME_STYLES[ts]) throw new RangeError("Value " + ts + " out of range for Intl.DateTimeFormat options property timeStyle"); Object.assign(r, TIME_STYLES[ts]); }
    if (!ds && !ts) {
      let need = true;
      if (required !== "time" && ["weekday", "year", "month", "day"].some((k) => r[k] !== undefined)) need = false;
      if (required !== "date" && ["dayPeriod", "hour", "minute", "second", "fractionalSecondDigits"].some((k) => r[k] !== undefined)) need = false;
      if (need && (defaults === "date" || defaults === "all")) Object.assign(r, { year: "numeric", month: "numeric", day: "numeric" });
      if (need && (defaults === "time" || defaults === "all")) Object.assign(r, { hour: "numeric", minute: "numeric", second: "numeric" });
    }
    let h12 = true;
    if (o.hour12 !== undefined) h12 = !!o.hour12;
    else if (o.hourCycle !== undefined) h12 = /^h1[12]$/.test(String(o.hourCycle));
    r.hour12 = r.hour !== undefined ? h12 : undefined;
    r.hourCycle = r.hour !== undefined ? (h12 ? "h12" : "h23") : undefined;
    r.dateStyle = ds; r.timeStyle = ts;
    return r;
  }

  // The parts for `ms` with options `r`, in en-US order.
  function parts(r, zoneId, zoneArg, ms) {
    const f = fields(ms, zoneArg);
    const P = [];
    const lit = (v) => { if (!v) return; const last = P[P.length - 1]; if (last && last.type === "literal") last.value += v; else P.push({ type: "literal", value: v }); };
    const add = (type, value) => P.push({ type, value: String(value) });
    const num = (v, style) => (style === "2-digit" ? pad2(v % 100) : String(v));
    const textMonth = r.month === "long" || r.month === "short" || r.month === "narrow";
    const monthText = r.month === "long" ? MONTH_NAMES[f.mo - 1] : r.month === "short" ? MONTH_NAMES[f.mo - 1].slice(0, 3) : r.month === "narrow" ? MONTH_NAMES[f.mo - 1][0] : num(f.mo, r.month);
    const wdText = r.weekday === "long" ? DAY_NAMES[f.wd] : r.weekday === "short" ? DAY_NAMES[f.wd].slice(0, 3) : r.weekday === "narrow" ? DAY_NAMES[f.wd][0] : "";
    const yearText = r.year === "2-digit" ? pad2(f.y % 100) : String(f.y);
    const hasDate = r.year !== undefined || r.month !== undefined || r.day !== undefined || r.weekday !== undefined;
    const hasTime = r.hour !== undefined || r.minute !== undefined || r.second !== undefined;
    if (hasDate) {
      if (r.weekday) { add("weekday", wdText); if (r.year || r.month || r.day) lit(", "); }
      if (textMonth) {
        if (r.month) add("month", monthText);
        if (r.day) { if (r.month) lit(" "); add("day", num(f.d, r.day)); }
        if (r.year) { if (r.day) lit(", "); else if (r.month) lit(" "); add("year", yearText); }
      } else {
        const seq = [];
        if (r.month) seq.push(["month", monthText]);
        if (r.day) seq.push(["day", num(f.d, r.day)]);
        if (r.year) seq.push(["year", yearText]);
        seq.forEach(([t, v], i) => { if (i) lit("/"); add(t, v); });
      }
      if (r.era) { lit(" "); add("era", r.era === "long" ? (f.y > 0 ? "Anno Domini" : "Before Christ") : f.y > 0 ? "AD" : "BC"); }
    }
    if (hasTime) {
      if (hasDate) lit(r.dateStyle === "full" || r.dateStyle === "long" ? " at " : ", ");
      const h12 = r.hour12 !== false && r.hour !== undefined;
      if (r.hour !== undefined) {
        const h = h12 ? (f.h % 12 === 0 ? 12 : f.h % 12) : f.h;
        add("hour", r.hour === "2-digit" || (!h12 && (r.minute !== undefined)) ? pad2(h) : String(h));
      }
      if (r.minute !== undefined) { if (r.hour !== undefined) lit(":"); add("minute", r.hour !== undefined || r.minute === "2-digit" ? pad2(f.mi) : String(f.mi)); }
      if (r.second !== undefined) { if (r.minute !== undefined || r.hour !== undefined) lit(":"); add("second", r.minute !== undefined || r.second === "2-digit" ? pad2(f.s) : String(f.s)); }
      if (r.fractionalSecondDigits) { lit("."); add("fractionalSecond", String(f.ms).padStart(3, "0").slice(0, r.fractionalSecondDigits)); }
      if (h12) { lit(" "); add("dayPeriod", f.h < 12 ? "AM" : "PM"); }
    }
    if (r.timeZoneName) { lit(" "); add("timeZoneName", zoneText(r.timeZoneName, zoneId, f)); }
    return P;
  }

  const SLOT = Symbol("dtf");
  function DateTimeFormat(locales, options) {
    if (!new.target) return new DateTimeFormat(locales, options);
    let locale = "en-US";
    try { locale = new PrevDTF(locales).resolvedOptions().locale || locale; } catch (e) { if (e instanceof RangeError) throw e; }
    if (!isEnglish(locale)) return Reflect.construct(PrevDTF, [locales, options], new.target === DateTimeFormat ? PrevDTF : new.target);
    const o = options === undefined ? {} : Object(options);
    const zoneArg = canonicalZone(o.timeZone);
    const r = resolve(o, "any", "date");
    const self = Object.create(new.target.prototype);
    self[SLOT] = { locale, r, zoneArg, zoneId: zoneArg || localZone() };
    return self;
  }
  const P = DateTimeFormat.prototype;
  const slot = (x) => { const s = x && x[SLOT]; if (!s) throw new TypeError("Method called on incompatible receiver"); return s; };
  const timeOf = (date) => { const t = date === undefined ? NativeDate.now() : Number(date instanceof NativeDate ? date.getTime() : date); if (!Number.isFinite(t)) throw new RangeError("Invalid time value"); return t; };
  P.formatToParts = function formatToParts(date) { const s = slot(this); return parts(s.r, s.zoneId, s.zoneArg, timeOf(date)); };
  Object.defineProperty(P, "format", {
    get() { const s = slot(this); const self = this; return s.bound || (s.bound = function (date) { return P.formatToParts.call(self, date).map((p) => p.value).join(""); }); },
    configurable: true,
  });
  P.formatRange = function formatRange(a, b) { return this.format(a) + " – " + this.format(b); };
  P.formatRangeToParts = function formatRangeToParts(a, b) { return [...this.formatToParts(a), { type: "literal", value: " – " }, ...this.formatToParts(b)]; };
  P.resolvedOptions = function resolvedOptions() {
    const s = slot(this);
    const out = { locale: s.locale, calendar: "gregory", numberingSystem: "latn", timeZone: s.zoneId };
    if (s.r.hourCycle) { out.hourCycle = s.r.hourCycle; out.hour12 = s.r.hour12; }
    for (const k of FIELD_NAMES) if (s.r[k] !== undefined && !s.r.dateStyle && !s.r.timeStyle) out[k] = s.r[k];
    if (s.r.dateStyle) out.dateStyle = s.r.dateStyle;
    if (s.r.timeStyle) out.timeStyle = s.r.timeStyle;
    return out;
  };
  Object.defineProperty(P, Symbol.toStringTag, { value: "Intl.DateTimeFormat", configurable: true });
  DateTimeFormat.supportedLocalesOf = PrevDTF.supportedLocalesOf;
  Intl.DateTimeFormat = DateTimeFormat;

  // Date's locale methods through the formatter, with their own defaults.
  const viaFormatter = (required, defaults, name) => {
    const prev = NativeDate.prototype[name];
    return function (locales, options) {
      const t = this.getTime();
      if (Number.isNaN(t)) return "Invalid Date";
      let locale = "en-US";
      try { locale = new PrevDTF(locales).resolvedOptions().locale || locale; } catch (e) { if (e instanceof RangeError) throw e; }
      if (!isEnglish(locale)) return prev.call(this, locales, options);
      const o = options === undefined ? {} : Object(options);
      const zoneArg = canonicalZone(o.timeZone);
      return parts(resolve(o, required, defaults), zoneArg || localZone(), zoneArg, t).map((p) => p.value).join("");
    };
  };
  for (const [name, req, def] of [["toLocaleString", "any", "all"], ["toLocaleDateString", "date", "date"], ["toLocaleTimeString", "time", "time"]]) {
    Object.defineProperty(NativeDate.prototype, name, { value: viaFormatter(req, def, name), writable: true, configurable: true, enumerable: false });
  }
})();
