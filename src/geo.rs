//! Geolocation: `navigator.geolocation` positions from GeoClue, the desktop's location
//! service, over D-Bus (the system bus). Inside Flatpak, or when the system bus or GeoClue
//! can't be reached, the XDG Location portal (`org.freedesktop.portal.Location` on the session
//! bus) is asked instead: it asks the user and passes GeoClue's positions on.
//!
//! This runs in the browser process. Pages in site processes (which can't reach D-Bus) get
//! their positions relayed through it (geo_relay.rs).
//!
//! Pages only get here after the site was allowed to know your location (src/prelude.js asks
//! first). One background thread talks to GeoClue while any page is waiting for a position
//! or watching it, and stops the GeoClue client again when nobody is, so the location
//! hardware isn't kept busy. Results go back to the page that asked as script calls
//! (`__geoDone(id, position)` / `__geoFail(id, code, message)`), delivered by the page's
//! event loop like worker and socket events.
//!
//! `IRONSWIFT_FAKE_LOCATION="latitude,longitude[,accuracy]"` reports a fixed place instead of
//! asking GeoClue (for testing pages, like the location override in other browsers' tools).

use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use boa_engine::{Context, JsResult, JsString, JsValue, NativeFunction};
#[cfg(target_os = "linux")]
use zbus::blocking::Connection;
#[cfg(target_os = "linux")]
use zbus::zvariant::OwnedObjectPath;

#[cfg(target_os = "linux")]
const SERVICE: &str = "org.freedesktop.GeoClue2";
#[cfg(target_os = "linux")]
const MANAGER_PATH: &str = "/org/freedesktop/GeoClue2/Manager";
#[cfg(target_os = "linux")]
/// GeoClue matches this against its list of apps (our desktop file is ironswift.desktop).
const DESKTOP_ID: &str = "ironswift";
#[cfg(target_os = "linux")]
/// GeoClue accuracy levels: street level normally, exact (GPS) when a page asks for it.
const LEVEL_STREET: u32 = 6;
#[cfg(target_os = "linux")]
const LEVEL_EXACT: u32 = 8;
/// How often the thread looks for a new position and for pages that stopped waiting.
const POLL: Duration = Duration::from_millis(200);
#[cfg(target_os = "linux")]
/// GeoClue reports unknown altitude as -DBL_MAX and unknown speed/heading as -1.
const UNKNOWN_ALTITUDE: f64 = -1.0e300;

/// A position as GeoClue gives it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Position {
    pub latitude: f64,
    pub longitude: f64,
    /// Metres.
    pub accuracy: f64,
    pub altitude: Option<f64>,
    /// Metres per second.
    pub speed: Option<f64>,
    /// Degrees clockwise from north.
    pub heading: Option<f64>,
    /// Milliseconds since 1970.
    pub timestamp: f64,
}

impl Position {
    /// The `GeolocationPosition` object for scripts, as JSON.
    fn to_js(self) -> String {
        let opt = |v: Option<f64>| v.filter(|v| v.is_finite()).map_or("null".to_string(), |v| v.to_string());
        format!(
            "{{\"latitude\":{},\"longitude\":{},\"accuracy\":{},\"altitude\":{},\"altitudeAccuracy\":null,\"heading\":{},\"speed\":{},\"timestamp\":{}}}",
            self.latitude,
            self.longitude,
            self.accuracy,
            opt(self.altitude),
            opt(self.heading),
            opt(self.speed),
            self.timestamp.floor()
        )
    }
}

/// A page waiting for a position (`once`: getCurrentPosition) or watching it.
struct Want {
    epoch: u64,
    id: u64,
    once: bool,
    high: bool,
    /// Has it had a position yet? (A page that starts watching gets the current one.)
    got: bool,
}

struct State {
    wants: Vec<Want>,
    /// Is the thread talking to GeoClue running?
    running: bool,
    /// The last position and when it arrived (answers maximumAge).
    last: Option<(Position, Instant)>,
}

static STATE: Mutex<State> = Mutex::new(State { wants: Vec::new(), running: false, last: None });
/// Tests that use the shared request list run one at a time.
#[cfg(test)]
pub static TEST_SERIAL: Mutex<()> = Mutex::new(());
/// Script calls waiting for the page with this epoch.
static EVENTS: Mutex<Vec<(u64, String)>> = Mutex::new(Vec::new());

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn push(epoch: u64, code: String) {
    EVENTS.lock().unwrap_or_else(|e| e.into_inner()).push((epoch, code));
}

pub(crate) fn fail_code(id: u64, code: u32, msg: &str) -> String {
    let mut s = format!("__geoFail({id}, {code}, ");
    crate::js::json_str(msg, &mut s);
    s.push(')');
    s
}

