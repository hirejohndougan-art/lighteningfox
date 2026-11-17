// Stand-ins for ad and analytics libraries, answered in place of the real file when a filter
// says `$redirect=<name>`. Each one gives a page the objects it expects from the library, doing
// nothing, so the page keeps working without the ads or the tracking. Written for Ironswift
// (MIT); the names are the ones uBlock Origin's filter lists use.
//
// Each section starts with a line `//@ name alias…` and runs on its own.

//@ googletagmanager_gtm.js googletagmanager.com/gtm.js googletagmanager_gtag.js
(function() {
  'use strict';
  var noop = function() {};
  var w = window;
  // Pages wait for these callbacks before going on (often before following a link).
  function callBack(o) {
    if (!o || typeof o !== 'object') return;
    var cb = o.eventCallback;
    if (typeof cb === 'function') setTimeout(function() { try { cb(); } catch (e) {} }, 1);
  }
  var dl = w.dataLayer;
  if (!Array.isArray(dl)) dl = w.dataLayer = [];
  dl.hide = { end: noop };
  for (var i = 0; i < dl.length; i++) callBack(dl[i]);
  var push = Array.prototype.push;
  dl.push = function() {
    for (var i = 0; i < arguments.length; i++) callBack(arguments[i]);
    return push.apply(this, arguments);
  };
  w.google_tag_manager = w.google_tag_manager || { dataLayer: { get: noop, set: noop, reset: noop, gtmDom: true, gtmLoad: true } };
  if (typeof w.gtag !== 'function') w.gtag = function() { dl.push(arguments); };
  // The anti-flicker snippet hides the page until this is called.
  if (w.dataLayer.hide && typeof w.dataLayer.hide.end === 'function') w.dataLayer.hide.end();
  try { document.documentElement.classList.remove('async-hide'); } catch (e) {}
})();

//@ google-analytics_analytics.js google-analytics.com/analytics.js google-analytics_ga.js google-analytics.com/ga.js google-analytics_cx_api.js google-analytics_inpage_linkid.js
(function() {
  'use strict';
  var noop = function() {};
  var w = window;
  var tracker = {
    get: function() {},
    set: noop,
    send: function() {
      var last = arguments[arguments.length - 1];
      if (last && typeof last.hitCallback === 'function') { try { last.hitCallback(); } catch (e) {} }
    },
  };
  // analytics.js: ga('create', …), ga(function(tracker) {…}), ga('send', {hitCallback}).
  var name = w.GoogleAnalyticsObject || 'ga';
  var queue = w[name] && w[name].q;
  var ga = function() {
    var a = arguments;
    if (a.length === 0) return;
    var f = a[0];
    if (typeof f === 'function') { try { f(tracker); } catch (e) {} return; }
    var last = a[a.length - 1];
    if (last && typeof last === 'object' && typeof last.hitCallback === 'function') { try { last.hitCallback(); } catch (e) {} }
    else if (typeof last === 'function') { try { last(tracker); } catch (e) {} }
  };
  ga.create = function() { return tracker; };
  ga.getByName = function() { return tracker; };
  ga.getAll = function() { return [tracker]; };
  ga.remove = noop;
  ga.loaded = true;
  w[name] = ga;
  if (Array.isArray(queue)) for (var i = 0; i < queue.length; i++) ga.apply(null, queue[i]);
  // ga.js (the old _gaq API).
  var gaq = w._gaq;
  var gat = {
    _getTracker: function() { return { _initData: noop, _trackPageview: noop, _trackEvent: noop, _setAllowLinker: noop, _setCustomVar: noop, _link: function(u) { location.href = u; }, _getLinkerUrl: function(u) { return u; } }; },
    _getTrackerByName: function() { return gat._getTracker(); },
    _anonymizeIp: noop,
    _createTracker: function() { return gat._getTracker(); },
    _forceSSL: noop,
  };
  w._gat = gat;
  var q = { push: function() { for (var i = 0; i < arguments.length; i++) { var c = arguments[i]; if (typeof c === 'function') { try { c(); } catch (e) {} } } return 0; } };
  if (Array.isArray(gaq)) for (var j = 0; j < gaq.length; j++) q.push(gaq[j]);
  w._gaq = q;
  // cx_api.js / inpage_linkid.js
  w.cxApi = w.cxApi || { chooseVariation: function() { return 0; }, getChosenVariation: function() { return 0; }, setAllowHash: noop, setChosenVariation: noop, setCookiePath: noop, setDomainName: noop };
})();

