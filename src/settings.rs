//! User settings, saved in the profile folder (settings.tsv) and edited on about:settings.

use std::sync::{OnceLock, RwLock};

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Page for new windows and the start of a session when tabs aren't restored.
    pub home: String,
    /// Key into SEARCH_ENGINES.
    pub search: String,
    /// Reopen the last session's tabs at startup.
    pub restore_tabs: bool,
    /// Background tabs unused this long free their page (0 = never).
    pub sleep_minutes: u32,
    pub third_party_cookies: bool,
    pub strip_tracking: bool,
    pub send_gpc: bool,
    pub show_bookmarks_bar: bool,
    /// Draw with the graphics card (takes effect at the next start).
    pub use_gpu: bool,
    /// Run each site's pages in their own sandboxed process (process.rs; next start).
    pub site_processes: bool,
    /// "system" (desktop settings and the *_proxy variables), "none" or "manual".
    pub proxy_mode: String,
    /// For manual: http://host:port, https://… or socks5://host:port.
    pub proxy_url: String,
    /// Delete cookies, site storage and cached files when Ironswift closes.
    pub clear_on_exit: bool,
    /// Text is never drawn smaller than this (px; 0 = no minimum).
    pub min_font_size: u32,
    /// Tell sites to keep animation to a minimum (prefers-reduced-motion: reduce).
    pub reduce_motion: bool,
    /// Block ads (EasyList) and trackers (EasyPrivacy).
    pub block_ads: bool,
    pub block_trackers: bool,
    /// Hide cookie notices and similar nuisances (EasyList Cookie, uBlock Annoyances).
    pub block_annoyances: bool,
    /// Warn before opening sites on malware and phishing lists.
    pub safe_browsing: bool,
    /// Sites where ad and tracker blocking is turned off.
    pub no_blocking: Vec<String>,
    /// Give scripts the same answers most browsers give (screen size, fonts…) and add noise to
    /// canvas readouts, so sites can't single you out by your computer's details.
    pub resist_fingerprinting: bool,
    /// Per-site cookie rules (sites, e.g. example.com):
    /// always allowed (even when embedded in other sites)…
    pub cookies_allow: Vec<String>,
    /// …never stored or sent…
    pub cookies_block: Vec<String>,
    /// …or kept only until Ironswift closes.
    pub cookies_session: Vec<String>,
    /// Check for a new version of Ironswift once a day.
    pub check_updates: bool,
    /// The browser's language: "auto" (the desktop's) or a code from i18n::LANGUAGES.
    pub language: String,
    /// Draw pages and the browser in high-contrast colors.
    pub high_contrast: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            home: "about:home".into(),
            search: "duckduckgo".into(),
            restore_tabs: true,
            sleep_minutes: 10,
            third_party_cookies: false,
            strip_tracking: true,
            send_gpc: true,
            show_bookmarks_bar: true,
            use_gpu: true,
            site_processes: true,
            proxy_mode: "system".into(),
            proxy_url: String::new(),
            clear_on_exit: false,
            min_font_size: 0,
            reduce_motion: false,
            block_ads: true,
            block_trackers: true,
            block_annoyances: false,
            safe_browsing: true,
            no_blocking: Vec::new(),
            resist_fingerprinting: true,
            cookies_allow: Vec::new(),
            cookies_block: Vec::new(),
            cookies_session: Vec::new(),
            check_updates: true,
            language: "auto".into(),
            high_contrast: false,
        }
    }
}

/// (key, name, search URL with %s for the query)
pub const SEARCH_ENGINES: &[(&str, &str, &str)] = &[
    ("duckduckgo", "DuckDuckGo", "https://html.duckduckgo.com/html/?q=%s"),
    ("startpage", "Startpage", "https://www.startpage.com/do/search?q=%s"),
    ("brave", "Brave Search", "https://search.brave.com/search?q=%s"),
    ("google", "Google", "https://www.google.com/search?q=%s"),
    ("bing", "Bing", "https://www.bing.com/search?q=%s"),
];

