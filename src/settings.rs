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
            proxy_mode: "system".into(),
            proxy_url: String::new(),
            clear_on_exit: false,
            min_font_size: 0,
            reduce_motion: false,
            block_ads: true,
            block_trackers: true,
            safe_browsing: true,
            no_blocking: Vec::new(),
            resist_fingerprinting: true,
            cookies_allow: Vec::new(),
            cookies_block: Vec::new(),
            cookies_session: Vec::new(),
            check_updates: true,
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
        "proxy_mode" if matches!(v, "system" | "none" | "manual") => s.proxy_mode = v.to_string(),
        "proxy_url" => s.proxy_url = v.trim().replace(['\t', '\n'], ""),
        "clear_on_exit" => s.clear_on_exit = on(v),
        "reduce_motion" => s.reduce_motion = on(v),
        "min_font_size" => s.min_font_size = v.trim().parse().unwrap_or(s.min_font_size).min(40),
        "block_ads" => s.block_ads = on(v),
        "block_trackers" => s.block_trackers = on(v),
        "safe_browsing" => s.safe_browsing = on(v),
        "resist_fingerprinting" => s.resist_fingerprinting = on(v),
        "check_updates" => s.check_updates = on(v),
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
        "block_ads\t{}\nblock_trackers\t{}\nsafe_browsing\t{}\nresist_fingerprinting\t{}\ncheck_updates\t{}\nno_blocking\t{}\ncookies_allow\t{}\ncookies_block\t{}\ncookies_session\t{}\n",
        b(s.block_ads),
        b(s.block_trackers),
        b(s.safe_browsing),
        b(s.resist_fingerprinting),
        b(s.check_updates),
        s.no_blocking.join(" "),
        s.cookies_allow.join(" "),
        s.cookies_block.join(" "),
        s.cookies_session.join(" ")
    ));
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
    let ok = std::process::Command::new("xdg-settings").args(["set", "default-web-browser", "ironswift.desktop"]).output().is_ok_and(|o| o.status.success());
    if ok {
        "Ironswift is now your default browser.".into()
    } else {
        "Couldn't set the default browser: install Ironswift first (packaging/linux/install.sh), then try again.".into()
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
            cleared = format!("<p class=\"saved\">Cached files cleared ({:.1} MB freed).</p>", freed as f64 / 1_048_576.0);
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
                    format!("Cookies from {site} deleted.")
                }
                "allow" => {
                    set_site_rule("cookies_allow", &site, true);
                    format!("{site} may always use cookies.")
                }
                "block" => {
                    set_site_rule("cookies_block", &site, true);
                    crate::cookies::clear_site(&site);
                    format!("{site} can't use cookies any more (its cookies were deleted).")
                }
                "session" => {
                    set_site_rule("cookies_session", &site, true);
                    format!("{site}'s cookies will be deleted when Ironswift closes.")
                }
                _ => {
                    set_site_rule("cookies_allow", &site, false);
                    format!("{site} uses the normal cookie rules.")
                }
            };
            if !site.is_empty() {
                cleared = format!("<p class=\"saved\">{}</p>", esc(&msg));
            }
        }
        q if q.starts_with("forget_permission=") => {
            let site = crate::net::percent_decode_str(&q["forget_permission=".len()..]);
            crate::permissions::forget(&site);
            cleared = format!("<p class=\"saved\">Permissions for {} reset.</p>", esc(&site));
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
        s.clear_on_exit = false;
        s.reduce_motion = false;
        s.block_ads = false;
        s.block_trackers = false;
        s.safe_browsing = false;
        s.resist_fingerprinting = false;
        s.check_updates = false;
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
        saved = true;
    }
    let s = get();
    let check = |v: bool| if v { " checked" } else { "" };
    let engines: String = SEARCH_ENGINES
        .iter()
        .map(|(k, name, _)| format!("<option value=\"{k}\"{}>{name}</option>", if *k == s.search { " selected" } else { "" }))
        .collect();
    format!(
        r#"<!DOCTYPE html><html><head><title>Settings</title><style>
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
<h1>Settings</h1><p class="sub">Saved on this computer only.</p>{saved}{cleared}
<form action="about:settings" method="get"><input type="hidden" name="save" value="1">
<section><h2>Startup and search</h2>
<div class="row field"><label>Home page</label><input type="text" name="home" value="{home}"></div>
<div class="row field"><label>Search engine</label><select name="search">{engines}</select></div>
<div class="row"><label><input type="checkbox" name="restore_tabs" value="1"{restore}> Reopen the tabs from last time</label></div>
<div class="row"><label><input type="checkbox" name="show_bookmarks_bar" value="1"{bmbar}> Show the bookmarks bar</label></div>
</section>
<section><h2>Memory</h2>
<div class="row field"><label>Put background tabs to sleep after (minutes, 0 = never)</label><input type="number" name="sleep_minutes" value="{sleep}" min="0" max="1440"></div>
<p class="hint" style="margin-left:0">A sleeping tab frees its page and reloads when you come back to it.</p>
<div class="row"><label><input type="checkbox" name="use_gpu" value="1"{gpu_on}> Draw with the graphics card</label><p class="hint">Smoother scrolling and less work for the processor; the graphics driver uses about 40–70 MB more memory. Takes effect the next time Ironswift starts.</p></div>
</section>
<section><h2>Privacy</h2>
<div class="row"><label><input type="checkbox" name="third_party_cookies" value="1"{tpc}> Share cookies with embedded sites</label><p class="hint">Off by default: a site embedded in other sites (an ad or tracking network, a video player) gets a separate cookie jar on each site, so it can't follow you around.</p></div>
<div class="row"><label><input type="checkbox" name="strip_tracking" value="1"{strip}> Remove tracking tags from links</label><p class="hint">utm_source, fbclid, gclid and similar.</p></div>
<div class="row"><label><input type="checkbox" name="send_gpc" value="1"{gpc}> Send Global Privacy Control</label><p class="hint">Asks sites not to sell or share your data.</p></div>
<div class="row"><label><input type="checkbox" name="clear_on_exit" value="1"{clear_exit}> Delete cookies and site data when Ironswift closes</label><p class="hint">Signs you out of sites each time; history and bookmarks are kept.</p></div>
<div class="row field"><label>Always allow cookies from these sites</label><textarea name="cookies_allow" rows="2" cols="60" placeholder="example.com">{ck_allow}</textarea><p class="hint" style="margin-left:0">Even when they're embedded in other sites (for sign-in or comment boxes that need them).</p></div>
<div class="row field"><label>Never allow cookies from these sites</label><textarea name="cookies_block" rows="2" cols="60">{ck_block}</textarea></div>
<div class="row field"><label>Delete these sites' cookies when Ironswift closes</label><textarea name="cookies_session" rows="2" cols="60">{ck_session}</textarea></div>
<p class="hint" style="margin-left:0">Other sites embedded in a page get their own separate cookie jar for that page, so they can't follow you from site to site.</p>
<details><summary>Sites with cookies ({cookie_count})</summary><table class="sites">{cookie_rows}</table></details>
<div class="row"><a href="about:settings?clear_data">Clear browsing data now</a> <span class="hint">History, cookies, site data and cached files. Bookmarks and settings stay.</span></div>
<p class="hint" style="margin-left:0">For browsing that keeps nothing at all, open a private window with Ctrl+Shift+N.</p>
</section>
<section><h2>Blocking and safety</h2>
<div class="row"><label><input type="checkbox" name="block_ads" value="1"{blk_ads}> Block ads</label><p class="hint">EasyList: ad requests are stopped and empty ad boxes hidden. Pages load faster.</p></div>
<div class="row"><label><input type="checkbox" name="block_trackers" value="1"{blk_trk}> Block trackers</label><p class="hint">EasyPrivacy: analytics, tracking pixels and fingerprinting scripts.</p></div>
<div class="row field"><label>Turn blocking off on these sites</label><textarea name="no_blocking" rows="2" cols="60" placeholder="example.com">{no_blk}</textarea></div>
<div class="row"><label><input type="checkbox" name="safe_browsing" value="1"{safe}> Warn me before dangerous sites</label><p class="hint">Sites on the URLhaus malware list and the Phishing Army list. The check happens on this computer; nothing is sent anywhere.</p></div>
<div class="row"><label><input type="checkbox" name="resist_fingerprinting" value="1"{rfp}> Resist fingerprinting</label><p class="hint">Sites see a common screen size, the standard fonts and slightly varied canvas readouts, so they can't recognize your computer by its details.</p></div>
<p class="hint" style="margin-left:0">{lists} · <a href="about:settings?update_lists">Update lists now</a></p>
<p class="hint" style="margin-left:0">Camera, microphone, location and notifications: sites ask first. {perms}</p>
</section>
<section><h2>Updates</h2>
<div class="row"><label><input type="checkbox" name="check_updates" value="1"{chk_upd}> Check for new versions once a day</label><p class="hint">{upd_status}</p></div>
<div class="row"><a href="about:settings?check_update">Check now</a></div>
</section>
<section><h2>Appearance</h2>
<div class="row field"><label>Minimum font size (px, 0 = none)</label><input type="number" name="min_font_size" value="{min_font}" min="0" max="40"></div>
<p class="hint" style="margin-left:0">Makes small print on pages larger. Ctrl + and Ctrl − zoom a whole site.</p>
<div class="row"><label><input type="checkbox" name="reduce_motion" value="1"{reduce}> Reduce motion</label><p class="hint">Asks sites to keep animations to a minimum.</p></div>
</section>
<section><h2>Default browser</h2>
<div class="row"><a href="about:settings?default_browser">Make Ironswift my default browser</a></div>
</section>
<section><h2>Network</h2>
<div class="row field"><label>Proxy</label><select name="proxy_mode">{proxy_opts}</select></div>
<div class="row field"><label>Proxy address (for "Use this proxy")</label><input type="text" name="proxy_url" value="{proxy_url}" placeholder="http://proxy.example.com:8080 or socks5://127.0.0.1:1080"></div>
<p class="hint" style="margin-left:0">"System settings" follows your desktop's proxy and the http_proxy / https_proxy / no_proxy variables. Secure sites are checked against your computer's certificate store.</p>
<div class="row"><a href="about:settings?clear_cache">Clear cached files</a> <span class="hint">({cache_mb} MB saved so pages load faster)</span></div>
</section>
<button type="submit">Save settings</button></form>
<p class="sub" style="margin-top:20px"><a href="about:history">History</a> · <a href="about:bookmarks">Bookmarks</a></p>
</body></html>"#,
        saved = if saved { "<p class=\"saved\">Settings saved.</p>" } else { "" },
        cleared = cleared,
        proxy_opts = [("system", "System settings"), ("none", "No proxy"), ("manual", "Use this proxy")]
            .iter()
            .map(|(k, n)| format!("<option value=\"{k}\"{}>{n}</option>", if *k == s.proxy_mode { " selected" } else { "" }))
            .collect::<String>(),
        proxy_url = esc(&s.proxy_url),
        cache_mb = format!("{:.1}", crate::cache::size() as f64 / 1_048_576.0),
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
        cookie_count = crate::cookies::sites().len(),
        cookie_rows = cookie_rows(&s),
        ck_allow = esc(&s.cookies_allow.join("\n")),
        ck_block = esc(&s.cookies_block.join("\n")),
        ck_session = esc(&s.cookies_session.join("\n")),
        blk_ads = check(s.block_ads),
        blk_trk = check(s.block_trackers),
        no_blk = esc(&s.no_blocking.join("\n")),
        safe = check(s.safe_browsing),
        rfp = check(s.resist_fingerprinting),
        lists = esc(&crate::blocklist::status()),
        perms = crate::permissions::settings_html(),
        chk_upd = check(s.check_updates),
        upd_status = esc(&crate::updates::status()),
    )
}