//@ googletagservices_gpt.js googletagservices.com/gpt.js
(function() {
  'use strict';
  var noop = function() {};
  var noopThis = function() { return this; };
  var noopNull = function() { return null; };
  var noopArr = function() { return []; };
  var noopStr = function() { return ''; };
  var w = window;
  var slot = function(path, id) {
    var s = {
      addService: noopThis, clearCategoryExclusions: noopThis, clearTargeting: noopThis, defineSizeMapping: noopThis,
      get: noopNull, getAdUnitPath: function() { return path || ''; }, getAttributeKeys: noopArr, getCategoryExclusions: noopArr,
      getDomId: function() { return id || ''; }, getResponseInformation: noopNull, getSlotElementId: function() { return id || ''; },
      getSlotId: function() { return { getId: function() { return id || ''; }, getDomId: function() { return id || ''; } }; },
      getTargeting: noopArr, getTargetingKeys: noopArr, set: noopThis, setCategoryExclusion: noopThis, setClickUrl: noopThis,
      setCollapseEmptyDiv: noopThis, setConfig: noopThis, setForceSafeFrame: noopThis, setSafeFrameConfig: noopThis, setTargeting: noopThis,
      updateTargetingFromMap: noopThis,
    };
    return s;
  };
  var listeners = {};
  var pubads = {
    addEventListener: function(name, f) { (listeners[name] = listeners[name] || []).push(f); return this; },
    removeEventListener: function() { return true; },
    clear: noop, clearCategoryExclusions: noopThis, clearTagForChildDirectedTreatment: noopThis, clearTargeting: noopThis,
    collapseEmptyDivs: noopThis, defineOutOfPagePassback: function() { return slot(); }, definePassback: function() { return slot(); },
    disableInitialLoad: noop, display: noop, enableAsyncRendering: noop, enableLazyLoad: noop, enableSingleRequest: noop,
    enableSyncRendering: noop, enableVideoAds: noop, get: noopNull, getAttributeKeys: noopArr, getTargeting: noopArr,
    getTargetingKeys: noopArr, getSlots: noopArr, isInitialLoadDisabled: function() { return true; }, refresh: noop, set: noopThis,
    setCategoryExclusion: noopThis, setCentering: noop, setCookieOptions: noopThis, setForceSafeFrame: noopThis, setLocation: noopThis,
    setPrivacySettings: noopThis, setPublisherProvidedId: noopThis, setRequestNonPersonalizedAds: noopThis, setSafeFrameConfig: noopThis,
    setTagForChildDirectedTreatment: noopThis, setTargeting: noopThis, setVideoContent: noopThis, updateCorrelator: noop,
  };
  var companion = { addEventListener: noopThis, enableSyncLoading: noop, setRefreshUnfilledSlots: noop, getSlots: noopArr };
  var content = { addEventListener: noopThis, setContent: noop };
  var cmd = w.googletag && w.googletag.cmd;
  var run = function(f) { if (typeof f === 'function') { try { f(); } catch (e) { console.error(e); } } };
  var gt = {
    apiReady: true,
    pubadsReady: true,
    cmd: { push: function() { for (var i = 0; i < arguments.length; i++) run(arguments[i]); return 0; } },
    companionAds: function() { return companion; },
    content: function() { return content; },
    defineOutOfPageSlot: function(p, id) { return slot(p, id); },
    defineSlot: function(p, s, id) { return slot(p, id); },
    destroySlots: function() { return true; },
    disablePublisherConsole: noop,
    display: noop,
    enableServices: noop,
    getVersion: noopStr,
    pubads: function() { return pubads; },
    setAdIframeTitle: noop,
    setConfig: noop,
    sizeMapping: function() { var b = { addSize: function() { return b; }, build: noopNull }; return b; },
    openConsole: noop,
    getEventLog: function() { return { getAllEvents: noopArr, getEventsBySeverity: noopArr, getEventsBySlot: noopArr, getEventsByLevel: noopArr }; },
    secureSignalProviders: { push: noop, clearAllCache: noop },
    enums: { OutOfPageFormat: { REWARDED: 1, TOP_ANCHOR: 2, BOTTOM_ANCHOR: 3, INTERSTITIAL: 5 } },
  };
  w.googletag = gt;
  if (Array.isArray(cmd)) for (var i = 0; i < cmd.length; i++) run(cmd[i]);
})();