/// Turn ad and tracker blocking on or off for a site (the "Turn off blocking on these sites"
/// list), from the shield menu in the toolbar.
pub fn set_no_blocking(list: Vec<String>) {
    let mut s = get();
    s.no_blocking = list;
    if let Ok(mut w) = store().write() {
        *w = s.clone();
    }
    save(&s);
}

fn store() -> &'static RwLock<Settings> {
    static S: OnceLock<RwLock<Settings>> = OnceLock::new();
    S.get_or_init(|| RwLock::new(load()))
}

fn path() -> Option<std::path::PathBuf> {
    crate::cookies::profile_dir().map(|d| d.join("settings.tsv"))
}

fn load() -> Settings {
    let mut s = Settings::default();
    let text = path().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    for line in text.lines() {
        if let Some((k, v)) = line.split_once('\t') {
            apply(&mut s, k, v);
        }
    }
    s
}

fn apply(s: &mut Settings, k: &str, v: &str) {
    let on = |v: &str| matches!(v, "1" | "on" | "true" | "yes");
    match k {
        "home" if !v.trim().is_empty() => s.home = v.trim().to_string(),
        "search" if SEARCH_ENGINES.iter().any(|e| e.0 == v) => s.search = v.to_string(),
        "restore_tabs" => s.restore_tabs = on(v),
        "sleep_minutes" => s.sleep_minutes = v.trim().parse().unwrap_or(s.sleep_minutes).min(24 * 60),
        "third_party_cookies" => s.third_party_cookies = on(v),
        "strip_tracking" => s.strip_tracking = on(v),
        "send_gpc" => s.send_gpc = on(v),
        "show_bookmarks_bar" => s.show_bookmarks_bar = on(v),
        "use_gpu" => s.use_gpu = on(v),
        // Saved under a new name since site processes became the default: the old key was
        // written as off by every profile that never chose.
        "site_isolation" => s.site_processes = on(v),
        "proxy_mode" if matches!(v, "system" | "none" | "manual") => s.proxy_mode = v.to_string(),
        "proxy_url" => s.proxy_url = v.trim().replace(['\t', '\n'], ""),
        "clear_on_exit" => s.clear_on_exit = on(v),
        "reduce_motion" => s.reduce_motion = on(v),
        "min_font_size" => s.min_font_size = v.trim().parse().unwrap_or(s.min_font_size).min(40),
        "block_ads" => s.block_ads = on(v),
        "block_trackers" => s.block_trackers = on(v),
        "block_annoyances" => s.block_annoyances = on(v),
        "safe_browsing" => s.safe_browsing = on(v),
        "resist_fingerprinting" => s.resist_fingerprinting = on(v),
        "check_updates" => s.check_updates = on(v),
        "language" if v == "auto" || crate::i18n::LANGUAGES.iter().any(|l| l.0 == v) => s.language = v.to_string(),
        "high_contrast" => s.high_contrast = on(v),
        "no_blocking" => s.no_blocking = site_list(v),
        "cookies_allow" => s.cookies_allow = site_list(v),
        "cookies_block" => s.cookies_block = site_list(v),
        "cookies_session" => s.cookies_session = site_list(v),
        _ => {}
    }
}

/// "example.com, https://www.news.org/page" -> ["example.com", "www.news.org"]
fn site_list(v: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in v.split(|c: char| c.is_whitespace() || c == ',' || c == ';').filter(|x| !x.is_empty()) {
        let host = match item.split_once("://") {
            Some((_, rest)) => rest,
            None => item,
        };
        let host = host.split(['/', '?', '#', ':']).next().unwrap_or("").trim_start_matches("*.").trim_start_matches('.').to_ascii_lowercase();
        if host.contains('.') || host == "localhost" {
            if !out.contains(&host) {
                out.push(host);
            }
        }
    }
    out
}

/// Add or remove a site from one of the per-site lists, and save.
pub fn set_site_rule(list: &str, site: &str, add: bool) {
    let Ok(mut w) = store().write() else { return };
    let mut s = w.clone();
    for (name, l) in [("no_blocking", &mut s.no_blocking), ("cookies_allow", &mut s.cookies_allow), ("cookies_block", &mut s.cookies_block), ("cookies_session", &mut s.cookies_session)] {
        // A site has one cookie rule at a time.
        if name == list || (list.starts_with("cookies_") && name.starts_with("cookies_")) {
            l.retain(|x| x != site);
        }
        if name == list && add {
            l.push(site.to_string());
        }
    }
    *w = s.clone();
    drop(w);
    save(&s);
}

