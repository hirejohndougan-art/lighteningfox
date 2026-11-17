// Stand-ins for the browser modules security.rs and blocklist.rs reach into (no profile
// folder, no network), so the parsers can be fuzzed on their own.

macro_rules! log {
    ($($t:tt)*) => {{
        let _ = format!($($t)*);
    }};
}

pub mod settings {
    #[derive(Default)]
    pub struct Settings {
        pub resist_fingerprinting: bool,
        pub block_ads: bool,
        pub block_trackers: bool,
        pub block_annoyances: bool,
        pub safe_browsing: bool,
        pub no_blocking: Vec<String>,
    }
    pub fn get() -> Settings {
        Settings { resist_fingerprinting: true, block_ads: true, block_trackers: true, block_annoyances: false, safe_browsing: true, no_blocking: Vec::new() }
    }
}

pub mod net {
    pub fn percent_decode_str(s: &str) -> String {
        s.to_string()
    }
    pub fn origin_of(url: &str) -> String {
        match url::Url::parse(url) {
            Ok(u) if u.scheme() == "http" || u.scheme() == "https" => u.origin().ascii_serialization(),
            _ => "null".into(),
        }
    }
    pub struct Request<'a> {
        pub method: &'a str,
        pub url: &'a str,
        pub headers: &'a [(String, String)],
        pub body: Option<&'a [u8]>,
        pub top: Option<&'a str>,
        pub cookies: bool,
        pub extra: &'a [(String, String)],
    }
    pub struct Response {
        pub status: u16,
        pub body: Vec<u8>,
    }
    pub fn http(_: &Request) -> Result<Response, String> {
        Err("offline".into())
    }
}

pub mod cookies {
    pub fn site_of(host: &str) -> String {
        let labels: Vec<&str> = host.split('.').collect();
        labels[labels.len().saturating_sub(2)..].join(".")
    }
    pub fn profile_dir() -> Option<std::path::PathBuf> {
        None
    }
}

/// dom.rs keeps per-document form state; the parsers don't need any.
pub mod validation {
    #[derive(Debug, Clone, Default)]
    pub struct FormState;
}

pub mod platform {
    pub fn path_from_url(path: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(path)
    }
}