//@ googlesyndication_adsbygoogle.js googlesyndication.com/adsbygoogle.js
(function() {
  'use strict';
  var w = window;
  var mark = function() {
    var slots = document.querySelectorAll('ins.adsbygoogle');
    for (var i = 0; i < slots.length; i++) {
      var s = slots[i];
      if (s.getAttribute('data-adsbygoogle-status')) continue;
      s.setAttribute('data-adsbygoogle-status', 'done');
      s.setAttribute('data-ad-status', 'unfilled');
    }
  };
  var old = w.adsbygoogle;
  w.adsbygoogle = {
    loaded: true,
    push: function(o) {
      if (o && typeof o === 'object' && o.params && typeof o.params.google_ad_client === 'string') { /* page-level ads: nothing */ }
      if (o && typeof o === 'object' && typeof o.onload === 'function') { try { o.onload(); } catch (e) {} }
      mark();
    },
  };
  if (Array.isArray(old)) for (var i = 0; i < old.length; i++) w.adsbygoogle.push(old[i]);
  mark();
})();

//@ google-ima.js imasdk.googleapis.com/js/sdkloader/ima3.js
(function() {
  'use strict';
  // Enough of the IMA SDK for video players to ask for an ad, get an error back, and play the
  // video itself.
  var w = window;
  if (w.google && w.google.ima && w.google.ima.AdsLoader) return;
  var noop = function() {};
  function Emitter() { this.l = {}; }
  Emitter.prototype.addEventListener = function(types, f, c, scope) {
    if (!Array.isArray(types)) types = [types];
    for (var i = 0; i < types.length; i++) (this.l[types[i]] = this.l[types[i]] || []).push(scope ? f.bind(scope) : f);
  };
  Emitter.prototype.removeEventListener = function(types, f) {
    if (!Array.isArray(types)) types = [types];
    for (var i = 0; i < types.length; i++) { var a = this.l[types[i]] || []; var k = a.indexOf(f); if (k >= 0) a.splice(k, 1); }
  };
  Emitter.prototype.emit = function(type, ev) {
    var a = (this.l[type] || []).slice();
    for (var i = 0; i < a.length; i++) { try { a[i](ev); } catch (e) { console.error(e); } }
  };
  var AdErrorType = { AD_LOAD: 'adLoadError', AD_PLAY: 'adPlayError' };
  var ErrorCode = { VAST_EMPTY_RESPONSE: 1009, VAST_NO_ADS_AFTER_WRAPPER: 303, UNKNOWN_ERROR: 900 };
  function AdError() {}
  AdError.prototype.getErrorCode = function() { return ErrorCode.VAST_EMPTY_RESPONSE; };
  AdError.prototype.getVastErrorCode = function() { return 303; };
  AdError.prototype.getInnerError = function() { return null; };
  AdError.prototype.getMessage = function() { return 'No ads'; };
  AdError.prototype.getType = function() { return AdErrorType.AD_LOAD; };
  AdError.prototype.toString = function() { return 'AdError 1009: No ads'; };
  AdError.ErrorCode = ErrorCode;
  AdError.Type = AdErrorType;
  function AdErrorEvent(err, ctx) { this.err = err; this.ctx = ctx; this.type = AdErrorEvent.Type.AD_ERROR; }
  AdErrorEvent.prototype.getError = function() { return this.err; };
  AdErrorEvent.prototype.getUserRequestContext = function() { return this.ctx || {}; };
  AdErrorEvent.Type = { AD_ERROR: 'adError' };
  var AdEventType = {};
  ['AD_BREAK_READY', 'AD_BUFFERING', 'AD_CAN_PLAY', 'AD_METADATA', 'AD_PROGRESS', 'ALL_ADS_COMPLETED', 'CLICK', 'COMPLETE', 'CONTENT_PAUSE_REQUESTED',
   'CONTENT_RESUME_REQUESTED', 'DURATION_CHANGE', 'FIRST_QUARTILE', 'IMPRESSION', 'INTERACTION', 'LINEAR_CHANGED', 'LOADED', 'LOG', 'MIDPOINT',
   'PAUSED', 'RESUMED', 'SKIPPABLE_STATE_CHANGED', 'SKIPPED', 'STARTED', 'THIRD_QUARTILE', 'USER_CLOSE', 'VIDEO_CLICKED', 'VIDEO_ICON_CLICKED',
   'VOLUME_CHANGED', 'VOLUME_MUTED'].forEach(function(k) { AdEventType[k] = k.toLowerCase(); });
  function AdEvent(type) { this.type = type; }
  AdEvent.prototype.getAd = function() { return null; };
  AdEvent.prototype.getAdData = function() { return {}; };
  AdEvent.Type = AdEventType;
  function AdsManager() { Emitter.call(this); this.volume = 1; }
  AdsManager.prototype = Object.create(Emitter.prototype);
  ['collapse', 'configureAdsManager', 'destroy', 'discardAdBreak', 'expand', 'focus', 'pause', 'resize', 'resume', 'setAdWillAutoPlay',
   'setAdWillPlayMuted', 'skip', 'stop', 'updateAdsRenderingSettings', 'clicked'].forEach(function(k) { AdsManager.prototype[k] = noop; });
  AdsManager.prototype.init = noop;
  AdsManager.prototype.start = function() {
    var self = this;
    setTimeout(function() { self.emit(AdEventType.CONTENT_RESUME_REQUESTED, new AdEvent(AdEventType.CONTENT_RESUME_REQUESTED)); self.emit(AdEventType.ALL_ADS_COMPLETED, new AdEvent(AdEventType.ALL_ADS_COMPLETED)); }, 1);
  };
  AdsManager.prototype.getCuePoints = function() { return []; };
  AdsManager.prototype.getAdSkippableState = function() { return false; };
  AdsManager.prototype.getRemainingTime = function() { return 0; };
  AdsManager.prototype.getVolume = function() { return this.volume; };
  AdsManager.prototype.setVolume = function(v) { this.volume = v; };
  AdsManager.prototype.isCustomClickTrackingUsed = function() { return false; };
  AdsManager.prototype.isCustomPlaybackUsed = function() { return false; };
  function AdsManagerLoadedEvent(ctx) { this.ctx = ctx; this.type = AdsManagerLoadedEvent.Type.ADS_MANAGER_LOADED; }
  AdsManagerLoadedEvent.prototype.getAdsManager = function() { return new AdsManager(); };
  AdsManagerLoadedEvent.prototype.getUserRequestContext = function() { return this.ctx || {}; };
  AdsManagerLoadedEvent.Type = { ADS_MANAGER_LOADED: 'adsManagerLoaded' };
  function AdsLoader() { Emitter.call(this); this.settings = new ImaSdkSettings(); }
  AdsLoader.prototype = Object.create(Emitter.prototype);
  AdsLoader.prototype.contentComplete = noop;
  AdsLoader.prototype.destroy = noop;
  AdsLoader.prototype.getSettings = function() { return this.settings; };
  AdsLoader.prototype.getVersion = function() { return '3.0.0'; };
  AdsLoader.prototype.requestAds = function(req, ctx) {
    var self = this;
    setTimeout(function() { self.emit(AdErrorEvent.Type.AD_ERROR, new AdErrorEvent(new AdError(), ctx)); }, 1);
  };
  function AdDisplayContainer() {}
  AdDisplayContainer.prototype.initialize = noop;
  AdDisplayContainer.prototype.destroy = noop;
  function AdsRenderingSettings() {}
  function AdsRequest() {}
  AdsRequest.prototype.setAdWillAutoPlay = noop;
  AdsRequest.prototype.setAdWillPlayMuted = noop;
  AdsRequest.prototype.setContinuousPlayback = noop;
  function ImaSdkSettings() {}
  ['setAutoPlayAdBreaks', 'setCompanionBackfill', 'setCookiesEnabled', 'setDisableCustomPlaybackForIOS10Plus', 'setFeatureFlags', 'setLocale',
   'setNumRedirects', 'setPlayerType', 'setPlayerVersion', 'setPpid', 'setSessionId', 'setVpaidAllowed', 'setVpaidMode'].forEach(function(k) { ImaSdkSettings.prototype[k] = noop; });
  ImaSdkSettings.prototype.getLocale = function() { return 'en'; };
  ImaSdkSettings.prototype.getNumRedirects = function() { return 0; };
  ImaSdkSettings.prototype.getPlayerType = function() { return ''; };
  ImaSdkSettings.prototype.getPlayerVersion = function() { return ''; };
  ImaSdkSettings.prototype.getPpid = function() { return ''; };
  ImaSdkSettings.prototype.getCompanionBackfill = function() { return 'always'; };
  ImaSdkSettings.prototype.getDisableCustomPlaybackForIOS10Plus = function() { return false; };
  ImaSdkSettings.prototype.isCookiesEnabled = function() { return false; };
  ImaSdkSettings.prototype.isVpaidAdapter = function() { return false; };
  ImaSdkSettings.VpaidMode = { DISABLED: 0, ENABLED: 1, INSECURE: 2 };
  ImaSdkSettings.CompanionBackfillMode = { ALWAYS: 'always', ON_MASTER_AD: 'on_master_ad' };
  w.google = w.google || {};
  w.google.ima = {
    AdDisplayContainer: AdDisplayContainer, AdError: AdError, AdErrorEvent: AdErrorEvent, AdEvent: AdEvent, AdsLoader: AdsLoader,
    AdsManager: AdsManager, AdsManagerLoadedEvent: AdsManagerLoadedEvent, AdsRenderingSettings: AdsRenderingSettings, AdsRequest: AdsRequest,
    ImaSdkSettings: ImaSdkSettings, settings: new ImaSdkSettings(), UiElements: { AD_ATTRIBUTION: 'adAttribution', COUNTDOWN: 'countdown' },
    ViewMode: { NORMAL: 'normal', FULLSCREEN: 'fullscreen' }, VERSION: '3.0.0', OmidVerificationVendor: {}, OmidAccessMode: {},
    CompanionAdSelectionSettings: function() {},
    gptProxyInstance: {}, dai: {},
  };
  w.google.ima.CompanionAdSelectionSettings.CreativeType = { ALL: 'All', FLASH: 'Flash', IMAGE: 'Image' };
  w.google.ima.CompanionAdSelectionSettings.ResourceType = { ALL: 'All', HTML: 'Html', IFRAME: 'IFrame', STATIC: 'Static' };
  w.google.ima.CompanionAdSelectionSettings.SizeCriteria = { IGNORE: 'IgnoreSize', SELECT_EXACT_MATCH: 'SelectExactMatch', SELECT_NEAR_MATCH: 'SelectNearMatch' };
})();