/// Add the natives to a page's script engine.
pub fn install(ctx: &mut Context) {
    let natives: &[(&str, usize, fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>)] =
        &[("__geoStart", 4, n_geo_start), ("__geoStop", 1, n_geo_stop)];
    for (name, len, f) in natives {
        let _ = ctx.register_global_callable(JsString::from(*name), *len, NativeFunction::from_fn_ptr(*f));
    }
}

/// __geoStart(id, once, highAccuracy, maximumAge ms): start finding the position for a
/// request the user already allowed. The answer comes back through __geoDone / __geoFail.
fn n_geo_start(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = a.first().map(|v| v.to_number(ctx)).transpose()?.unwrap_or(0.0) as u64;
    let once = a.get(1).is_some_and(JsValue::to_boolean);
    let high = a.get(2).is_some_and(JsValue::to_boolean);
    let max_age = a.get(3).map(|v| v.to_number(ctx)).transpose()?.unwrap_or(0.0);
    // Only a page the user allowed can get here, and only that page itself (not a frame
    // inside it or a sandboxed document): check again rather than trust the script.
    let allowed = crate::js::page_ctx(ctx, |p| {
        !p.opaque() && crate::permissions::state(&p.url, &p.top, crate::permissions::Kind::Location) == "granted"
    });
    let epoch = crate::js::page_ctx(ctx, |p| p.epoch).unwrap_or(0);
    if allowed != Some(true) {
        push(epoch, fail_code(id, 1, "User denied Geolocation"));
        return Ok(JsValue::from(false));
    }
    if crate::process::in_renderer() {
        // A sandboxed site process: the browser finds the position (and checks the
        // permission again).
        let (url, top) = crate::js::page_ctx(ctx, |p| (p.url.clone(), p.top.clone())).unwrap_or_default();
        crate::geo_relay::start(epoch, id, once, high, max_age, &url, &top);
        return Ok(JsValue::from(true));
    }
    start(epoch, id, once, high, max_age);
    Ok(JsValue::from(true))
}

/// __geoStop(id): the page stopped waiting (clearWatch, or a timeout).
fn n_geo_stop(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = a.first().map(|v| v.to_number(ctx)).transpose()?.unwrap_or(0.0) as u64;
    let epoch = crate::js::page_ctx(ctx, |p| p.epoch).unwrap_or(0);
    if crate::process::in_renderer() {
        crate::geo_relay::stop(epoch, id);
    } else {
        stop(epoch, id);
    }
    Ok(JsValue::undefined())
}

/// Begin a request. A one-off request is answered from the last position when that is no
/// older than `max_age_ms`.
pub fn start(epoch: u64, id: u64, once: bool, high: bool, max_age_ms: f64) {
    let mut st = state();
    if once
        && let Some((pos, at)) = st.last
        && (at.elapsed().as_secs_f64() * 1000.0) <= max_age_ms
    {
        push(epoch, format!("__geoDone({id}, {})", pos.to_js()));
        return;
    }
    st.wants.retain(|w| !(w.epoch == epoch && w.id == id));
    st.wants.push(Want { epoch, id, once, high, got: false });
    if !st.running {
        st.running = true;
        drop(st);
        if std::thread::Builder::new().name("geolocation".into()).spawn(thread_main).is_err() {
            state().running = false;
            fail_all("The location service couldn't be started");
        }
    }
}

pub fn stop(epoch: u64, id: u64) {
    state().wants.retain(|w| !(w.epoch == epoch && w.id == id));
}

/// Queued script calls for the page with this epoch.
pub fn take_events(epoch: u64) -> Vec<String> {
    let mut q = EVENTS.lock().unwrap_or_else(|e| e.into_inner());
    let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut *q).into_iter().partition(|e| e.0 == epoch);
    *q = rest;
    mine.into_iter().map(|e| e.1).collect()
}

/// Is a page waiting for its first position, or are answers not yet delivered? (Keeps the
/// event loop polling, and a headless render waiting, like a network request.)
pub fn busy() -> bool {
    !EVENTS.lock().unwrap_or_else(|e| e.into_inner()).is_empty() || state().wants.iter().any(|w| !w.got) || crate::geo_relay::waiting()
}

/// Is anything still waiting for positions for `epoch`? (geo_relay.rs)
pub(crate) fn watching(epoch: u64) -> bool {
    state().wants.iter().any(|w| w.epoch == epoch)
}

