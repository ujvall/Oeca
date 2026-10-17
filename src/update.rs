//! Update availability (notification only): GitHub's latest stable release
//! of Oeca, compared with this build's version. Fetched off the UI thread
//! (a thread-pool work item), the result posted back to the notch. Nothing
//! is downloaded or installed; the notice only links to the release page.

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Networking::WinHttp::{
    WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_OPTION_REDIRECT_POLICY,
    WINHTTP_OPTION_REDIRECT_POLICY_NEVER, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders,
    WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetOption,
    WinHttpSetTimeouts,
};
use windows::Win32::System::Threading::{PTP_CALLBACK_INSTANCE, TrySubmitThreadpoolCallback};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::core::{PCWSTR, w};

/// This build's version (Cargo.toml, at build time).
pub const INSTALLED: &str = env!("CARGO_PKG_VERSION");

/// The one endpoint ever asked: the latest published, non-draft,
/// non-prerelease release of the repository.
const API_HOST: PCWSTR = w!("api.github.com");
const API_PATH: PCWSTR = w!("/repos/ujvall/Oeca/releases/latest");
/// A release page this repository's releases live under (the only URL the
/// notice will open).
const RELEASE_PAGE: &str = "https://github.com/ujvall/Oeca/releases/tag/";
/// The installer every release carries, `Oeca-Setup-{VERSION}.exe`
/// (installer/Oeca.iss's output, `Oeca-Setup.exe`, uploaded under its
/// release's version: see `installer_name`), and where GitHub serves a
/// release's assets. Only checked to say whether the release page offers
/// it: never downloaded or run here.
pub const INSTALLER_STEM: &str = "Oeca-Setup";
const RELEASE_DOWNLOAD: &str = "https://github.com/ujvall/Oeca/releases/download/";
/// An installer larger than this is not Oeca's (it is about 3 MB).
const MAX_INSTALLER_BYTES: f64 = 200.0 * 1024.0 * 1024.0;

/// The first check, once startup has settled, then one a day (ms). A
/// failure simply waits for the next one (no retries: rate limits).
pub const FIRST_CHECK_MS: u32 = 30_000;
pub const RECHECK_MS: u32 = 24 * 60 * 60 * 1000;
/// The notch's timers: the next check, and the notice's time on show (it
/// then leaves on its own, as if dismissed).
pub const CHECK_TIMER_ID: usize = 1011;
pub const HIDE_TIMER_ID: usize = 1012;
pub const SHOW_MS: u32 = 12_000;

/// Per-step network timeouts (ms), the whole request's bound (s), and the
/// largest response read (release notes included, well under this).
const RESOLVE_MS: i32 = 10_000;
const CONNECT_MS: i32 = 10_000;
const SEND_MS: i32 = 10_000;
const RECEIVE_MS: i32 = 15_000;
const TOTAL_SECS: u64 = 30;
const MAX_BODY: usize = 1 << 20;

/// A newer stable release: its version, its validated release page, and
/// the official installer that page offers (its asset name), if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: (u64, u64, u64),
    pub url: String,
    pub installer: Option<String>,
}

impl Release {
    /// "Oeca update available · v1.2.3".
    pub fn label(&self) -> String {
        let (a, b, c) = self.version;
        format!("Oeca update available \u{00B7} v{a}.{b}.{c}")
    }

    /// The notice's accessible name: the label and what a click does (the
    /// installer is mentioned only when the release has it).
    pub fn title(&self) -> String {
        let action = match &self.installer {
            Some(name) => format!("Opens the release page to download {name}."),
            None => "Opens the release page.".to_string(),
        };
        format!("{}. {action}", self.label())
    }
}

/// `MAJOR.MINOR.PATCH`, one optional leading `v`; anything else (missing
/// parts, pre-release or build suffixes, signs, spaces) is not a version.
pub fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.strip_prefix(['v', 'V']).unwrap_or(s);
    let mut parts = s.split('.').map(|p| {
        (!p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit()))
            .then(|| p.parse().ok())
            .flatten()
    });
    let v = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(v)
}

/// What one check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A valid stable release strictly newer than this build.
    Newer(Release),
    /// The latest valid stable release is this build's version or older.
    Current,
    /// No answer: offline, timed out, rate limited, an error status, or a
    /// response that is not a valid stable release of this repository.
    Failed,
}

/// The release in a latest-release response, if it is a valid, stable one
/// strictly newer than `installed`. Malformed JSON, missing fields, drafts,
/// prereleases and any page outside this repository's releases: None.
#[cfg(test)]
pub fn newer_release(json: &str, installed: &str) -> Option<Release> {
    match evaluate(json, installed) {
        Outcome::Newer(release) => Some(release),
        Outcome::Current | Outcome::Failed => None,
    }
}