//@ scorecardresearch_beacon.js scorecardresearch.com/beacon.js
(function() {
  'use strict';
  var noop = function() {};
  window.COMSCORE = { purge: function() { window._comscore = []; }, beacon: noop };
  window._comscore = [];
  window.ns_ = window.ns_ || { StreamingAnalytics: function() { return { playVideoContentPart: noop, notifyPlay: noop, notifyPause: noop, notifyEnd: noop, setLabels: noop, createPlaybackSession: noop }; } };
})();

//@ outbrain-widget.js widgets.outbrain.com/outbrain.js
(function() {
  'use strict';
  var noop = function() {};
  var obr = { extern: { video: { getVideoRecs: noop, videoClicked: noop } } };
  ['callClick', 'closeCard', 'closeModal', 'getCountOfRecs', 'getStat', 'imageError', 'manualVideoClicked', 'onOdbReturn', 'onVideoClick',
   'pagerLoad', 'recClicked', 'refreshSpecificWidget', 'refreshWidget', 'reloadWidget', 'researchWidget', 'returnedError', 'returnedHtmlData',
   'returnedIrdData', 'returnedJsonData', 'scrollLoad', 'showDescription', 'showRecInIframe', 'userZappedMessage', 'zappedMessage'].forEach(function(k) { obr.extern[k] = noop; });
  window.OBR = window.OBR || obr;
})();