/// The page is gone: forget its requests and undelivered answers.
pub fn page_gone(epoch: u64) {
    if crate::process::in_renderer() {
        crate::geo_relay::page_gone(epoch);
    }
    state().wants.retain(|w| w.epoch != epoch);
    EVENTS.lock().unwrap_or_else(|e| e.into_inner()).retain(|e| e.0 != epoch);
}

/// Hand a new position to everyone waiting (or, with `only_new`, the current one to those
/// who haven't had it yet); one-off requests are then finished.
fn deliver(pos: Position, only_new: bool) {
    let mut st = state();
    if !only_new {
        st.last = Some((pos, Instant::now()));
    }
    let js = pos.to_js();
    for w in st.wants.iter_mut().filter(|w| !(only_new && w.got)) {
        push(w.epoch, format!("__geoDone({}, {js})", w.id));
        w.got = true;
    }
    st.wants.retain(|w| !(w.once && w.got));
}

/// Tell everyone waiting that no position can be had.
fn fail_all(msg: &str) {
    let mut st = state();
    for w in st.wants.drain(..) {
        push(w.epoch, fail_code(w.id, 2, msg));
    }
}

/// Is anyone still waiting? If not, the thread is finished (decided under the same lock
/// `start` uses, so a request can't slip in between).
fn keep_going() -> Option<bool> {
    let mut st = state();
    if st.wants.is_empty() {
        st.running = false;
        None
    } else {
        Some(st.wants.iter().any(|w| w.high))
    }
}

fn thread_main() {
    if let Some(pos) = fake_position() {
        let pos = Position { timestamp: now_ms(), ..pos };
        deliver(pos, false);
        while keep_going().is_some() {
            deliver(pos, true);
            std::thread::sleep(POLL);
        }
        return;
    }
    #[cfg(target_os = "linux")]
    let result = {
        // Inside Flatpak the system bus is out of reach: go straight to the portal.
        let flatpak = std::path::Path::new("/.flatpak-info").exists();
        let geoclue = if flatpak { Err(Unavailable("inside Flatpak".into())) } else { Connection::system().map_err(|e| Unavailable(e.to_string())).and_then(|c| serve(&c)) };
        match geoclue {
            Ok(()) => Ok(()),
            Err(Unavailable(why)) => {
                log!("[geolocation] GeoClue can't be reached ({why}): asking the location portal");
                portal::run()
            }
            Err(Refused(msg)) => Err(msg),
        }
    };
    #[cfg(not(target_os = "linux"))]
    let result: Result<(), String> = Err("Location isn't available on this system yet".into());
    if let Err(msg) = result {
        log!("[geolocation] {msg}");
        // Fail everyone waiting and end the thread, under one lock so no request is left behind.
        let mut st = state();
        for w in st.wants.drain(..) {
            push(w.epoch, fail_code(w.id, 2, &msg));
        }
        st.running = false;
    }
}

fn fake_position() -> Option<Position> {
    let v = std::env::var("IRONSWIFT_FAKE_LOCATION").ok()?;
    let n: Vec<f64> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    let (&latitude, &longitude) = (n.first()?, n.get(1)?);
    Some(Position {
        latitude,
        longitude,
        accuracy: n.get(2).copied().unwrap_or(10.0),
        altitude: None,
        speed: None,
        heading: None,
        timestamp: now_ms(),
    })
}

fn now_ms() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64() * 1000.0)
}

#[cfg(target_os = "linux")]
fn proxy<'a>(conn: &Connection, path: &'a str, iface: &'a str) -> zbus::Result<zbus::blocking::Proxy<'a>> {
    zbus::blocking::proxy::Builder::<zbus::blocking::Proxy<'a>>::new(conn)
        .destination(SERVICE)?
        .path(path)?
        .interface(iface)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
}

#[cfg(target_os = "linux")]
/// A GeoClue client for this browser, started at `level`.
fn start_client(conn: &Connection, level: u32) -> zbus::Result<(OwnedObjectPath, zbus::blocking::Proxy<'static>)> {
    let manager = proxy(conn, MANAGER_PATH, "org.freedesktop.GeoClue2.Manager")?;
    let path: OwnedObjectPath = manager.call("GetClient", &())?;
    let client: zbus::blocking::Proxy<'static> = zbus::blocking::proxy::Builder::new(conn)
        .destination(SERVICE)?
        .path(path.clone())?
        .interface("org.freedesktop.GeoClue2.Client")?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()?;
    client.set_property("DesktopId", DESKTOP_ID).map_err(zbus::Error::from)?;
    client.set_property("RequestedAccuracyLevel", level).map_err(zbus::Error::from)?;
    client.set_property("DistanceThreshold", 0u32).map_err(zbus::Error::from)?;
    client.call_method("Start", &())?;
    Ok((path, client))
}