/// A latest-release response judged against `installed`: newer, current,
/// or (anything invalid, including `installed` itself) failed.
pub fn evaluate(json: &str, installed: &str) -> Outcome {
    let Some(fields) = top_level_fields(json) else {
        return Outcome::Failed;
    };
    let get = |key: &str| fields.iter().find(|(k, _)| k == key).map(|(_, v)| v);
    let (Some(Json::Str(tag)), Some(Json::Str(url))) = (get("tag_name"), get("html_url")) else {
        return Outcome::Failed;
    };
    if get("draft") != Some(&Json::Bool(false)) || get("prerelease") != Some(&Json::Bool(false)) {
        return Outcome::Failed;
    }
    let (Some(version), Some(installed)) = (parse_version(tag), parse_version(installed)) else {
        return Outcome::Failed;
    };
    // Only this repository's page for this very tag (the tag is a version:
    // digits, dots and a `v`, nothing that could change the URL)
    if url.strip_prefix(RELEASE_PAGE) != Some(tag.as_str()) {
        return Outcome::Failed;
    }
    if version <= installed {
        return Outcome::Current;
    }
    Outcome::Newer(Release {
        version,
        url: url.clone(),
        installer: get("assets").and_then(|assets| installer_in(assets, tag)),
    })
}

/// How the last update check went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Check {
    /// None yet this run.
    #[default]
    Idle,
    Checking,
    /// Answered: a newer release, or this build is current.
    Done,
    Failed,
}

/// What the Info page says about updates: how the last check went, and
/// the newer release known (kept through a failed check), if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Status {
    pub check: Check,
    pub available: Option<(u64, u64, u64)>,
}

/// The one update state, shared by the collapsed notch's icon, the notice
/// and Settings: the newest valid release known (kept until a check finds
/// this build current), the last check's result, the one check in flight
/// (a later request joins it instead of starting another; a result that is
/// not that check's is stale and ignored), and the version whose notice was
/// dismissed (the release itself stays available).
#[derive(Debug, Default)]
pub struct UpdateState {
    release: Option<Release>,
    last: Option<Check>,
    in_flight: Option<usize>,
    next_id: usize,
    dismissed: Option<(u64, u64, u64)>,
}

impl UpdateState {
    /// Starts a check: its id, or None if one is already running (its
    /// result serves this request too).
    pub fn begin(&mut self) -> Option<usize> {
        if self.in_flight.is_some() {
            return None;
        }
        self.next_id = self.next_id.wrapping_add(1);
        self.in_flight = Some(self.next_id);
        self.in_flight
    }

    /// Applies check `id`'s outcome; false (nothing changes) if it is not
    /// the check in flight. A failure keeps the release already known.
    pub fn finish(&mut self, id: usize, outcome: Outcome) -> bool {
        if self.in_flight != Some(id) {
            return false;
        }
        self.in_flight = None;
        self.last = Some(match outcome {
            Outcome::Newer(release) => {
                self.release = Some(release);
                Check::Done
            }
            Outcome::Current => {
                self.release = None;
                Check::Done
            }
            Outcome::Failed => Check::Failed,
        });
        true
    }

    /// The newest valid release known to be newer than this build.
    pub fn release(&self) -> Option<&Release> {
        self.release.as_ref()
    }

    /// The notice should be offered: a release whose notice was not
    /// dismissed.
    pub fn notice_due(&self) -> bool {
        self.release
            .as_ref()
            .is_some_and(|r| self.dismissed != Some(r.version))
    }

    /// The notice was dismissed (or opened): not offered again for this
    /// version. The release stays available (the notch's icon, Settings).
    pub fn dismiss_notice(&mut self) {
        if let Some(r) = &self.release {
            self.dismissed = Some(r.version);
        }
    }

    pub fn status(&self) -> Status {
        Status {
            check: if self.in_flight.is_some() {
                Check::Checking
            } else {
                self.last.unwrap_or_default()
            },
            available: self.release.as_ref().map(|r| r.version),
        }
    }
}

/// The installer asset's exact name for release `tag` (a validated version
/// tag): `Oeca-Setup-0.3.0.exe` for `v0.3.0`.
pub fn installer_name(tag: &str) -> String {
    let version = tag.strip_prefix(['v', 'V']).unwrap_or(tag);
    format!("{INSTALLER_STEM}-{version}.exe")
}

/// The official installer in `assets`, by name: exactly `installer_name`,
/// fully uploaded, a plausible size, served from exactly this release's own
/// download path. Anything missing, malformed or elsewhere: None.
fn installer_in(assets: &Json, tag: &str) -> Option<String> {
    let Json::Arr(assets) = assets else {
        return None;
    };
    let name = installer_name(tag);
    let url = format!("{RELEASE_DOWNLOAD}{tag}/{name}");
    let found = assets.iter().any(|asset| {
        let Json::Obj(fields) = asset else {
            return false;
        };
        let get = |key: &str| fields.iter().find(|(k, _)| k == key).map(|(_, v)| v);
        let is = |key: &str, want: &str| matches!(get(key), Some(Json::Str(s)) if s == want);
        is("name", &name)
            && is("state", "uploaded")
            && is("browser_download_url", &url)
            && matches!(get("size"), Some(Json::Num(n)) if *n > 0.0 && *n <= MAX_INSTALLER_BYTES)
    });
    found.then_some(name)
}