//@ amazon_apstag.js amazon-adsystem.com/aax2/amzn_ads.js amazon_ads.js
(function() {
  'use strict';
  var noop = function() {};
  var w = window;
  var q = w.apstag && w.apstag._Q;
  w.apstag = {
    _Q: [],
    fetchBids: function(cfg, cb) { if (typeof cb === 'function') setTimeout(function() { try { cb([]); } catch (e) {} }, 1); },
    init: noop,
    setDisplayBids: noop,
    targetingKeys: function() { return []; },
    renderImp: noop,
    punt: noop,
    dpa: noop,
    rpa: noop,
    upa: noop,
  };
  if (Array.isArray(q)) for (var i = 0; i < q.length; i++) { var c = q[i]; if (c && c[0] === 'f' && typeof c[1] === 'object') w.apstag.fetchBids(c[1][0], c[1][1]); }
  w.amznads = w.amznads || { getAds: noop, getAdsCallback: function(id, cb) { if (typeof cb === 'function') cb(); }, renderAd: noop, appendTargetingToAdServerUrl: function(u) { return u; }, appendScriptTag: noop, getTargeting: function() { return {}; } };
  w.amzn_ads = w.amzn_ads || noop;
})();

//@ doubleclick_instream_ad_status.js doubleclick.net/instream/ad_status.js
window.google_ad_status = 1;