pub fn get() -> Settings {
    store().read().map(|s| s.clone()).unwrap_or_default()
}

fn save(s: &Settings) {
    let Some(p) = path() else { return };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let b = |v: bool| if v { "1" } else { "0" };
    let mut body = format!(
        "home\t{}\nsearch\t{}\nrestore_tabs\t{}\nsleep_minutes\t{}\nthird_party_cookies\t{}\nstrip_tracking\t{}\nsend_gpc\t{}\nshow_bookmarks_bar\t{}\nuse_gpu\t{}\nproxy_mode\t{}\nproxy_url\t{}\nclear_on_exit\t{}\nmin_font_size\t{}\nreduce_motion\t{}\n",
        s.home.replace(['\t', '\n'], " "),
        s.search,
        b(s.restore_tabs),
        s.sleep_minutes,
        b(s.third_party_cookies),
        b(s.strip_tracking),
        b(s.send_gpc),
        b(s.show_bookmarks_bar),
        b(s.use_gpu),
        s.proxy_mode,
        s.proxy_url,
        b(s.clear_on_exit),
        s.min_font_size,
        b(s.reduce_motion)
    );
    body.push_str(&format!(
        "block_ads\t{}\nblock_trackers\t{}\nblock_annoyances\t{}\nsafe_browsing\t{}\nresist_fingerprinting\t{}\ncheck_updates\t{}\nno_blocking\t{}\ncookies_allow\t{}\ncookies_block\t{}\ncookies_session\t{}\n",
        b(s.block_ads),
        b(s.block_trackers),
        b(s.block_annoyances),
        b(s.safe_browsing),
        b(s.resist_fingerprinting),
        b(s.check_updates),
        s.no_blocking.join(" "),
        s.cookies_allow.join(" "),
        s.cookies_block.join(" "),
        s.cookies_session.join(" ")
    ));
    body.push_str(&format!("language\t{}\nhigh_contrast\t{}\n", s.language, b(s.high_contrast)));
    body.push_str(&format!("site_isolation\t{}\n", b(s.site_processes)));
    let tmp = p.with_extension("tmp");
    if std::fs::write(&tmp, body).is_ok() {
        let _ = std::fs::rename(&tmp, &p);
    }
}

/// Delete cookies, site storage and cached files (and with `everything`, history, per-site
/// zoom and the HTTPS-only list too). Returns what was cleared, for the page to say.
pub fn clear_data(everything: bool) -> String {
    crate::cookies::clear();
    if let Some(d) = crate::cookies::profile_dir() {
        let _ = std::fs::remove_dir_all(d.join("storage"));
        let _ = std::fs::remove_dir_all(d.join("indexeddb"));
    }
    let freed = crate::cache::clear();
    if everything {
        crate::library::clear_history();
        crate::zoom::clear();
        crate::hsts::clear();
        format!("History, cookies, site data and cached files cleared ({:.1} MB freed). Bookmarks and settings are kept.", freed as f64 / 1_048_576.0)
    } else {
        format!("Cookies, site data and cached files cleared ({:.1} MB freed).", freed as f64 / 1_048_576.0)
    }
}

/// Make Ironswift the desktop's default web browser (needs the desktop entry that
/// packaging/linux/install.sh puts in place).
fn make_default() -> String {
    match crate::platform::make_default_browser() {
        Ok(msg) => msg,
        Err(why) => format!("Couldn't set the default browser: {why}."),
    }
}