/// A JSON value.
#[derive(Debug, PartialEq)]
enum Json {
    Str(String),
    Bool(bool),
    Num(f64),
    Null,
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

/// The fields of a JSON object. None unless the whole text is one
/// well-formed object.
fn top_level_fields(json: &str) -> Option<Vec<(String, Json)>> {
    let mut p = Parser {
        s: json.as_bytes(),
        i: 0,
    };
    let Json::Obj(fields) = p.value(0)? else {
        return None;
    };
    p.ws();
    (p.i == p.s.len()).then_some(fields)
}

/// A minimal, strict JSON reader (bounded nesting; the input is bounded by
/// `MAX_BODY`).
struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    const MAX_DEPTH: u32 = 64;

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.i += 1;
        Some(b)
    }

    fn eat(&mut self, b: u8) -> Option<()> {
        (self.next()? == b).then_some(())
    }

    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn literal(&mut self, word: &[u8]) -> Option<()> {
        let end = self.i + word.len();
        (self.s.get(self.i..end)? == word).then(|| self.i = end)
    }

    fn hex4(&mut self) -> Option<u32> {
        let h = std::str::from_utf8(self.s.get(self.i..self.i + 4)?).ok()?;
        self.i += 4;
        u32::from_str_radix(h, 16).ok()
    }

    fn string(&mut self) -> Option<String> {
        self.eat(b'"')?;
        let mut out = Vec::new();
        loop {
            match self.next()? {
                b'"' => return String::from_utf8(out).ok(),
                b'\\' => {
                    let c = match self.next()? {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let hi = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&hi) {
                                self.literal(b"\\u")?;
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return None;
                                }
                                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                            } else {
                                hi
                            };
                            char::from_u32(code)?
                        }
                        _ => return None,
                    };
                    out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
                }
                b if b < 0x20 => return None,
                b => out.push(b),
            }
        }
    }

    fn value(&mut self, depth: u32) -> Option<Json> {
        if depth > Self::MAX_DEPTH {
            return None;
        }
        self.ws();
        match self.peek()? {
            b'"' => self.string().map(Json::Str),
            b't' => self.literal(b"true").map(|()| Json::Bool(true)),
            b'f' => self.literal(b"false").map(|()| Json::Bool(false)),
            b'n' => self.literal(b"null").map(|()| Json::Null),
            open @ (b'{' | b'[') => {
                self.i += 1;
                let object = open == b'{';
                let close = if object { b'}' } else { b']' };
                let (mut fields, mut items) = (Vec::new(), Vec::new());
                self.ws();
                if self.peek() == Some(close) {
                    self.i += 1;
                } else {
                    loop {
                        let key = if object {
                            self.ws();
                            let key = self.string()?;
                            self.ws();
                            self.eat(b':')?;
                            Some(key)
                        } else {
                            None
                        };
                        let value = self.value(depth + 1)?;
                        match key {
                            Some(key) => fields.push((key, value)),
                            None => items.push(value),
                        }
                        self.ws();
                        match self.next()? {
                            b',' => {}
                            b if b == close => break,
                            _ => return None,
                        }
                    }
                }
                Some(if object {
                    Json::Obj(fields)
                } else {
                    Json::Arr(items)
                })
            }
            b'-' | b'0'..=b'9' => {
                let start = self.i;
                while matches!(
                    self.peek(),
                    Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
                ) {
                    self.i += 1;
                }
                std::str::from_utf8(&self.s[start..self.i])
                    .ok()?
                    .parse::<f64>()
                    .ok()
                    .filter(|n| n.is_finite())
                    .map(Json::Num)
            }
            _ => None,
        }
    }
}

/// Starts check `id` off the UI thread; its result arrives as `msg` (wParam:
/// `id`; lParam: a `Box<Outcome>` to take back with `Box::from_raw`). False
/// if no work item could be queued.
pub fn check_async(hwnd: HWND, msg: u32, id: usize) -> bool {
    let context = Box::into_raw(Box::new((hwnd.0 as isize, msg, id)));
    let queued =
        unsafe { TrySubmitThreadpoolCallback(Some(check_work), Some(context.cast()), None) };
    if queued.is_err() {
        // SAFETY: never handed to a work item, so still owned here
        drop(unsafe { Box::from_raw(context) });
    }
    queued.is_ok()
}

unsafe extern "system" fn check_work(_: PTP_CALLBACK_INSTANCE, context: *mut core::ffi::c_void) {
    // SAFETY: the `Box` from `check_async`, handed over exactly once
    let (target, msg, id) = *unsafe { Box::from_raw(context as *mut (isize, u32, usize)) };
    let outcome = fetch_latest().map_or(Outcome::Failed, |body| evaluate(&body, INSTALLED));
    let outcome = Box::into_raw(Box::new(outcome));
    let hwnd = HWND(target as *mut _);
    if unsafe { PostMessageW(Some(hwnd), msg, WPARAM(id), LPARAM(outcome as isize)) }.is_err() {
        // SAFETY: never posted, so still owned here
        drop(unsafe { Box::from_raw(outcome) });
    }
}