#[cfg(target_os = "linux")]
/// Read a Location object.
fn read_location(conn: &Connection, path: &str) -> zbus::Result<Position> {
    let loc = proxy(conn, path, "org.freedesktop.GeoClue2.Location")?;
    let f = |name: &str| loc.get_property::<f64>(name);
    let altitude = f("Altitude").ok().filter(|&a| a > UNKNOWN_ALTITUDE);
    let speed = f("Speed").ok().filter(|&s| s >= 0.0);
    // Heading means nothing when standing still.
    let heading = f("Heading").ok().filter(|&h| h >= 0.0 && speed.is_some_and(|s| s > 0.0));
    let timestamp = loc
        .get_property::<(u64, u64)>("Timestamp")
        .ok()
        .map(|(s, us)| s as f64 * 1000.0 + us as f64 / 1000.0)
        .filter(|&t| t > 0.0)
        .unwrap_or_else(now_ms);
    Ok(Position { latitude: f("Latitude")?, longitude: f("Longitude")?, accuracy: f("Accuracy")?, altitude, speed, heading, timestamp })
}

#[cfg(target_os = "linux")]
/// Why GeoClue gave no positions.
#[derive(Debug)]
enum Failure {
    /// It isn't there (no system bus, or no GeoClue on it): the portal may do.
    Unavailable(String),
    /// It answered, and refused or failed.
    Refused(String),
}
#[cfg(target_os = "linux")]
use Failure::{Refused, Unavailable};

#[cfg(target_os = "linux")]
/// Talk to GeoClue until nobody is waiting any more.
fn serve(conn: &Connection) -> Result<(), Failure> {
    let explain = |e: zbus::Error| match &e {
        zbus::Error::MethodError(name, msg, _) if name.as_str().ends_with("AccessDenied") => {
            Refused(format!("The location service refused Ironswift{}", msg.as_deref().map(|m| format!(": {m}")).unwrap_or_default()))
        }
        zbus::Error::MethodError(name, _, _) if name.as_str().ends_with("ServiceUnknown") || name.as_str().ends_with("NameHasNoOwner") => Unavailable(e.to_string()),
        _ => Refused(format!("No location service is available ({e})")),
    };
    let Some(high) = keep_going() else { return Ok(()) };
    let mut level = if high { LEVEL_EXACT } else { LEVEL_STREET };
    let (_, mut client) = start_client(conn, level).map_err(explain)?;
    let mut seen = String::from("/");
    let mut current = None;
    loop {
        let Some(high) = keep_going() else { break };
        // A page asked for GPS accuracy: restart the client at the exact level.
        if high && level != LEVEL_EXACT {
            let _ = client.call_method("Stop", &());
            level = LEVEL_EXACT;
            client = start_client(conn, level).map_err(explain)?.1;
        }
        match client.get_property::<OwnedObjectPath>("Location") {
            Ok(p) if p.as_str() != "/" && p.as_str() != seen => {
                seen = p.as_str().to_string();
                match read_location(conn, &seen) {
                    Ok(pos) => {
                        current = Some(pos);
                        deliver(pos, false);
                    }
                    Err(e) => log!("[geolocation] couldn't read {seen}: {e}"),
                }
            }
            Ok(_) => {
                if let Some(pos) = current {
                    deliver(pos, true);
                }
            }
            Err(e) => {
                let _ = client.call_method("Stop", &());
                return Err(explain(e.into()));
            }
        }
        std::thread::sleep(POLL);
    }
    let _ = client.call_method("Stop", &());
    Ok(())
}

#[cfg(target_os = "linux")]
/// The XDG Location portal (`org.freedesktop.portal.Location`): a session with the accuracy
/// wanted, started (the portal may ask the user first: its answer comes as a `Response` on a
/// request object), then positions as `LocationUpdated` signals until the session is closed.
mod portal {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
    use zbus::zvariant::{OwnedValue, Value};

    const SERVICE: &str = "org.freedesktop.portal.Desktop";
    pub(super) const PATH: &str = "/org/freedesktop/portal/desktop";
    const IFACE: &str = "org.freedesktop.portal.Location";
    /// Portal accuracy levels (the portal's own scale): street, and exact.
    pub(super) const STREET: u32 = 4;
    pub(super) const EXACT: u32 = 5;
    /// How long the user has to answer the portal's question.
    const ANSWER_WAIT: Duration = Duration::from_secs(120);

    /// The session bus connection and the portal's signals, kept for the browser's lifetime
    /// (the thread reading signals has no way to stop while it waits for one).
    static BUS: Mutex<Option<(Connection, std::sync::Arc<Mutex<Receiver<zbus::Message>>>)>> = Mutex::new(None);

