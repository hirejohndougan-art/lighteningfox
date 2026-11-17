// Fills the gaps in Boa's Intl (ECMA-402): currency, percent and unit number formats,
// formatToParts, RelativeTimeFormat, DisplayNames and supportedValuesOf. Everything else
// (dates, collation, plurals, lists, segmenting, locales) is Boa's own, backed by ICU data.
(function () {
  if (typeof Intl === "undefined") return;
  var BaseNF = Intl.NumberFormat, BaseDTF = Intl.DateTimeFormat;
  var hasOwn = function (o, k) { return Object.prototype.hasOwnProperty.call(o, k); };
  var defaultLocale = function () { return (typeof __lang === "string" && __lang) || "en-US"; };
  var pickLocale = function (locales) {
    if (locales === undefined) return defaultLocale();
    var list = Array.isArray(locales) ? locales : [locales];
    try { return Intl.getCanonicalLocales(list)[0] || defaultLocale(); } catch (e) { return defaultLocale(); }
  };
  var lang = function (loc) { return String(loc).split("-")[0].toLowerCase(); };
  var region = function (loc) { var m = /-([A-Z]{2}|\d{3})(?:-|$)/.exec(String(loc)); return m ? m[1] : ""; };

  // ------------------------------------------------------------ NumberFormat
  var SYMBOLS = { USD: "$", EUR: "€", GBP: "£", JPY: "¥", CNY: "CN¥", INR: "₹", KRW: "₩", CAD: "CA$", AUD: "A$",
    MXN: "MX$", BRL: "R$", NZD: "NZ$", HKD: "HK$", TWD: "NT$", ILS: "₪", VND: "₫", PHP: "₱", NGN: "₦", XAF: "FCFA",
    UAH: "₴", THB: "฿", TRY: "TRY", RUB: "RUB", PLN: "PLN", CHF: "CHF", SEK: "SEK", NOK: "NOK", DKK: "DKK" };
  // Where the currency is the local one, its symbol drops the country prefix.
  var HOME = { CAD: "CA", AUD: "AU", MXN: "MX", NZD: "NZ", HKD: "HK", TWD: "TW", CNY: "CN", JPY: "JP" };
  var LOCAL_SYMBOL = { "pl:PLN": "zł", "ru:RUB": "₽", "tr:TRY": "₺", "sv:SEK": "kr", "nb:NOK": "kr", "da:DKK": "kr.", "zh:CNY": "¥", "ja:JPY": "￥" };
  var DIGITS = { JPY: 0, KRW: 0, VND: 0, CLP: 0, ISK: 0, UGX: 0, XAF: 0, XOF: 0, PYG: 0, KWD: 3, BHD: 3, JOD: 3, OMR: 3, TND: 3 };
  // Languages that write the currency after the number ("1.234,50 €").
  var AFTER = { de: 1, fr: 1, es: 1, it: 1, pt: 1, sv: 1, fi: 1, nb: 1, no: 1, da: 1, pl: 1, cs: 1, sk: 1, ru: 1, uk: 1,
    hu: 1, ro: 1, bg: 1, hr: 1, sl: 1, lt: 1, lv: 1, et: 1, el: 1, vi: 1, ca: 1, is: 1, be: 1, sr: 1 };
  var PCT_SPACE = { de: 1, fr: 1, sv: 1, fi: 1, nb: 1, da: 1, cs: 1, sk: 1, ru: 1, uk: 1, pl: 0, es: 1, ca: 1, el: 1 };
  var UNITS = { kilometer: ["km", "kilometer"], meter: ["m", "meter"], centimeter: ["cm", "centimeter"], millimeter: ["mm", "millimeter"],
    mile: ["mi", "mile"], foot: ["ft", "foot", "feet"], inch: ["in", "inch", "inches"], yard: ["yd", "yard"],
    kilogram: ["kg", "kilogram"], gram: ["g", "gram"], pound: ["lb", "pound"], ounce: ["oz", "ounce"],
    liter: ["L", "liter"], milliliter: ["mL", "milliliter"], gallon: ["gal", "gallon"],
    second: ["sec", "second"], minute: ["min", "minute"], hour: ["hr", "hour"], day: ["day", "day"], week: ["wk", "week"],
    month: ["mth", "month"], year: ["yr", "year"], millisecond: ["ms", "millisecond"],
    byte: ["byte", "byte"], kilobyte: ["kB", "kilobyte"], megabyte: ["MB", "megabyte"], gigabyte: ["GB", "gigabyte"], terabyte: ["TB", "terabyte"],
    celsius: ["°C", "degree Celsius", "degrees Celsius"], fahrenheit: ["°F", "degree Fahrenheit", "degrees Fahrenheit"],
    percent: ["%", "percent", "percent"], degree: ["deg", "degree"], hectare: ["ha", "hectare"], acre: ["ac", "acre"] };
  var unitLabel = function (unit, n, display) {
    var parts = String(unit).split("-per-");
    var one = function (u, plural) {
      var e = UNITS[u] || [u, u];
      if (display === "long") return plural ? (e[2] || e[1] + "s") : e[1];
      return e[0];
    };
    var plural = Math.abs(n) !== 1;
    if (parts.length === 2) {
      if (parts[0] === "kilometer" && parts[1] === "hour" && display !== "long") return "km/h";
      if (parts[0] === "mile" && parts[1] === "hour" && display !== "long") return "mph";
      return display === "long" ? one(parts[0], plural) + " per " + one(parts[1], false) : one(parts[0], plural) + "/" + one(parts[1], false);
    }
    return one(unit, plural);
  };

  function NumberFormat(locales, options) {
    if (!(this instanceof NumberFormat)) return new NumberFormat(locales, options);
    var o = Object.assign({}, options || {});
    var style = o.style || "decimal";
    var loc = pickLocale(locales);
    var inner = Object.assign({}, o);
    delete inner.style; delete inner.currency; delete inner.currencyDisplay; delete inner.currencySign;
    delete inner.unit; delete inner.unitDisplay;
    if (style === "currency") {
      if (!o.currency) throw new TypeError("Currency code is required with currency style.");
      var d = hasOwn(DIGITS, o.currency.toUpperCase()) ? DIGITS[o.currency.toUpperCase()] : 2;
      if (inner.minimumFractionDigits === undefined && inner.maximumFractionDigits === undefined && inner.maximumSignificantDigits === undefined && inner.notation !== "compact") {
        inner.minimumFractionDigits = d; inner.maximumFractionDigits = d;
      } else if (inner.maximumFractionDigits === undefined && inner.maximumSignificantDigits === undefined) {
        inner.maximumFractionDigits = Math.max(d, inner.minimumFractionDigits || 0);
      }
    } else if (style === "percent") {
      if (inner.maximumFractionDigits === undefined && inner.maximumSignificantDigits === undefined) inner.maximumFractionDigits = Math.max(0, inner.minimumFractionDigits || 0);
    } else if (style !== "decimal" && style !== "unit") {
      throw new RangeError("Invalid style: " + style);
    }
    if (style === "unit" && !o.unit) throw new TypeError("Unit is required with unit style.");
    this._nf = new BaseNF(loc, inner);
    this._loc = this._nf.resolvedOptions().locale || loc;
    this._dec = (/\d(\D+)5/.exec(new BaseNF(loc, { useGrouping: false }).format(1.5)) || [0, "."])[1];
    this._o = o;
    this._style = style;
  }
  NumberFormat.prototype._raw = function (x) {
    var o = this._o, s = this._style, lg = lang(this._loc);
    var n = typeof x === "bigint" ? x : Number(x);
    if (s === "percent") n = n * 100;
    var neg = (n < 0 || Object.is(n, -0)) && o.signDisplay !== "never";
    var body = this._nf.format(typeof n === "bigint" ? n : Math.abs(n));
    var sign = neg && !(Number(n) === 0 && o.signDisplay !== "negative" && o.signDisplay !== "always") ? "-" : "";
    if (!neg && (o.signDisplay === "always" || (o.signDisplay === "exceptZero" && Number(n) !== 0))) sign = "+";
    if (Number.isNaN(Number(n))) sign = "";
    var parts = [];
    if (s === "currency") {
      var code = o.currency.toUpperCase();
      var disp = o.currencyDisplay || "symbol";
      var sym = code;
      if (disp === "symbol" || disp === "narrowSymbol") {
        sym = LOCAL_SYMBOL[lg + ":" + code] || (HOME[code] === region(this._loc) ? "$" : null) || SYMBOLS[code] || code;
        if (code === "JPY" && lg === "ja") sym = "￥";
        if (disp === "narrowSymbol" && /^[A-Z]{1,3}\$$/.test(sym)) sym = "$";
        if (lg === "en" && code === "USD" && region(this._loc) && region(this._loc) !== "US" && !{ "PR": 1, "VI": 1, "GU": 1 }[region(this._loc)]) sym = "US$";
      } else if (disp === "name") {
        sym = code;
      }
      var accounting = o.currencySign === "accounting" && sign === "-";
      if (accounting) sign = "";
      if (AFTER[lg] || disp === "name") {
        if (sign) parts.push(["minusSign", sign]);
        pushNumber(parts, body, this._dec);
        parts.push(["literal", " "]);
        parts.push(["currency", sym]);
      } else {
        if (accounting) parts.push(["literal", "("]);
        if (sign) parts.push([sign === "+" ? "plusSign" : "minusSign", sign]);
        parts.push(["currency", sym]);
        if (/^[A-Z]{3}$/.test(sym) || lg === "nl") parts.push(["literal", " "]);
        pushNumber(parts, body, this._dec);
        if (accounting) parts.push(["literal", ")"]);
      }
      return parts;
    }
    if (sign) parts.push([sign === "+" ? "plusSign" : "minusSign", sign]);
    pushNumber(parts, body, this._dec);
    if (s === "percent") {
      if (PCT_SPACE[lg]) parts.push(["literal", " "]);
      parts.push(["percentSign", "%"]);
    } else if (s === "unit") {
      var disp2 = o.unitDisplay || "short";
      var label = unitLabel(o.unit, Number(n), disp2);
      if (disp2 === "narrow" && /^(°C|°F|%)$/.test(label)) parts.push(["unit", label]);
      else { parts.push(["literal", label === "%" ? "" : " "]); parts.push(["unit", label]); }
    }
    return parts;
  };
  // Split a formatted number into integer, group, decimal and fraction parts.
  function pushNumber(parts, body, decChar) {
    var m = /^([^\d]*)([\d\s.,'  ٫٬]*\d)?([^\d]*)$/.exec(body);
    if (!m || !m[2]) { parts.push([/∞/.test(body) ? "infinity" : /NaN/.test(body) ? "nan" : "literal", body]); return; }
    if (m[1]) parts.push(["literal", m[1]]);
    var num = m[2];
    // The locale's decimal separator, if it's there, splits off the fraction.
    var dec = decChar ? num.lastIndexOf(decChar) : -1;
    var intPart = dec >= 0 ? num.slice(0, dec) : num;
    var re = /(\d+)|([^\d]+)/g, t;
    while ((t = re.exec(intPart))) parts.push(t[1] ? ["integer", t[1]] : ["group", t[2]]);
    if (dec >= 0) { parts.push(["decimal", num[dec]]); parts.push(["fraction", num.slice(dec + 1)]); }
    if (m[3]) parts.push([/[KMBT]|thousand|million|billion/.test(m[3]) ? "compact" : "literal", m[3]]);
  }
  Object.defineProperty(NumberFormat.prototype, "format", {
    configurable: true,
    get: function () {
      var self = this;
      if (!this._bound) this._bound = function (x) { return self._raw(x).map(function (p) { return p[1]; }).join(""); };
      return this._bound;
    }
  });
  NumberFormat.prototype.formatToParts = function (x) {
    return this._raw(x).filter(function (p) { return p[1] !== ""; }).map(function (p) { return { type: p[0], value: p[1] }; });
  };
  NumberFormat.prototype.formatRange = function (a, b) { return this.format(a) + "–" + this.format(b); };
  NumberFormat.prototype.resolvedOptions = function () {
    var r = this._nf.resolvedOptions();
    r.style = this._style;
    if (this._style === "currency") { r.currency = this._o.currency.toUpperCase(); r.currencyDisplay = this._o.currencyDisplay || "symbol"; r.currencySign = this._o.currencySign || "standard"; }
    if (this._style === "unit") { r.unit = this._o.unit; r.unitDisplay = this._o.unitDisplay || "short"; }
    return r;
  };
  NumberFormat.supportedLocalesOf = BaseNF.supportedLocalesOf;
  Object.defineProperty(NumberFormat.prototype, Symbol.toStringTag, { value: "Intl.NumberFormat", configurable: true });
  Intl.NumberFormat = NumberFormat;
  var nfCache = {};
  Number.prototype.toLocaleString = function (locales, options) {
    var key = options === undefined ? String(locales) : null;
    var nf = key !== null ? (nfCache[key] || (nfCache[key] = new NumberFormat(locales))) : new NumberFormat(locales, options);
    return nf.format(Number(this));
  };
  if (typeof BigInt !== "undefined") BigInt.prototype.toLocaleString = function (l, o) { return new NumberFormat(l, o).format(this); };

  // ------------------------------------------------------------ DateTimeFormat.formatToParts
  var FIELDS = ["weekday", "era", "year", "month", "day", "hour", "minute", "second", "fractionalSecondDigits", "timeZoneName", "dayPeriod"];
  // Remember each formatter's options: Boa's resolvedOptions() leaves the fields out.
  var DTF = function DateTimeFormat(locales, options) {
    var f = new BaseDTF(locales, options);
    f.__opts = Object.assign({}, options || {});
    return f;
  };
  DTF.prototype = BaseDTF.prototype;
  DTF.supportedLocalesOf = BaseDTF.supportedLocalesOf;
  Intl.DateTimeFormat = DTF;
  if (!BaseDTF.prototype.formatToParts) {
    BaseDTF.prototype.formatToParts = function (date) {
      var d = date === undefined ? new Date() : new Date(date);
      var full = this.format(d);
      var ro = Object.assign({}, this.resolvedOptions());
      var given = this.__opts || {};
      var any = false;
      FIELDS.forEach(function (f) { if (given[f] !== undefined) { ro[f] = given[f]; any = true; } });
      if (!any && given.dateStyle === undefined && given.timeStyle === undefined && ro.year === undefined && ro.hour === undefined) {
        ro.year = "numeric"; ro.month = "numeric"; ro.day = "numeric";
      }
      if (given.hour12 !== undefined) ro.hour12 = given.hour12;
      if (ro.hour !== undefined && ro.hour12 === undefined && !/^(h23|h24)$/.test(ro.hourCycle || "")) ro.hour12 = /\b(AM|PM)\b/.test(this.format(new Date(2000, 0, 1, 13)));
      var opts = {};
      if (ro.timeZone) opts.timeZone = ro.timeZone;
      if (ro.hour12 !== undefined) opts.hour12 = ro.hour12;
      // Each field's text for this date, found in the full string in order.
      var tokens = [];
      for (var i = 0; i < FIELDS.length; i++) {
        var f = FIELDS[i];
        if (ro[f] === undefined || f === "dayPeriod") continue;
        var one = {}; one[f] = ro[f];
        var text;
        try { text = new BaseDTF(ro.locale, Object.assign({}, opts, one)).format(d); } catch (e) { continue; }
        if (f === "hour") text = (/\d+/.exec(text) || [text])[0];
        if (f === "minute" || f === "second") text = (/\d+/.exec(text) || [text])[0];
        if (f === "year") text = (/\d+/.exec(text) || [text])[0];
        if (f === "day" && /^\d/.test(text)) text = (/\d+/.exec(text) || [text])[0];
        tokens.push([f, text]);
      }
      if (ro.hour !== undefined && (ro.hour12 || ro.hourCycle === "h12" || ro.hourCycle === "h11")) {
        var pm = /\b(AM|PM|a\.m\.|p\.m\.|am|pm)\b/.exec(full);
        if (pm) tokens.push(["dayPeriod", pm[1]]);
      }
      // Find each token's position, preferring the leftmost unused spot.
      var found = [];
      var used = [];
      tokens.forEach(function (t) {
        var from = 0, at;
        while ((at = full.indexOf(t[1], from)) >= 0) {
          var clash = used.some(function (u) { return at < u[1] && at + t[1].length > u[0]; });
          var inDigits = /\d/.test(t[1][0]) && ((at > 0 && /\d/.test(full[at - 1])) || /\d/.test(full[at + t[1].length] || ""));
          if (!clash && !inDigits) break;
          from = at + 1;
        }
        if (at >= 0) { used.push([at, at + t[1].length]); found.push({ type: t[0], at: at, value: t[1] }); }
      });
      found.sort(function (a, b) { return a.at - b.at; });
      var out = [], pos = 0;
      found.forEach(function (f) {
        if (f.at > pos) out.push({ type: "literal", value: full.slice(pos, f.at) });
        out.push({ type: f.type, value: f.value });
        pos = f.at + f.value.length;
      });
      if (pos < full.length) out.push({ type: "literal", value: full.slice(pos) });
      return out;
    };
  }
  if (!BaseDTF.prototype.formatRange) {
    BaseDTF.prototype.formatRange = function (a, b) { return this.format(a) + " – " + this.format(b); };
  }

  // ------------------------------------------------------------ RelativeTimeFormat
  var RT = {
    en: { units: { year: ["year", "years"], quarter: ["quarter", "quarters"], month: ["month", "months"], week: ["week", "weeks"], day: ["day", "days"], hour: ["hour", "hours"], minute: ["minute", "minutes"], second: ["second", "seconds"] },
      short: { year: ["yr.", "yr."], quarter: ["qtr.", "qtrs."], month: ["mo.", "mo."], week: ["wk.", "wk."], day: ["day", "days"], hour: ["hr.", "hr."], minute: ["min.", "min."], second: ["sec.", "sec."] },
      future: "in {0}", past: "{0} ago",
      auto: { day: { "-1": "yesterday", "0": "today", "1": "tomorrow" }, year: { "-1": "last year", "0": "this year", "1": "next year" }, month: { "-1": "last month", "0": "this month", "1": "next month" }, week: { "-1": "last week", "0": "this week", "1": "next week" }, second: { "0": "now" }, quarter: { "-1": "last quarter", "0": "this quarter", "1": "next quarter" } } },
    es: { units: { year: ["año", "años"], quarter: ["trimestre", "trimestres"], month: ["mes", "meses"], week: ["semana", "semanas"], day: ["día", "días"], hour: ["hora", "horas"], minute: ["minuto", "minutos"], second: ["segundo", "segundos"] },
      future: "dentro de {0}", past: "hace {0}",
      auto: { day: { "-2": "anteayer", "-1": "ayer", "0": "hoy", "1": "mañana", "2": "pasado mañana" }, year: { "-1": "el año pasado", "0": "este año", "1": "el próximo año" }, second: { "0": "ahora" } } },
    fr: { units: { year: ["an", "ans"], quarter: ["trimestre", "trimestres"], month: ["mois", "mois"], week: ["semaine", "semaines"], day: ["jour", "jours"], hour: ["heure", "heures"], minute: ["minute", "minutes"], second: ["seconde", "secondes"] },
      future: "dans {0}", past: "il y a {0}",
      auto: { day: { "-2": "avant-hier", "-1": "hier", "0": "aujourd’hui", "1": "demain", "2": "après-demain" }, year: { "-1": "l’année dernière", "0": "cette année", "1": "l’année prochaine" }, second: { "0": "maintenant" } } },
    de: { units: { year: ["Jahr", "Jahren", "Jahren"], quarter: ["Quartal", "Quartalen", "Quartalen"], month: ["Monat", "Monaten", "Monaten"], week: ["Woche", "Wochen", "Wochen"], day: ["Tag", "Tagen", "Tagen"], hour: ["Stunde", "Stunden", "Stunden"], minute: ["Minute", "Minuten", "Minuten"], second: ["Sekunde", "Sekunden", "Sekunden"] },
      futureUnits: { year: ["Jahr", "Jahren"], month: ["Monat", "Monaten"], week: ["Woche", "Wochen"], day: ["Tag", "Tagen"], hour: ["Stunde", "Stunden"], minute: ["Minute", "Minuten"], second: ["Sekunde", "Sekunden"], quarter: ["Quartal", "Quartalen"] },
      future: "in {0}", past: "vor {0}",
      auto: { day: { "-2": "vorgestern", "-1": "gestern", "0": "heute", "1": "morgen", "2": "übermorgen" }, year: { "-1": "letztes Jahr", "0": "dieses Jahr", "1": "nächstes Jahr" }, second: { "0": "jetzt" } } }
  };
  var UNIT_ALIASES = { years: "year", quarters: "quarter", months: "month", weeks: "week", days: "day", hours: "hour", minutes: "minute", seconds: "second" };
  function RelativeTimeFormat(locales, options) {
    if (!(this instanceof RelativeTimeFormat)) throw new TypeError("Constructor Intl.RelativeTimeFormat requires 'new'");
    var o = options || {};
    var loc = pickLocale(locales);
    this._lang = RT[lang(loc)] ? lang(loc) : "en";
    this._loc = RT[lang(loc)] ? loc : "en";
    this._numeric = o.numeric || "always";
    this._style = o.style || "long";
    this._nf = new NumberFormat(this._loc);
  }
  RelativeTimeFormat.prototype.formatToParts = function (value, unit) {
    var u = UNIT_ALIASES[unit] || unit;
    var t = RT[this._lang];
    if (!t.units[u]) throw new RangeError("Invalid unit argument for format() '" + unit + "'");
    var v = Number(value);
    if (!isFinite(v)) throw new RangeError("Invalid value");
    if (this._numeric === "auto" && t.auto[u] && hasOwn(t.auto[u], String(v))) return [{ type: "literal", value: t.auto[u][String(v)] }];
    var past = v < 0 || Object.is(v, -0);
    var abs = Math.abs(v);
    var names = (this._style !== "long" && t.short && t.short[u]) || (!past && t.futureUnits && t.futureUnits[u]) || t.units[u];
    var word = abs === 1 ? names[0] : names[1];
    var num = this._nf.formatToParts(abs).map(function (p) { p.unit = u; return p; });
    var pattern = past ? t.past : t.future;
    var i = pattern.indexOf("{0}");
    var out = [];
    if (i > 0) out.push({ type: "literal", value: pattern.slice(0, i) });
    out = out.concat(num);
    out.push({ type: "literal", value: " " + word + pattern.slice(i + 3) });
    return out;
  };
  RelativeTimeFormat.prototype.format = function (value, unit) {
    return this.formatToParts(value, unit).map(function (p) { return p.value; }).join("");
  };
  RelativeTimeFormat.prototype.resolvedOptions = function () {
    return { locale: this._loc, style: this._style, numeric: this._numeric, numberingSystem: "latn" };
  };
  RelativeTimeFormat.supportedLocalesOf = function (l) {
    return Intl.getCanonicalLocales(l === undefined ? [] : l).filter(function (x) { return RT[lang(x)]; });
  };
  if (!Intl.RelativeTimeFormat) Intl.RelativeTimeFormat = RelativeTimeFormat;

  // ------------------------------------------------------------ DisplayNames (English names)
  var NAMES = {
    region: { US: "United States", GB: "United Kingdom", CA: "Canada", MX: "Mexico", DE: "Germany", FR: "France", ES: "Spain", IT: "Italy",
      PT: "Portugal", NL: "Netherlands", BE: "Belgium", CH: "Switzerland", AT: "Austria", SE: "Sweden", NO: "Norway", DK: "Denmark",
      FI: "Finland", PL: "Poland", CZ: "Czechia", IE: "Ireland", GR: "Greece", TR: "Türkiye", RU: "Russia", UA: "Ukraine",
      CN: "China", JP: "Japan", KR: "South Korea", IN: "India", PK: "Pakistan", BD: "Bangladesh", ID: "Indonesia", PH: "Philippines",
      VN: "Vietnam", TH: "Thailand", MY: "Malaysia", SG: "Singapore", AU: "Australia", NZ: "New Zealand", BR: "Brazil", AR: "Argentina",
      CL: "Chile", CO: "Colombia", PE: "Peru", VE: "Venezuela", ZA: "South Africa", NG: "Nigeria", EG: "Egypt", KE: "Kenya",
      SA: "Saudi Arabia", AE: "United Arab Emirates", IL: "Israel", IR: "Iran", IQ: "Iraq", TW: "Taiwan", HK: "Hong Kong SAR China", "419": "Latin America", "001": "world" },
    language: { en: "English", es: "Spanish", fr: "French", de: "German", it: "Italian", pt: "Portuguese", nl: "Dutch", sv: "Swedish",
      no: "Norwegian", nb: "Norwegian Bokmål", da: "Danish", fi: "Finnish", pl: "Polish", cs: "Czech", el: "Greek", tr: "Turkish",
      ru: "Russian", uk: "Ukrainian", zh: "Chinese", ja: "Japanese", ko: "Korean", hi: "Hindi", ar: "Arabic", he: "Hebrew", fa: "Persian",
      ur: "Urdu", bn: "Bangla", id: "Indonesian", ms: "Malay", vi: "Vietnamese", th: "Thai", tl: "Tagalog", sw: "Swahili",
      "en-US": "American English", "en-GB": "British English", "es-MX": "Mexican Spanish", "fr-CA": "Canadian French",
      "pt-BR": "Brazilian Portuguese", "zh-Hans": "Simplified Chinese", "zh-Hant": "Traditional Chinese" },
    currency: { USD: "US Dollar", EUR: "Euro", GBP: "British Pound", JPY: "Japanese Yen", CNY: "Chinese Yuan", INR: "Indian Rupee",
      CAD: "Canadian Dollar", AUD: "Australian Dollar", MXN: "Mexican Peso", BRL: "Brazilian Real", CHF: "Swiss Franc", KRW: "South Korean Won" },
    script: { Latn: "Latin", Cyrl: "Cyrillic", Arab: "Arabic", Hans: "Simplified Han", Hant: "Traditional Han", Grek: "Greek", Hebr: "Hebrew", Deva: "Devanagari" }
  };
  function DisplayNames(locales, options) {
    if (!(this instanceof DisplayNames)) throw new TypeError("Constructor Intl.DisplayNames requires 'new'");
    var o = options || {};
    if (!o.type || !NAMES[o.type] && o.type !== "calendar" && o.type !== "dateTimeField") throw new TypeError("Intl.DisplayNames: type is required");
    this._type = o.type;
    this._fallback = o.fallback || "code";
    this._loc = pickLocale(locales);
    this._style = o.style || "long";
  }
  DisplayNames.prototype.of = function (code) {
    var c = String(code);
    var table = NAMES[this._type] || {};
    var key = this._type === "region" || this._type === "currency" ? c.toUpperCase() : c;
    if (hasOwn(table, key)) return table[key];
    if (this._type === "language") {
      var p = key.split("-");
      if (table[p[0]] && p[1] && NAMES.region[p[1].toUpperCase()]) return table[p[0]] + " (" + NAMES.region[p[1].toUpperCase()] + ")";
    }
    return this._fallback === "none" ? undefined : key;
  };
  DisplayNames.prototype.resolvedOptions = function () { return { locale: this._loc, style: this._style, type: this._type, fallback: this._fallback }; };
  DisplayNames.supportedLocalesOf = function (l) { return Intl.getCanonicalLocales(l === undefined ? [] : l); };
  if (!Intl.DisplayNames) Intl.DisplayNames = DisplayNames;

  if (!Intl.supportedValuesOf) {
    Intl.supportedValuesOf = function (key) {
      switch (key) {
        case "currency": return Object.keys(SYMBOLS).concat(Object.keys(DIGITS)).filter(function (v, i, a) { return a.indexOf(v) === i; }).sort();
        case "unit": return Object.keys(UNITS).sort();
        case "calendar": return ["buddhist", "chinese", "gregory", "hebrew", "indian", "islamic", "iso8601", "japanese", "persian", "roc"];
        case "collation": return ["compat", "emoji", "eor", "phonebk", "pinyin", "stroke", "trad", "unihan", "zhuyin"];
        case "numberingSystem": return ["arab", "beng", "deva", "fullwide", "latn", "thai"];
        case "timeZone": return ["UTC", "America/New_York", "America/Chicago", "America/Denver", "America/Los_Angeles", "Europe/London", "Europe/Paris", "Europe/Berlin", "Asia/Tokyo", "Asia/Kolkata", "Asia/Shanghai", "Australia/Sydney"];
        default: throw new RangeError("Invalid key : " + key);
      }
    };
  }
})();