//@ fuckadblock.js-3.2.0 fuckadblock.com/fuckadblock.js fuckadblock.js nofab.js
(function() {
  'use strict';
  // Anti-adblock scripts built on FuckAdBlock / BlockAdBlock: always report "no ad blocker".
  var noop = function() {};
  var Fab = function() {};
  Fab.prototype.check = noop;
  Fab.prototype.clearEvent = noop;
  Fab.prototype.emitEvent = noop;
  Fab.prototype.on = function(detected, fn) { if (!detected && typeof fn === 'function') { try { fn(); } catch (e) {} } return this; };
  Fab.prototype.onDetected = function() { return this; };
  Fab.prototype.onNotDetected = function(fn) { if (typeof fn === 'function') { try { fn(); } catch (e) {} } return this; };
  Fab.prototype.setOption = noop;
  Fab.prototype.options = { set: noop, get: noop };
  var fab = new Fab();
  var names = ['FuckAdBlock', 'BlockAdBlock', 'SniffAdBlock'];
  for (var i = 0; i < names.length; i++) window[names[i]] = Fab;
  window.fuckAdBlock = window.blockAdBlock = window.sniffAdBlock = fab;
})();

//@ prebid-ads.js
window.canRunAds = true;
window.isAdBlockActive = false;

//@ chartbeat.js static.chartbeat.com/chartbeat.js
(function() {
  'use strict';
  var noop = function() {};
  window.pSUPERFLY = window.pSUPERFLY || { activity: noop, virtualPage: noop };
  window.pSUPERFLY_mab = window.pSUPERFLY_mab || { get: noop };
})();

//@ ampproject_v0.js popads.js popads.net.js
(function() {})();