    /// Read the portal's location updates and request answers on their own threads.
    pub(super) fn signals(conn: &Connection) -> zbus::Result<Receiver<zbus::Message>> {
        let (tx, rx) = channel();
        for (iface, member) in [(IFACE, "LocationUpdated"), ("org.freedesktop.portal.Request", "Response")] {
            let rule = zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).interface(iface)?.member(member)?.build();
            let iter = zbus::blocking::MessageIterator::for_match_rule(rule, conn, Some(256))?;
            let tx = tx.clone();
            std::thread::Builder::new()
                .name("geolocation-portal".into())
                .spawn(move || {
                    for m in iter {
                        let Ok(m) = m else { break };
                        if tx.send(m).is_err() {
                            break;
                        }
                    }
                })
                .map_err(|e| zbus::Error::Failure(e.to_string()))?;
        }
        Ok(rx)
    }

    pub(super) fn run() -> Result<(), String> {
        let (conn, rx) = {
            let mut bus = BUS.lock().unwrap_or_else(|e| e.into_inner());
            if bus.is_none() {
                let conn = Connection::session().map_err(|e| format!("No location service is available ({e})"))?;
                let rx = signals(&conn).map_err(|e| format!("No location service is available ({e})"))?;
                *bus = Some((conn, std::sync::Arc::new(Mutex::new(rx))));
            }
            let (c, r) = bus.as_ref().expect("just set");
            (c.clone(), r.clone())
        };
        let rx = rx.lock().unwrap_or_else(|e| e.into_inner());
        // Answers left over from an earlier session.
        while rx.try_recv().is_ok() {}
        serve(&conn, &rx)
    }

    fn f64_of(v: Option<&OwnedValue>) -> Option<f64> {
        v.and_then(|v| f64::try_from(v).ok())
    }

    /// A `LocationUpdated` location dictionary.
    pub(super) fn position(d: &HashMap<String, OwnedValue>) -> Option<Position> {
        let altitude = f64_of(d.get("Altitude")).filter(|&a| a > UNKNOWN_ALTITUDE);
        let speed = f64_of(d.get("Speed")).filter(|&s| s >= 0.0);
        let heading = f64_of(d.get("Heading")).filter(|&h| h >= 0.0 && speed.is_some_and(|s| s > 0.0));
        let timestamp = d
            .get("Timestamp")
            .and_then(|v| <(u64, u64)>::try_from(v.try_clone().ok()?).ok())
            .map(|(s, us)| s as f64 * 1000.0 + us as f64 / 1000.0)
            .filter(|&t| t > 0.0)
            .unwrap_or_else(now_ms);
        Some(Position {
            latitude: f64_of(d.get("Latitude"))?,
            longitude: f64_of(d.get("Longitude"))?,
            accuracy: f64_of(d.get("Accuracy")).unwrap_or(0.0),
            altitude,
            speed,
            heading,
            timestamp,
        })
    }

    fn close(conn: &Connection, session: &OwnedObjectPath) {
        let _ = conn.call_method(Some(SERVICE), session.as_str(), Some("org.freedesktop.portal.Session"), "Close", &());
    }

    /// Start a session at `level`. None if nobody is waiting any more by the time the user
    /// answers. Location updates that come in meanwhile are kept in `early`.
    fn start(conn: &Connection, rx: &Receiver<zbus::Message>, level: u32, early: &mut Vec<zbus::Message>) -> Result<Option<OwnedObjectPath>, String> {
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let failed = |e: zbus::Error| format!("The location portal failed ({e})");
        let loc = zbus::blocking::proxy::Builder::<zbus::blocking::Proxy<'_>>::new(conn)
            .destination(SERVICE)
            .and_then(|b| b.path(PATH))
            .and_then(|b| b.interface(IFACE))
            .map(|b| b.cache_properties(zbus::proxy::CacheProperties::No))
            .and_then(|b| b.build())
            .map_err(failed)?;
        let token = format!("ironswift{}_{n}", std::process::id());
        let opts: HashMap<&str, Value> = HashMap::from([
            ("session_handle_token", Value::from(token.as_str())),
            ("distance-threshold", Value::from(0u32)),
            ("time-threshold", Value::from(0u32)),
            ("accuracy", Value::from(level)),
        ]);
        let session: OwnedObjectPath = loc.call("CreateSession", &(opts,)).map_err(|e| match &e {
            zbus::Error::MethodError(name, _, _) if name.as_str().ends_with("ServiceUnknown") || name.as_str().ends_with("UnknownMethod") || name.as_str().ends_with("UnknownInterface") => {
                format!("No location service is available (the desktop has no location portal: {e})")
            }
            _ => failed(e),
        })?;
        let handle = format!("ironswift_r{n}");
        let opts: HashMap<&str, Value> = HashMap::from([("handle_token", Value::from(handle.as_str()))]);
        let request: OwnedObjectPath = match loc.call("Start", &(session.as_ref(), "", opts)) {
            Ok(r) => r,
            Err(e) => {
                close(conn, &session);
                return Err(failed(e));
            }
        };
        let until = Instant::now() + ANSWER_WAIT;
        loop {
            if keep_going().is_none() {
                close(conn, &session);
                return Ok(None);
            }
            if Instant::now() >= until {
                close(conn, &session);
                return Err("The location portal didn't answer".into());
            }
            let m = match rx.recv_timeout(POLL) {
                Ok(m) => m,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return Err("The location portal went away".into()),
            };
            let h = m.header();
            match h.member().map(|m| m.as_str()) {
                Some("Response") if h.path().is_some_and(|p| p.as_str() == request.as_str()) => {
                    let (code, _): (u32, HashMap<String, OwnedValue>) = m.body().deserialize().map_err(|e| failed(e.into()))?;
                    return match code {
                        0 => Ok(Some(session)),
                        1 => {
                            close(conn, &session);
                            Err("The location service refused Ironswift: location access wasn't allowed".into())
                        }
                        _ => {
                            close(conn, &session);
                            Err("The location portal couldn't find the position".into())
                        }
                    };
                }
                Some("LocationUpdated") => early.push(m.clone()),
                _ => {}
            }
        }
    }

    /// Talk to the portal until nobody is waiting any more.
    pub(super) fn serve(conn: &Connection, rx: &Receiver<zbus::Message>) -> Result<(), String> {
        let Some(high) = keep_going() else { return Ok(()) };
        let mut level = if high { EXACT } else { STREET };
        let mut early = Vec::new();
        let Some(mut session) = start(conn, rx, level, &mut early)? else { return Ok(()) };
        let mut current = None;
        loop {
            let Some(high) = keep_going() else { break };
            // A page asked for GPS accuracy: a new session at the exact level.
            if high && level != EXACT {
                close(conn, &session);
                level = EXACT;
                match start(conn, rx, level, &mut early)? {
                    Some(s) => session = s,
                    None => return Ok(()),
                }
            }
            let next = if early.is_empty() { rx.recv_timeout(POLL) } else { Ok(early.remove(0)) };
            match next {
                Ok(m) => {
                    let h = m.header();
                    if h.member().is_some_and(|n| n.as_str() == "LocationUpdated") {
                        match m.body().deserialize::<(OwnedObjectPath, HashMap<String, OwnedValue>)>() {
                            Ok((s, loc)) if s == session => {
                                if let Some(pos) = position(&loc) {
                                    current = Some(pos);
                                    deliver(pos, false);
                                }
                            }
                            Ok(_) => {}
                            Err(e) => log!("[geolocation] couldn't read a position from the portal: {e}"),
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Err("The location portal went away".into()),
            }
            // Pages that started watching get the current position.
            if let Some(pos) = current {
                deliver(pos, true);
            }
        }
        close(conn, &session);
        Ok(())
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A pretend GeoClue: one client whose location appears once it is started.
    struct Manager;
    struct Client {
        started: Arc<AtomicU32>,
        level: Arc<AtomicU32>,
        desktop_id: Arc<Mutex<String>>,
    }
    struct Location;

    #[zbus::interface(name = "org.freedesktop.GeoClue2.Manager")]
    impl Manager {
        fn get_client(&self) -> OwnedObjectPath {
            OwnedObjectPath::try_from("/org/freedesktop/GeoClue2/Client/1").unwrap()
        }
    }

    #[zbus::interface(name = "org.freedesktop.GeoClue2.Client")]
    impl Client {
        fn start(&self) {
            self.started.fetch_add(1, Ordering::SeqCst);
        }
        fn stop(&self) {}
        #[zbus(property)]
        fn location(&self) -> OwnedObjectPath {
            let p = if self.started.load(Ordering::SeqCst) > 0 { "/org/freedesktop/GeoClue2/Client/1/Location/1" } else { "/" };
            OwnedObjectPath::try_from(p).unwrap()
        }
        #[zbus(property)]
        fn desktop_id(&self) -> String {
            self.desktop_id.lock().unwrap().clone()
        }
        #[zbus(property)]
        fn set_desktop_id(&mut self, v: String) {
            *self.desktop_id.lock().unwrap() = v;
        }
        #[zbus(property)]
        fn requested_accuracy_level(&self) -> u32 {
            self.level.load(Ordering::SeqCst)
        }
        #[zbus(property)]
        fn set_requested_accuracy_level(&mut self, v: u32) {
            self.level.store(v, Ordering::SeqCst);
        }
        #[zbus(property)]
        fn distance_threshold(&self) -> u32 {
            0
        }
        #[zbus(property)]
        fn set_distance_threshold(&mut self, _v: u32) {}
    }

    #[zbus::interface(name = "org.freedesktop.GeoClue2.Location")]
    impl Location {
        #[zbus(property)]
        fn latitude(&self) -> f64 {
            29.4608
        }
        #[zbus(property)]
        fn longitude(&self) -> f64 {
            -95.0513
        }
        #[zbus(property)]
        fn accuracy(&self) -> f64 {
            25.0
        }
        #[zbus(property)]
        fn altitude(&self) -> f64 {
            -f64::MAX
        }
        #[zbus(property)]
        fn speed(&self) -> f64 {
            -1.0
        }
        #[zbus(property)]
        fn heading(&self) -> f64 {
            -1.0
        }
        #[zbus(property)]
        fn timestamp(&self) -> (u64, u64) {
            (1_700_000_000, 500_000)
        }
    }

    /// A pretend location portal: sessions, a Start that answers (before its reply, as a real
    /// portal may) and then sends one position.
    struct Portal {
        accuracy: Arc<AtomicU32>,
        closed: Arc<AtomicU32>,
        answer: u32,
    }
    struct Session {
        closed: Arc<AtomicU32>,
    }

    #[zbus::interface(name = "org.freedesktop.portal.Location")]
    impl Portal {
        async fn create_session(
            &self,
            options: std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
            #[zbus(object_server)] server: &zbus::ObjectServer,
        ) -> OwnedObjectPath {
            let token = options.get("session_handle_token").and_then(|v| String::try_from(v.try_clone().ok()?).ok()).unwrap_or_default();
            self.accuracy.store(options.get("accuracy").and_then(|v| u32::try_from(v).ok()).unwrap_or(0), Ordering::SeqCst);
            let path = format!("/org/freedesktop/portal/desktop/session/1_1/{token}");
            server.at(path.as_str(), Session { closed: self.closed.clone() }).await.unwrap();
            OwnedObjectPath::try_from(path).unwrap()
        }

        async fn start(
            &self,
            session: OwnedObjectPath,
            _parent: String,
            options: std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
            #[zbus(connection)] conn: &zbus::Connection,
        ) -> OwnedObjectPath {
            let token = options.get("handle_token").and_then(|v| String::try_from(v.try_clone().ok()?).ok()).unwrap_or_default();
            let request = format!("/org/freedesktop/portal/desktop/request/1_1/{token}");
            let none: std::collections::HashMap<&str, zbus::zvariant::Value> = std::collections::HashMap::new();
            conn.emit_signal(None::<()>, request.as_str(), "org.freedesktop.portal.Request", "Response", &(self.answer, none)).await.unwrap();
            if self.answer == 0 {
                use zbus::zvariant::Value;
                let loc: std::collections::HashMap<&str, Value> = std::collections::HashMap::from([
                    ("Latitude", Value::from(29.4608)),
                    ("Longitude", Value::from(-95.0513)),
                    ("Accuracy", Value::from(25.0)),
                    ("Altitude", Value::from(-f64::MAX)),
                    ("Speed", Value::from(-1.0)),
                    ("Heading", Value::from(-1.0)),
                    ("Timestamp", Value::from((1_700_000_000u64, 500_000u64))),
                    ("Description", Value::from("")),
                ]);
                conn.emit_signal(None::<()>, "/org/freedesktop/portal/desktop", "org.freedesktop.portal.Location", "LocationUpdated", &(session.as_ref(), loc)).await.unwrap();
            }
            OwnedObjectPath::try_from(request).unwrap()
        }
    }

    #[zbus::interface(name = "org.freedesktop.portal.Session")]
    impl Session {
        fn close(&self) {
            self.closed.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn pretend_portal(answer: u32) -> (zbus::blocking::Connection, zbus::blocking::Connection, Arc<AtomicU32>, Arc<AtomicU32>) {
        let (a, b) = std::os::unix::net::UnixStream::pair().unwrap();
        let (accuracy, closed) = (Arc::new(AtomicU32::new(0)), Arc::new(AtomicU32::new(0)));
        let portal = Portal { accuracy: accuracy.clone(), closed: closed.clone(), answer };
        let server = std::thread::spawn(move || {
            zbus::blocking::connection::Builder::async_io_unix_stream(a)
                .server(zbus::Guid::generate())
                .unwrap()
                .p2p()
                .serve_at(portal::PATH, portal)
                .unwrap()
                .build()
                .unwrap()
        });
        let conn = zbus::blocking::connection::Builder::async_io_unix_stream(b).p2p().build().unwrap();
        (conn, server.join().unwrap(), accuracy, closed)
    }

    /// The location portal (what Flatpak gets), against the pretend portal over a socket pair.
    #[test]
    fn talks_to_the_location_portal() {
        let _serial = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (conn, _server, accuracy, closed) = pretend_portal(0);
        let rx = portal::signals(&conn).unwrap();
        let epoch = u64::MAX - 8;
        state().wants.push(Want { epoch, id: 1, once: true, high: false, got: false });
        portal::serve(&conn, &rx).unwrap();
        let events = take_events(epoch);
        assert_eq!(
            events,
            ["__geoDone(1, {\"latitude\":29.4608,\"longitude\":-95.0513,\"accuracy\":25,\"altitude\":null,\"altitudeAccuracy\":null,\"heading\":null,\"speed\":null,\"timestamp\":1700000000500})"]
        );
        assert_eq!(accuracy.load(Ordering::SeqCst), portal::STREET);
        // The session is closed once nobody is waiting.
        assert_eq!(closed.load(Ordering::SeqCst), 1);
        page_gone(epoch);

        // The user said no in the portal's dialog: the page hears why.
        let (conn, _server, _, closed) = pretend_portal(1);
        let rx = portal::signals(&conn).unwrap();
        state().wants.push(Want { epoch, id: 2, once: true, high: true, got: false });
        let err = portal::serve(&conn, &rx).unwrap_err();
        assert!(err.contains("wasn't allowed"), "{err}");
        assert_eq!(closed.load(Ordering::SeqCst), 1);
        state().wants.retain(|w| w.epoch != epoch);
    }

    #[test]
    fn position_json() {
        let p = Position { latitude: 1.5, longitude: -2.0, accuracy: 3.0, altitude: None, speed: Some(0.0), heading: None, timestamp: 12.7 };
        assert_eq!(
            p.to_js(),
            "{\"latitude\":1.5,\"longitude\":-2,\"accuracy\":3,\"altitude\":null,\"altitudeAccuracy\":null,\"heading\":null,\"speed\":0,\"timestamp\":12}"
        );
    }

    /// The real D-Bus conversation with GeoClue, against the pretend service over a socket pair.
    #[test]
    fn talks_to_geoclue() {
        let _serial = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (a, b) = std::os::unix::net::UnixStream::pair().unwrap();
        let started = Arc::new(AtomicU32::new(0));
        let level = Arc::new(AtomicU32::new(0));
        let desktop_id = Arc::new(Mutex::new(String::new()));
        let client = Client { started: started.clone(), level: level.clone(), desktop_id: desktop_id.clone() };
        let server = std::thread::spawn(move || {
            zbus::blocking::connection::Builder::async_io_unix_stream(a)
                .server(zbus::Guid::generate())
                .unwrap()
                .p2p()
                .serve_at(MANAGER_PATH, Manager)
                .unwrap()
                .serve_at("/org/freedesktop/GeoClue2/Client/1", client)
                .unwrap()
                .serve_at("/org/freedesktop/GeoClue2/Client/1/Location/1", Location)
                .unwrap()
                .build()
                .unwrap()
        });
        let conn = zbus::blocking::connection::Builder::async_io_unix_stream(b).p2p().build().unwrap();
        let _server_conn = server.join().unwrap();

        // An epoch no real page uses.
        let epoch = u64::MAX - 7;
        state().wants.push(Want { epoch, id: 1, once: true, high: true, got: false });
        serve(&conn).unwrap();
        let events = take_events(epoch);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(
            events[0],
            "__geoDone(1, {\"latitude\":29.4608,\"longitude\":-95.0513,\"accuracy\":25,\"altitude\":null,\"altitudeAccuracy\":null,\"heading\":null,\"speed\":null,\"timestamp\":1700000000500})"
        );
        assert_eq!(started.load(Ordering::SeqCst), 1);
        assert_eq!(level.load(Ordering::SeqCst), LEVEL_EXACT);
        assert_eq!(*desktop_id.lock().unwrap(), DESKTOP_ID);
        // The one-off request is finished, and the last position answers maximumAge.
        assert!(state().wants.iter().all(|w| w.epoch != epoch));
        start(epoch, 2, true, false, 60_000.0);
        assert!(take_events(epoch)[0].starts_with("__geoDone(2, {\"latitude\":29.4608"));
        page_gone(epoch);
    }
}