/// One row per site with cookies: its rule and links to change it.
fn cookie_rows(s: &Settings) -> String {
    let rule = |site: &str| {
        let has = |l: &[String]| l.iter().any(|d| d == site);
        if has(&s.cookies_block) {
            "blocked"
        } else if has(&s.cookies_allow) {
            "always allowed"
        } else if has(&s.cookies_session) {
            "deleted on close"
        } else {
            ""
        }
    };
    crate::cookies::sites()
        .iter()
        .take(300)
        .map(|(site, n)| {
            let e = esc(site);
            let link = |what: &str, label: &str| format!("<a href=\"about:settings?cookie_site={e}&do={what}\">{label}</a>");
            format!(
                "<tr><td><b>{e}</b></td><td>{n} cookie{}</td><td>{}</td><td>{} · {} · {} · {} · {}</td></tr>",
                if *n == 1 { "" } else { "s" },
                rule(site),
                link("delete", "Delete"),
                link("allow", "Always allow"),
                link("block", "Block"),
                link("session", "Delete on close"),
                link("default", "Normal")
            )
        })
        .collect()
}

/// The search URL for a query, with the chosen engine.
pub fn search_url(query_encoded: &str) -> String {
    let key = get().search;
    let tpl = SEARCH_ENGINES.iter().find(|e| e.0 == key).map_or(SEARCH_ENGINES[0].2, |e| e.2);
    tpl.replace("%s", query_encoded)
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// about:settings. A submitted form arrives as the query (`?save=1&home=…`); checkboxes that
/// are off simply aren't in it.
pub fn page(query: &str) -> String {
    let mut saved = false;
    let mut cleared = String::new();
    match query.trim_start_matches('?') {
        "clear_cache" => {
            let freed = crate::cache::clear();
            cleared = format!("<p class=\"saved\">{}</p>", esc(&crate::i18n::tf("settings-cache-cleared", &[("mb", &format!("{:.1}", freed as f64 / 1_048_576.0))])));
        }
        "clear_data" => cleared = format!("<p class=\"saved\">{}</p>", clear_data(true)),
        "default_browser" => cleared = format!("<p class=\"saved\">{}</p>", make_default()),
        "update_lists" => cleared = format!("<p class=\"saved\">{}</p>", esc(&crate::blocklist::update_now(true))),
        "check_update" => cleared = format!("<p class=\"saved\">{}</p>", esc(&crate::updates::check_now())),
        q if q.starts_with("cookie_site=") => {
            // cookie_site=<site>&do=delete|allow|block|session|default
            let mut site = String::new();
            let mut what = String::new();
            for pair in q.split('&') {
                match pair.split_once('=') {
                    Some(("cookie_site", v)) => site = crate::net::percent_decode_str(v).to_ascii_lowercase(),
                    Some(("do", v)) => what = v.to_string(),
                    _ => {}
                }
            }
            let msg = match what.as_str() {
                "delete" => {
                    crate::cookies::clear_site(&site);
                    crate::i18n::tf("settings-cookies-deleted", &[("site", &site)])
                }
                "allow" => {
                    set_site_rule("cookies_allow", &site, true);
                    crate::i18n::tf("settings-cookies-allowed", &[("site", &site)])
                }
                "block" => {
                    set_site_rule("cookies_block", &site, true);
                    crate::cookies::clear_site(&site);
                    crate::i18n::tf("settings-cookies-blocked", &[("site", &site)])
                }
                "session" => {
                    set_site_rule("cookies_session", &site, true);
                    crate::i18n::tf("settings-cookies-session-set", &[("site", &site)])
                }
                _ => {
                    set_site_rule("cookies_allow", &site, false);
                    crate::i18n::tf("settings-cookies-default", &[("site", &site)])
                }
            };
            if !site.is_empty() {
                cleared = format!("<p class=\"saved\">{}</p>", esc(&msg));
            }
        }
        q if q.starts_with("forget_permission=") => {
            let site = crate::net::percent_decode_str(&q["forget_permission=".len()..]);
            crate::permissions::forget(&site);
            crate::process::tell_permission(&site, None, false);
            cleared = format!("<p class=\"saved\">{}</p>", esc(&crate::i18n::tf("settings-permissions-reset", &[("site", &site)])));
        }
        _ => {}
    }
    if query.contains("save=1") {
        let mut s = get();
        // Unchecked boxes are absent from a form submission.
        s.restore_tabs = false;
        s.third_party_cookies = false;
        s.strip_tracking = false;
        s.send_gpc = false;
        s.show_bookmarks_bar = false;
        s.use_gpu = false;
        s.site_processes = false;
        s.clear_on_exit = false;
        s.reduce_motion = false;
        s.block_ads = false;
        s.block_trackers = false;
        s.block_annoyances = false;
        s.safe_browsing = false;
        s.resist_fingerprinting = false;
        s.check_updates = false;
        s.high_contrast = false;
        for pair in query.trim_start_matches('?').split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                let v = crate::net::percent_decode_str(&v.replace('+', " "));
                apply(&mut s, k, &v);
            }
        }
        if let Ok(mut w) = store().write() {
            *w = s.clone();
        }
        save(&s);
        crate::blocklist::reload();
        crate::i18n::set_language(&s.language);
        saved = true;
    }
    let s = get();
    let check = |v: bool| if v { " checked" } else { "" };
    let engines: String = SEARCH_ENGINES
        .iter()
        .map(|(k, name, _)| format!("<option value=\"{k}\"{}>{name}</option>", if *k == s.search { " selected" } else { "" }))
        .collect();
    fill_messages(&format!(
        r#"<!DOCTYPE html><html lang="{lang}" dir="{dir}"><head><title>@@settings-title@@</title><style>
body{{font-family:sans-serif;margin:32px auto;max-width:720px;padding:0 20px;color:#1f2937}}
h1{{margin-bottom:4px}} .sub{{color:#64748b;margin-top:0}}
section{{border:1px solid #e5e7eb;border-radius:10px;padding:14px 18px;margin:16px 0}}
h2{{font-size:17px;margin:0 0 10px}} .row{{margin:10px 0}} label{{display:block}}
.hint{{color:#64748b;font-size:13px;margin:2px 0 0 24px}} .field label{{margin-bottom:4px;font-weight:600}}
input[type=text],input[type=number],select{{font-size:14px;padding:5px 8px}} input[type=text]{{width:420px}}
.saved{{background:#dcfce7;color:#166534;padding:8px 12px;border-radius:8px}}
table.sites{{border-collapse:collapse;margin:8px 0;font-size:13px}} table.sites td{{padding:3px 10px 3px 0}}
button{{background:#4f46e5;color:white;border:none;border-radius:8px;padding:8px 18px;font-size:15px}}
</style></head><body>
<h1>@@settings-title@@</h1><p class="sub">@@settings-sub@@</p>{saved}{cleared}
<form action="about:settings" method="get"><input type="hidden" name="save" value="1">
<section><h2>@@settings-startup@@</h2>
<div class="row field"><label>@@settings-home@@</label><input type="text" name="home" value="{home}"></div>
<div class="row field"><label>@@settings-search-engine@@</label><select name="search">{engines}</select></div>
<div class="row"><label><input type="checkbox" name="restore_tabs" value="1"{restore}> @@settings-restore-tabs@@</label></div>
<div class="row"><label><input type="checkbox" name="show_bookmarks_bar" value="1"{bmbar}> @@settings-bookmarks-bar@@</label></div>
</section>
<section><h2>@@settings-memory@@</h2>
<div class="row field"><label>@@settings-sleep@@</label><input type="number" name="sleep_minutes" value="{sleep}" min="0" max="1440"></div>
<p class="hint" style="margin-left:0">@@settings-sleep-hint@@</p>
<div class="row"><label><input type="checkbox" name="use_gpu" value="1"{gpu_on}> @@settings-gpu@@</label><p class="hint">@@settings-gpu-hint@@</p></div>
<div class="row"><label><input type="checkbox" name="site_isolation" value="1"{site_proc}> @@settings-site-processes@@</label><p class="hint">@@settings-site-processes-hint@@</p></div>
</section>
<section><h2>@@settings-privacy@@</h2>
<div class="row"><label><input type="checkbox" name="third_party_cookies" value="1"{tpc}> @@settings-tpc@@</label><p class="hint">@@settings-tpc-hint@@</p></div>
<div class="row"><label><input type="checkbox" name="strip_tracking" value="1"{strip}> @@settings-strip@@</label><p class="hint">@@settings-strip-hint@@</p></div>
<div class="row"><label><input type="checkbox" name="send_gpc" value="1"{gpc}> @@settings-gpc@@</label><p class="hint">@@settings-gpc-hint@@</p></div>
<div class="row"><label><input type="checkbox" name="clear_on_exit" value="1"{clear_exit}> @@settings-clear-exit@@</label><p class="hint">@@settings-clear-exit-hint@@</p></div>
<div class="row field"><label>@@settings-cookies-allow@@</label><textarea name="cookies_allow" rows="2" cols="60" placeholder="example.com">{ck_allow}</textarea><p class="hint" style="margin-left:0">@@settings-cookies-allow-hint@@</p></div>
<div class="row field"><label>@@settings-cookies-block@@</label><textarea name="cookies_block" rows="2" cols="60">{ck_block}</textarea></div>
<div class="row field"><label>@@settings-cookies-session@@</label><textarea name="cookies_session" rows="2" cols="60">{ck_session}</textarea></div>
<p class="hint" style="margin-left:0">@@settings-partition-hint@@</p>
<details><summary>{cookie_sites}</summary><table class="sites">{cookie_rows}</table></details>
<div class="row"><a href="about:settings?clear_data">@@settings-clear-data@@</a> <span class="hint">@@settings-clear-data-hint@@</span></div>
<p class="hint" style="margin-left:0">@@settings-private-hint@@</p>
</section>
<section><h2>@@settings-blocking@@</h2>
<div class="row"><label><input type="checkbox" name="block_ads" value="1"{blk_ads}> @@settings-block-ads@@</label><p class="hint">@@settings-block-ads-hint@@</p></div>
<div class="row"><label><input type="checkbox" name="block_trackers" value="1"{blk_trk}> @@settings-block-trackers@@</label><p class="hint">@@settings-block-trackers-hint@@</p></div>
<div class="row"><label><input type="checkbox" name="block_annoyances" value="1"{blk_ann}> @@settings-block-annoyances@@</label><p class="hint">@@settings-block-annoyances-hint@@</p></div>
<div class="row field"><label>@@settings-no-blocking@@</label><textarea name="no_blocking" rows="2" cols="60" placeholder="example.com">{no_blk}</textarea></div>
<div class="row"><label><input type="checkbox" name="safe_browsing" value="1"{safe}> @@settings-safe@@</label><p class="hint">@@settings-safe-hint@@</p></div>
<div class="row"><label><input type="checkbox" name="resist_fingerprinting" value="1"{rfp}> @@settings-rfp@@</label><p class="hint">@@settings-rfp-hint@@</p></div>
<p class="hint" style="margin-left:0">{lists} · <a href="about:settings?update_lists">@@settings-update-lists@@</a></p>
<p class="hint" style="margin-left:0">@@settings-permissions-hint@@ {perms}</p>
</section>
<section><h2>@@settings-updates@@</h2>
<div class="row"><label><input type="checkbox" name="check_updates" value="1"{chk_upd}> @@settings-check-updates@@</label><p class="hint">{upd_status}</p></div>
<div class="row"><a href="about:settings?check_update">@@settings-check-now@@</a></div>
</section>
<section><h2>@@settings-appearance@@</h2>
<div class="row field"><label>@@settings-min-font@@</label><input type="number" name="min_font_size" value="{min_font}" min="0" max="40"></div>
<p class="hint" style="margin-left:0">@@settings-min-font-hint@@</p>
<div class="row"><label><input type="checkbox" name="reduce_motion" value="1"{reduce}> @@settings-reduce-motion@@</label><p class="hint">@@settings-reduce-motion-hint@@</p></div>
<div class="row"><label><input type="checkbox" name="high_contrast" value="1"{hc}> {hc_label}</label><p class="hint">{hc_hint}</p></div>
<div class="row field"><label>{lang_label}</label><select name="language">{lang_opts}</select><p class="hint" style="margin-left:0">{lang_hint}</p></div>
</section>
<section><h2>@@settings-default-browser@@</h2>
<div class="row"><a href="about:settings?default_browser">@@settings-make-default@@</a></div>
</section>
<section><h2>@@settings-network@@</h2>
<div class="row field"><label>@@settings-proxy@@</label><select name="proxy_mode">{proxy_opts}</select></div>
<div class="row field"><label>@@settings-proxy-address@@</label><input type="text" name="proxy_url" value="{proxy_url}" placeholder="http://proxy.example.com:8080 or socks5://127.0.0.1:1080"></div>
<p class="hint" style="margin-left:0">@@settings-proxy-hint@@</p>
<div class="row"><a href="about:settings?clear_cache">@@settings-clear-cache@@</a> <span class="hint">{cache_size}</span></div>
</section>
<button type="submit">@@settings-save@@</button></form>
<p class="sub" style="margin-top:20px"><a href="about:history">@@settings-history@@</a> · <a href="about:bookmarks">@@settings-bookmarks@@</a></p>
</body></html>"#,
        saved = if saved { "<p class=\"saved\">@@settings-saved@@</p>" } else { "" },
        cleared = cleared,
        proxy_opts = [("system", "@@settings-proxy-system@@"), ("none", "@@settings-proxy-none@@"), ("manual", "@@settings-proxy-manual@@")]
            .iter()
            .map(|(k, n)| format!("<option value=\"{k}\"{}>{n}</option>", if *k == s.proxy_mode { " selected" } else { "" }))
            .collect::<String>(),
        proxy_url = esc(&s.proxy_url),
        cache_size = esc(&crate::i18n::tf("settings-cache-size", &[("mb", &format!("{:.1}", crate::cache::size() as f64 / 1_048_576.0))])),
        home = esc(&s.home),
        engines = engines,
        restore = check(s.restore_tabs),
        sleep = s.sleep_minutes,
        tpc = check(s.third_party_cookies),
        clear_exit = check(s.clear_on_exit),
        min_font = s.min_font_size,
        reduce = check(s.reduce_motion),
        strip = check(s.strip_tracking),
        gpc = check(s.send_gpc),
        bmbar = check(s.show_bookmarks_bar),
        gpu_on = check(s.use_gpu),
        site_proc = check(s.site_processes),
        cookie_sites = esc(&crate::i18n::tf("settings-cookie-sites", &[("count", &crate::cookies::sites().len().to_string())])),
        cookie_rows = cookie_rows(&s),
        ck_allow = esc(&s.cookies_allow.join("\n")),
        ck_block = esc(&s.cookies_block.join("\n")),
        ck_session = esc(&s.cookies_session.join("\n")),
        blk_ads = check(s.block_ads),
        blk_trk = check(s.block_trackers),
        blk_ann = check(s.block_annoyances),
        no_blk = esc(&s.no_blocking.join("\n")),
        safe = check(s.safe_browsing),
        rfp = check(s.resist_fingerprinting),
        lists = esc(&crate::blocklist::status()),
        perms = crate::permissions::settings_html(),
        chk_upd = check(s.check_updates),
        upd_status = esc(&crate::updates::status()),
        lang = esc(&crate::i18n::locale()),
        dir = if crate::i18n::rtl() { "rtl" } else { "ltr" },
        hc = check(s.high_contrast),
        hc_label = esc(&crate::i18n::t("set-high-contrast")),
        hc_hint = esc(&crate::i18n::t("set-high-contrast-hint")),
        lang_label = esc(&crate::i18n::t("set-language")),
        lang_hint = esc(&crate::i18n::t("set-language-hint")),
        lang_opts = std::iter::once(("auto".to_string(), crate::i18n::t("set-language-auto")))
            .chain(crate::i18n::LANGUAGES.iter().map(|l| (l.0.to_string(), l.1.to_string())))
            .map(|(k, n)| format!("<option value=\"{k}\"{}>{}</option>", if k == s.language { " selected" } else { "" }, esc(&n)))
            .collect::<String>(),
    ))
}

/// Replace each `@@message-id@@` with that UI message in the current language.
fn fill_messages(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(i) = rest.find("@@") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        match after.find("@@") {
            Some(j) if after[..j].bytes().all(|b| b.is_ascii_lowercase() || b == b'-') => {
                out.push_str(&esc(&crate::i18n::t(&after[..j])));
                rest = &after[j + 2..];
            }
            _ => {
                out.push_str("@@");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}
