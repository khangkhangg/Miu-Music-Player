//! The guests' page on the Wi-Fi (P.3): a small web server on this PC at
//! http://<this PC>:<port>/p/<token>/ — only that link works, so only people who scanned the
//! party's QR code get in. Windows asks once whether to let it through the firewall.

use std::io::Read;
use std::net::{IpAddr, UdpSocket};
use std::sync::Arc;

use super::web::{handle, Party, Request};

/// Ports tried in turn for the guests' page.
pub const FIRST_PORT: u16 = 8765;
const PORTS_TRIED: u16 = 10;
const MAX_BODY: u64 = 64 * 1024;

pub struct LocalServer {
    server: Arc<tiny_http::Server>,
    port: u16,
}

impl LocalServer {
    /// Serves `party` from the first free port from `port` on (0: any free port).
    pub fn start(party: Arc<Party>, port: u16) -> Result<Self, String> {
        let ports: Vec<u16> = if port == 0 { vec![0] } else { (port..port + PORTS_TRIED).collect() };
        let mut last = String::new();
        let server = ports
            .into_iter()
            .find_map(|p| tiny_http::Server::http(("0.0.0.0", p)).map_err(|e| last = e.to_string()).ok())
            .ok_or(last)?;
        let port = server.server_addr().to_ip().map_or(0, |a| a.port());
        let server = Arc::new(server);
        let s = server.clone();
        std::thread::spawn(move || {
            for req in s.incoming_requests() {
                let party = party.clone();
                // One thread each: a guest page waiting for news must not hold the others up.
                std::thread::spawn(move || serve(&party, req));
            }
        });
        log::info!("party page on port {port}");
        Ok(Self { server, port })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn stop(&self) {
        self.server.unblock();
    }
}

impl Drop for LocalServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn serve(party: &Party, mut req: tiny_http::Request) {
    let url = req.url().to_string();
    let (path, query) = url.split_once('?').unwrap_or((&url, ""));
    let prefix = format!("/p/{}", party.token());
    let response = match path.strip_prefix(&prefix) {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            let mut body = Vec::new();
            let _ = req.as_reader().take(MAX_BODY).read_to_end(&mut body);
            let method = req.method().as_str().to_string();
            handle(party, &Request { method: &method, path: if rest.is_empty() { "/" } else { rest }, query, body: &body }, &title_online)
        }
        _ => super::web::Response { status: 404, content_type: "text/plain", body: b"not found".to_vec() },
    };
    let header = |k: &str, v: &str| tiny_http::Header::from_bytes(k.as_bytes(), v.as_bytes()).ok();
    let mut out = tiny_http::Response::from_data(response.body).with_status_code(response.status);
    for h in [header("Content-Type", response.content_type), header("Cache-Control", "no-store")].into_iter().flatten() {
        out.add_header(h);
    }
    let _ = req.respond(out);
}

/// This PC's address on the local network (the one its default route uses; no packet is sent).
pub fn lan_ip() -> Option<IpAddr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("8.8.8.8:80").ok()?;
    s.local_addr().ok().map(|a| a.ip()).filter(|ip| !ip.is_loopback())
}

/// The link as a QR code (SVG) for the party screen.
pub fn qr_svg(text: &str) -> String {
    match qrcode::QrCode::new(text.as_bytes()) {
        // Just the <svg> element (the XML declaration is no use inside a page).
        Ok(code) => {
            let svg = code.render::<qrcode::render::svg::Color>().min_dimensions(240, 240).quiet_zone(true).build();
            svg.find("<svg").map_or(svg.clone(), |i| svg[i..].to_string())
        }
        Err(_) => String::new(),
    }
}

/// A random link token: 10 letters and digits.
pub fn new_token() -> String {
    const CHARS: &[u8] = b"abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut bytes = [0u8; 10];
    if getrandom::getrandom(&mut bytes).is_err() {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        bytes = (t as u128 ^ 0x5DEECE66D).to_le_bytes()[..10].try_into().unwrap_or([7; 10]);
    }
    bytes.iter().map(|b| CHARS[*b as usize % CHARS.len()] as char).collect()
}

/// Percent-encodes a link to pass it in a query.
pub fn escape(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// A YouTube or Spotify link's title from the service's public oEmbed page.
pub fn title_online(url: &str) -> Option<String> {
    let endpoint = if url.contains("spotify.com") {
        format!("https://open.spotify.com/oembed?url={}", escape(url))
    } else {
        format!("https://www.youtube.com/oembed?format=json&url={}", escape(url))
    };
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(6))).build().into();
    let body = agent.get(&endpoint).call().ok()?.body_mut().read_to_string().ok()?;
    let v: serde_json::Value = serde_json::from_str(&body).ok()?;
    v["title"].as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn the_page_is_served_only_under_the_party_link() {
        let party = Arc::new(crate::party::web::Party::new("tok123"));
        let server = LocalServer::start(party, 0).unwrap();
        let base = format!("http://127.0.0.1:{}", server.port());
        let page = ureq::get(&format!("{base}/p/tok123/")).call().unwrap();
        assert_eq!(page.status(), 200);
        let body = ureq::get(&format!("{base}/p/tok123/api/state?guest=g1")).call().unwrap().body_mut().read_to_string().unwrap();
        let state: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(state["queue"].as_array().unwrap().is_empty());
        let wrong = ureq::get(&format!("{base}/p/other/")).config().http_status_as_error(false).build().call().unwrap();
        assert_eq!(wrong.status(), 404, "another token gets nothing");
        let root = ureq::get(&format!("{base}/")).config().http_status_as_error(false).build().call().unwrap();
        assert_eq!(root.status(), 404);
        server.stop();
    }

    #[test]
    fn a_qr_code_for_the_link() {
        let svg = qr_svg("http://192.168.1.20:8765/p/abc/");
        assert!(svg.starts_with("<svg"), "{}", &svg[..60.min(svg.len())]);
        assert!(svg.len() > 500);
    }

    #[test]
    fn party_links_are_hard_to_guess() {
        let a = new_token();
        assert_eq!(a.len(), 10);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_ne!(a, new_token());
    }

    #[test]
    fn links_are_escaped_for_the_title_lookup() {
        assert_eq!(escape("https://youtu.be/x?a=1&b=2"), "https%3A%2F%2Fyoutu.be%2Fx%3Fa%3D1%26b%3D2");
    }
}