/// A WinHTTP handle, closed when dropped.
struct Handle(*mut core::ffi::c_void);

impl Handle {
    fn new(h: *mut core::ffi::c_void) -> Option<Self> {
        (!h.is_null()).then_some(Self(h))
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = WinHttpCloseHandle(self.0);
        }
    }
}

/// The latest-release response body: HTTPS only, no redirects, bounded in
/// time and size; None on any failure (offline, DNS, TLS, timeouts, any
/// status but 200 such as a rate limit's 403 / 429, an oversized body).
/// No credentials or personal data are sent: GitHub requires a
/// User-Agent, which names only the app and its version.
fn fetch_latest() -> Option<String> {
    let started = std::time::Instant::now();
    let agent: Vec<u16> = format!("Oeca/{INSTALLED}\0").encode_utf16().collect();
    unsafe {
        let session = Handle::new(WinHttpOpen(
            PCWSTR(agent.as_ptr()),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ))?;
        WinHttpSetTimeouts(session.0, RESOLVE_MS, CONNECT_MS, SEND_MS, RECEIVE_MS).ok()?;
        let connection = Handle::new(WinHttpConnect(session.0, API_HOST, 443, 0))?;
        let request = Handle::new(WinHttpOpenRequest(
            connection.0,
            w!("GET"),
            API_PATH,
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ))?;
        // Only the fixed endpoint: never follow a redirect elsewhere
        let never = WINHTTP_OPTION_REDIRECT_POLICY_NEVER.to_ne_bytes();
        WinHttpSetOption(
            Some(request.0),
            WINHTTP_OPTION_REDIRECT_POLICY,
            Some(&never),
        )
        .ok()?;
        let headers: Vec<u16> = "Accept: application/vnd.github+json\r\n"
            .encode_utf16()
            .collect();
        WinHttpSendRequest(request.0, Some(&headers), None, 0, 0, 0).ok()?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut()).ok()?;
        let mut status = 0u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&raw mut status).cast()),
            &mut size,
            std::ptr::null_mut(),
        )
        .ok()?;
        if status != 200 {
            return None;
        }
        let mut body = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            if started.elapsed().as_secs() > TOTAL_SECS {
                return None;
            }
            let mut read = 0u32;
            WinHttpReadData(
                request.0,
                chunk.as_mut_ptr().cast(),
                chunk.len() as u32,
                &mut read,
            )
            .ok()?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..read as usize]);
            if body.len() > MAX_BODY {
                return None;
            }
        }
        String::from_utf8(body).ok()
    }
}

/// `url` is this repository's page for the release of `version`.
fn is_release_page(url: &str, version: (u64, u64, u64)) -> bool {
    url.strip_prefix(RELEASE_PAGE)
        .is_some_and(|tag| parse_version(tag) == Some(version))
}

/// Opens a validated release page in the default browser (nothing else:
/// any other URL is refused).
pub fn open_release(release: &Release) {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    if !is_release_page(&release.url, release.version) {
        return;
    }
    let url: Vec<u16> = release.url.encode_utf16().chain([0]).collect();
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(url.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(tag: &str, url: &str, draft: bool, pre: bool) -> String {
        with_assets(tag, url, draft, pre, &installer_asset(tag))
    }

    fn with_assets(tag: &str, url: &str, draft: bool, pre: bool, assets: &str) -> String {
        format!(
            r#"{{"url":"https://api.github.com/repos/ujvall/Oeca/releases/1",
            "html_url":"{url}","author":{{"login":"ujvall","html_url":"https://github.com/ujvall"}},
            "tag_name":"{tag}","name":"Oeca {tag}","draft":{draft},"prerelease":{pre},
            "assets":[{assets}],
            "body":"Notes \u00b7 \"quoted\"\n- fixes","id":12345,"rate":-1.5e3,"x":null}}"#
        )
    }

    /// The asset as GitHub lists it (the fields Oeca checks, and others).
    fn installer_asset(tag: &str) -> String {
        let name = installer_name(tag);
        asset(
            &name,
            "uploaded",
            "2978067",
            &format!("{RELEASE_DOWNLOAD}{tag}/{name}"),
        )
    }

    fn asset(name: &str, state: &str, size: &str, url: &str) -> String {
        format!(
            r#"{{"id":1,"name":"{name}","label":null,"content_type":"application/x-msdownload",
            "state":"{state}","size":{size},"download_count":3,
            "uploader":{{"login":"ujvall"}},"browser_download_url":"{url}"}}"#
        )
    }

    fn page(tag: &str) -> String {
        format!("{RELEASE_PAGE}{tag}")
    }

    #[test]
    fn test_versions_parse_strictly() {
        assert_eq!(parse_version("0.2.0"), Some((0, 2, 0)));
        assert_eq!(parse_version("v1.10.3"), Some((1, 10, 3)));
        assert_eq!(parse_version("V12.0.100"), Some((12, 0, 100)));
        for bad in [
            "",
            "v",
            "1",
            "1.2",
            "1.2.3.4",
            "1.2.x",
            "1..3",
            "vv1.2.3",
            " 1.2.3",
            "1.2.3 ",
            "1.2.3-beta.1",
            "1.2.3+build",
            "-1.2.3",
            "+1.2.3",
            "1.2.3a",
            "9999999999.0.0",
        ] {
            assert_eq!(parse_version(bad), None, "{bad:?}");
        }
        assert!(
            parse_version(INSTALLED).is_some(),
            "the build's own version"
        );
    }

    #[test]
    fn test_only_strictly_newer_versions_count() {
        let check = |tag: &str, installed: &str| {
            newer_release(&response(tag, &page(tag), false, false), installed)
        };
        assert_eq!(
            check("v0.3.0", "0.2.0"),
            Some(Release {
                version: (0, 3, 0),
                url: page("v0.3.0"),
                installer: Some("Oeca-Setup-0.3.0.exe".into()),
            })
        );
        assert!(check("0.2.1", "0.2.0").is_some(), "no leading v");
        assert!(check("v1.0.0", "0.9.9").is_some());
        // Numeric, not textual: 0.10.0 > 0.9.0, 0.2.10 > 0.2.9
        assert!(check("v0.10.0", "0.9.0").is_some());
        assert!(check("v0.2.10", "0.2.9").is_some());
        assert!(check("v0.9.0", "0.10.0").is_none());
        // Equal or older: never
        assert!(check("v0.2.0", "0.2.0").is_none());
        assert!(check("v0.1.9", "0.2.0").is_none());
        // An invalid installed version: no comparison at all
        assert!(check("v9.0.0", "dev").is_none());
        assert!(check("v9.0.0", "").is_none());
        assert_eq!(
            check("v1.2.3", "0.2.0").unwrap().label(),
            "Oeca update available \u{00B7} v1.2.3"
        );
    }

    #[test]
    fn test_drafts_prereleases_and_bad_tags_are_ignored() {
        let tag = "v9.0.0";
        assert!(newer_release(&response(tag, &page(tag), true, false), "0.2.0").is_none());
        assert!(newer_release(&response(tag, &page(tag), false, true), "0.2.0").is_none());
        for tag in ["v9.0.0-beta", "nightly", "", "v9.0"] {
            assert!(
                newer_release(&response(tag, &page(tag), false, false), "0.2.0").is_none(),
                "{tag:?}"
            );
        }
    }

    #[test]
    fn test_only_this_repositorys_release_page_is_accepted() {
        let tag = "v9.0.0";
        for url in [
            "http://github.com/ujvall/Oeca/releases/tag/v9.0.0",
            "https://github.com/someone/Oeca/releases/tag/v9.0.0",
            "https://github.com/ujvall/Other/releases/tag/v9.0.0",
            "https://github.com.evil.example/ujvall/Oeca/releases/tag/v9.0.0",
            "https://evil.example/https://github.com/ujvall/Oeca/releases/tag/v9.0.0",
            "https://github.com/ujvall/Oeca/releases/tag/v8.0.0",
            "https://github.com/ujvall/Oeca/releases/tag/v9.0.0?x=1",
            "https://github.com/ujvall/Oeca/releases/download/v9.0.0/Oeca.exe",
            "https://github.com/ujvall/Oeca/releases/tag/",
            "",
        ] {
            assert!(
                newer_release(&response(tag, url, false, false), "0.2.0").is_none(),
                "{url}"
            );
        }
        // Opening refuses anything else too
        assert!(is_release_page(&page("v9.0.0"), (9, 0, 0)));
        assert!(!is_release_page(&page("v9.0.0"), (9, 0, 1)));
        assert!(!is_release_page("https://evil.example/v9.0.0", (9, 0, 0)));
    }

    #[test]
    fn test_malformed_or_incomplete_responses_are_ignored() {
        let good = response("v9.0.0", &page("v9.0.0"), false, false);
        assert!(
            newer_release(&good, "0.2.0").is_some(),
            "the fixture is valid"
        );
        let cases = [
            "".to_string(),
            "null".into(),
            "[]".into(),
            "{".into(),
            "not json".into(),
            good[..good.len() - 1].to_string(),
            format!("{good} trailing"),
            good.replace("\"tag_name\"", "\"tag\""),
            good.replace("\"html_url\":\"https", "\"page\":\"https"),
            good.replace("\"draft\":false", "\"draft\":\"false\""),
            good.replace(",\"prerelease\":false", ""),
            good.replace("\"tag_name\":\"v9.0.0\"", "\"tag_name\":9"),
            // A rate limit's (or any error's) body
            r#"{"message":"API rate limit exceeded","documentation_url":"https://docs.github.com"}"#
                .into(),
            r#"{"message":"Not Found"}"#.into(),
            "[".repeat(10_000),
        ];
        for json in &cases {
            assert!(newer_release(json, "0.2.0").is_none(), "{json:.60}");
        }
        // Only top-level fields count: a nested tag / page is not the release's
        let nested = format!(
            r#"{{"draft":false,"prerelease":false,"assets":[{{"tag_name":"v9.0.0","html_url":"{}"}}]}}"#,
            page("v9.0.0")
        );
        assert!(newer_release(&nested, "0.2.0").is_none());
    }

    #[test]
    fn test_checks_report_newer_current_or_failed() {
        let tag = "v9.0.0";
        let good = response(tag, &page(tag), false, false);
        assert!(matches!(evaluate(&good, "0.2.0"), Outcome::Newer(r) if r.url == page(tag)));
        assert_eq!(
            evaluate(&good, "9.0.0"),
            Outcome::Current,
            "the same version"
        );
        assert_eq!(evaluate(&good, "9.0.1"), Outcome::Current, "an older one");
        // Invalid data, or no valid installed version: failed, never current
        for json in [
            String::new(),
            "{}".into(),
            r#"{"message":"API rate limit exceeded"}"#.into(),
            response(tag, &page(tag), true, false),
            response(tag, &page(tag), false, true),
            response(tag, "https://evil.example/", false, false),
            response("nightly", &page("nightly"), false, false),
        ] {
            assert_eq!(evaluate(&json, "0.2.0"), Outcome::Failed, "{json:.60}");
        }
        assert_eq!(evaluate(&good, "dev"), Outcome::Failed);
    }

    fn status(check: Check, available: Option<(u64, u64, u64)>) -> Status {
        Status { check, available }
    }

    fn release(version: (u64, u64, u64)) -> Release {
        let (a, b, c) = version;
        let tag = format!("v{a}.{b}.{c}");
        Release {
            version,
            url: page(&tag),
            installer: Some(installer_name(&tag)),
        }
    }

    #[test]
    fn test_update_state_follows_checks_and_dismissal() {
        let mut s = UpdateState::default();
        assert_eq!(s.status(), status(Check::Idle, None));
        assert!(s.release().is_none() && !s.notice_due());
        // A scheduled check runs; a manual one meanwhile joins it
        let id = s.begin().unwrap();
        assert_eq!(s.begin(), None, "no second request in flight");
        assert_eq!(s.status().check, Check::Checking);
        // A stale or unknown result changes nothing
        assert!(!s.finish(id + 1, Outcome::Newer(release((9, 0, 0)))));
        assert_eq!(s.status().check, Check::Checking);
        // The newer release: available, offered, its validated page
        assert!(s.finish(id, Outcome::Newer(release((0, 3, 0)))));
        assert!(!s.finish(id, Outcome::Current), "applied once");
        assert_eq!(s.status(), status(Check::Done, Some((0, 3, 0))));
        assert_eq!(s.release().unwrap().url, page("v0.3.0"));
        assert!(s.notice_due());
        // Dismissing the notice keeps the release (the icon, Settings)
        s.dismiss_notice();
        assert!(!s.notice_due());
        assert_eq!(s.release().unwrap().version, (0, 3, 0));
        assert_eq!(s.status(), status(Check::Done, Some((0, 3, 0))));
        // A failed manual check keeps what is known; a retry is allowed
        let id = s.begin().unwrap();
        assert_eq!(s.status(), status(Check::Checking, Some((0, 3, 0))));
        assert!(s.finish(id, Outcome::Failed));
        assert_eq!(
            s.status(),
            status(Check::Failed, Some((0, 3, 0))),
            "the failure shows; the release stays"
        );
        assert!(!s.notice_due(), "still dismissed for 0.3.0");
        // An even newer release replaces it (its page opens) and is offered
        let id = s.begin().unwrap();
        assert!(s.finish(id, Outcome::Newer(release((0, 4, 0)))));
        assert_eq!(s.release().unwrap().url, page("v0.4.0"));
        assert!(s.notice_due());
        // This build current: nothing available, up to date
        let id = s.begin().unwrap();
        assert!(s.finish(id, Outcome::Current));
        assert!(s.release().is_none() && !s.notice_due());
        assert_eq!(s.status(), status(Check::Done, None));
        // Checks can run again after up to date and after failure
        let id = s.begin().unwrap();
        assert!(s.finish(id, Outcome::Failed));
        assert_eq!(s.status(), status(Check::Failed, None));
        let id = s.begin().unwrap();
        assert_eq!(s.status().check, Check::Checking);
        assert!(s.finish(id, Outcome::Current));
        assert_eq!(s.status(), status(Check::Done, None));
    }

    #[test]
    fn test_json_strings_and_values() {
        let fields = top_level_fields(
            r#" { "a" : "x\"\\\/\u00e9\ud83d\ude00\n" , "b":true, "c":[1,{"d":[]}], "e":{} } "#,
        )
        .unwrap();
        assert_eq!(
            fields[0],
            ("a".into(), Json::Str("x\"\\/\u{e9}\u{1F600}\n".into()))
        );
        assert_eq!(fields[1], ("b".into(), Json::Bool(true)));
        assert_eq!(
            fields[2].1,
            Json::Arr(vec![
                Json::Num(1.0),
                Json::Obj(vec![("d".into(), Json::Arr(vec![]))])
            ])
        );
        assert_eq!(fields[3].1, Json::Obj(vec![]));
        assert_eq!(top_level_fields("{}"), Some(vec![]));
        for bad in [
            r#"{"a":"\ud83d"}"#,
            r#"{"a":"\q"}"#,
            "{\"a\":\"line\nbreak\"}",
            r#"{"a":1,}"#,
            r#"{"a" 1}"#,
            r#"{"a":tru}"#,
            r#"{"a":-}"#,
        ] {
            assert_eq!(top_level_fields(bad), None, "{bad}");
        }
    }

    #[test]
    fn test_installer_asset_is_validated() {
        let tag = "v9.0.0";
        let installer = |assets: &str| {
            newer_release(&with_assets(tag, &page(tag), false, false, assets), "0.2.0")
                .map(|r| r.installer)
        };
        let name = "Oeca-Setup-9.0.0.exe";
        let found = Some(Some(name.to_string()));
        let url = format!("{RELEASE_DOWNLOAD}{tag}/{name}");
        // The release's version, without its `v`
        assert_eq!(installer_name("v0.3.0"), "Oeca-Setup-0.3.0.exe");
        assert_eq!(installer_name("v12.0.10"), "Oeca-Setup-12.0.10.exe");
        assert_eq!(installer_name("0.3.0"), "Oeca-Setup-0.3.0.exe");
        // Present, among other assets
        assert_eq!(installer(&asset(name, "uploaded", "2978067", &url)), found);
        let other = asset(
            "notes.txt",
            "uploaded",
            "10",
            &format!("{RELEASE_DOWNLOAD}{tag}/notes.txt"),
        );
        assert_eq!(
            installer(&format!("{other},{}", installer_asset(tag))),
            found
        );
        // A tag without its `v`: its own name and path
        let bare = "9.0.0";
        let r = newer_release(&response(bare, &page(bare), false, false), "0.2.0").unwrap();
        assert_eq!(r.installer.as_deref(), Some(name));
        // Missing or wrong: the release still counts, without an installer
        let at = |file: &str| format!("{RELEASE_DOWNLOAD}{tag}/{file}");
        for assets in [
            String::new(),
            other.clone(),
            // Other names: unversioned, another version, the tag's `v`,
            // case, suffixes, anything else
            asset("Oeca-Setup.exe", "uploaded", "1", &at("Oeca-Setup.exe")),
            asset(
                "Oeca-Setup-8.0.0.exe",
                "uploaded",
                "1",
                &at("Oeca-Setup-8.0.0.exe"),
            ),
            asset(
                "Oeca-Setup-9.0.1.exe",
                "uploaded",
                "1",
                &at("Oeca-Setup-9.0.1.exe"),
            ),
            asset(
                "Oeca-Setup-9.0.exe",
                "uploaded",
                "1",
                &at("Oeca-Setup-9.0.exe"),
            ),
            asset(
                "Oeca-Setup-v9.0.0.exe",
                "uploaded",
                "1",
                &at("Oeca-Setup-v9.0.0.exe"),
            ),
            asset(
                "oeca-setup-9.0.0.exe",
                "uploaded",
                "1",
                &at("oeca-setup-9.0.0.exe"),
            ),
            asset(
                "Oeca-Setup-9.0.0.EXE",
                "uploaded",
                "1",
                &at("Oeca-Setup-9.0.0.EXE"),
            ),
            asset(
                "Oeca-Setup-9.0.0.exe.zip",
                "uploaded",
                "1",
                &at("Oeca-Setup-9.0.0.exe.zip"),
            ),
            asset(
                "Oeca-Setup-9.0.0-x64.exe",
                "uploaded",
                "1",
                &at("Oeca-Setup-9.0.0-x64.exe"),
            ),
            asset("Setup.exe", "uploaded", "1", &at("Setup.exe")),
            // The right name, served from elsewhere
            asset(name, "uploaded", "1", &at("Oeca-Setup.exe")),
            asset(name, "uploaded", "1", &format!("{url}?raw=1")),
            asset(name, "uploaded", "1", &format!("{url}#x")),
            asset(
                name,
                "uploaded",
                "1",
                &format!("https://evil.example/{name}"),
            ),
            asset(
                name,
                "uploaded",
                "1",
                &format!("http://github.com/ujvall/Oeca/releases/download/{tag}/{name}"),
            ),
            asset(
                name,
                "uploaded",
                "1",
                &format!("https://objects.githubusercontent.com/ujvall/Oeca/{name}"),
            ),
            asset(
                name,
                "uploaded",
                "1",
                &format!("{RELEASE_DOWNLOAD}v8.0.0/{name}"),
            ),
            asset(
                name,
                "uploaded",
                "1",
                &format!("https://github.com/someone/Oeca/releases/download/{tag}/{name}"),
            ),
            // Not fully uploaded, or an implausible size
            asset(name, "starter", "1", &url),
            asset(name, "uploaded", "0", &url),
            asset(name, "uploaded", "-5", &url),
            asset(name, "uploaded", "\"3\"", &url),
            asset(name, "uploaded", "999999999999", &url),
            // Not asset objects at all
            "1".into(),
            "null".into(),
            format!("\"{name}\""),
        ] {
            assert_eq!(installer(&assets), Some(None), "{assets:.80}");
        }
        // Malformed tags: no release at all, whatever the assets say
        for bad in ["v9.0", "v9.0.0-beta", "release-9.0.0", "v9.0.0/../x"] {
            let assets = asset(
                &installer_name(bad),
                "uploaded",
                "1",
                &format!("{RELEASE_DOWNLOAD}{bad}/{}", installer_name(bad)),
            );
            assert!(
                newer_release(
                    &with_assets(bad, &page(bad), false, false, &assets),
                    "0.2.0"
                )
                .is_none(),
                "{bad}"
            );
        }
        // `assets` absent or not a list: no installer either
        let good = response(tag, &page(tag), false, false);
        let start = good.find("\"assets\"").unwrap();
        let end = good.find("],").unwrap() + 2;
        let without = format!("{}{}", &good[..start], &good[end..]);
        assert_eq!(
            newer_release(&without, "0.2.0").map(|r| r.installer),
            Some(None)
        );
        let not_list = format!("{}\"assets\":{{}},{}", &good[..start], &good[end..]);
        assert_eq!(
            newer_release(&not_list, "0.2.0").map(|r| r.installer),
            Some(None)
        );
        // Said only when present
        let mut r = newer_release(&good, "0.2.0").unwrap();
        assert!(
            r.title().ends_with("to download Oeca-Setup-9.0.0.exe."),
            "{}",
            r.title()
        );
        r.installer = None;
        assert_eq!(
            r.title(),
            "Oeca update available \u{00B7} v9.0.0. Opens the release page."
        );
    }

    /// The installer upgrades in place and never touches user data: its
    /// identity, scope, output name (the asset name's stem) and
    /// shutdown handshake stay what releases and Oeca expect.
    #[test]
    fn test_installer_upgrades_in_place_and_keeps_user_data() {
        let iss = include_str!("../installer/Oeca.iss");
        let setup = |key: &str| {
            iss.lines()
                .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
                .unwrap_or_else(|| panic!("{key} is set"))
                .trim()
        };
        // Same AppId every release: upgrades replace, never duplicate
        assert_eq!(setup("AppId"), "{{3BF695DD-61F2-4C5E-A27F-02FA3DBC01F9}");
        // Per user, never elevated, the same folder
        assert_eq!(setup("PrivilegesRequired"), "lowest");
        assert!(!iss.contains("PrivilegesRequiredOverridesAllowed"));
        assert_eq!(setup("DefaultDirName"), r"{autopf}\{#AppName}");
        assert!(iss.contains(r#"#define AppExeName   "Oeca.exe""#));
        // The asset name releases carry, before its version is added on upload
        assert_eq!(setup("OutputBaseFilename"), INSTALLER_STEM);
        // The version is the executable's own (Cargo.toml's)
        assert!(iss.contains(r#"GetStringFileInfo(AppExe, "ProductVersion")"#));
        // A running Oeca is asked to quit (its tray Quit), never restarted
        assert_eq!(setup("CloseApplications"), "yes");
        assert_eq!(setup("RestartApplications"), "no");
        assert!(iss.contains(&format!(
            "OecaQuitCommand = ${:04X};",
            crate::tray::Command::Quit as u32
        )));
        assert!(iss.contains("OecaWindowClass = 'OecaWindowClass'"));
        // Opening Oeca afterwards is the user's choice on the last page
        let run = iss
            .lines()
            .find(|l| l.starts_with("Filename: \"{app}"))
            .unwrap();
        assert!(run.contains("postinstall") && run.contains("nowait"));
        // Nothing deletes user data (%LOCALAPPDATA%\Oeca), runs silently or
        // terminates processes
        let lower = iss.to_lowercase();
        for banned in [
            "[installdelete]",
            "[uninstalldelete]",
            "{localappdata}",
            "{userappdata}",
            "deltree",
            "verysilent",
            "taskkill",
            "terminateprocess",
            "powershell",
        ] {
            assert!(!lower.contains(banned), "{banned}");
        }
    }
}
