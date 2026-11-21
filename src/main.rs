// Owl-Layoutd: per-application (window class) keyboard layout for Hyprland.
// Like KDE's "Application" switching policy. Standard library only, zero dependencies.
//
// Debug:             OWL_DEBUG=1 Owl-Layoutd
// Custom keyboard:   OWL_KEYBOARD=name Owl-Layoutd  (names: `hyprctl devices`)

use std::env;
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

/// Keyboard whose layout we track
const KEYBOARD: &str = "at-translated-set-2-keyboard";

/// Layer-shell namespaces that take keyboard input (Rofi, launchers).
/// Names show up in the debug log as "layer open: ...". Each one gets its own
/// remembered layout, stored under the key "layer:<name>".
const LAYERS: &[&str] = &["rofi"];

/// Layout when no window is focused: 0 = first in the list (us)
const NO_WINDOW_LAYOUT: u8 = 0;

struct State {
    cmd_path: String,
    keyboard: String,
    debug: bool,
    classes: Vec<(Vec<u8>, u8)>, // window class -> remembered layout
    cur: Option<usize>,          // currently focused class
    layer: Option<usize>,        // input layer (Rofi...) currently on screen
    curidx: Option<u8>,          // current keyboard layout
    store: Option<PathBuf>,      // file where state is persisted
    buf: Vec<u8>,                // buffer for Hyprland replies
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Where to store state: ~/.local/state/owl-layoutd/classes
fn store_path() -> Option<PathBuf> {
    let base = match env::var_os("XDG_STATE_HOME") {
        Some(s) if !s.is_empty() => PathBuf::from(s),
        _ => PathBuf::from(env::var_os("HOME")?).join(".local/state"),
    };
    let dir = base.join("owl-layoutd");
    fs::create_dir_all(&dir).ok()?;
    Some(dir.join("classes"))
}

impl State {
    /// Load state from file: lines like "1 com.ayugram.desktop"
    fn load(&mut self) {
        let Some(path) = &self.store else { return };
        let Ok(data) = fs::read(path) else { return };
        for line in data.split(|&b| b == b'\n') {
            let Some(sp) = line.iter().position(|&b| b == b' ') else { continue };
            let Some(idx) = std::str::from_utf8(&line[..sp])
                .ok()
                .and_then(|s| s.parse::<u8>().ok())
            else {
                continue;
            };
            self.classes.push((line[sp + 1..].to_vec(), idx));
        }
    }

    /// Save state to file (via a temp file to avoid corruption on crash)
    fn save(&self) {
        let Some(path) = &self.store else { return };
        let mut out = Vec::new();
        for (class, idx) in &self.classes {
            out.extend_from_slice(idx.to_string().as_bytes());
            out.push(b' ');
            out.extend_from_slice(class);
            out.push(b'\n');
        }
        let tmp = path.with_extension("tmp");
        if fs::write(&tmp, &out).is_ok() {
            let _ = fs::rename(&tmp, path);
        }
    }

    /// Class index in the table (new class gets layout 0)
    fn class_id(&mut self, class: &[u8]) -> usize {
        if let Some(i) = self.classes.iter().position(|c| c.0 == class) {
            return i;
        }
        self.classes.push((class.to_vec(), 0));
        self.classes.len() - 1
    }

    /// Send command to .socket.sock (like hyprctl); reply goes into self.buf
    fn cmd(&mut self, c: &[u8]) -> Option<()> {
        let mut s = UnixStream::connect(&self.cmd_path).ok()?;
        s.set_read_timeout(Some(Duration::from_millis(500))).ok()?;
        s.write_all(c).ok()?;
        self.buf.clear();
        s.take(65536).read_to_end(&mut self.buf).ok()?;
        Some(())
    }

    /// Current layout of our specific keyboard
    fn query_idx(&mut self) -> Option<u8> {
        self.cmd(b"j/devices")?;
        let b = &self.buf;
        let needle = format!("\"name\": \"{}\"", self.keyboard);
        let at = find(b, needle.as_bytes())?;
        // search for the index only within this keyboard's block (up to the next "name")
        let limit = find(&b[at + 1..], b"\"name\": \"").map_or(b.len(), |p| at + 1 + p);
        let key: &[u8] = b"\"active_layout_index\": ";
        let k = at + find(&b[at..limit], key)?;
        let digits = &b[k + key.len()..];
        let n = digits.iter().take_while(|c| c.is_ascii_digit()).count();
        std::str::from_utf8(&digits[..n]).ok()?.parse().ok()
    }

    /// Switch ALL keyboards to layout idx
    fn switch_all(&mut self, idx: u8) -> bool {
        let c = format!("/switchxkblayout all {idx}");
        let sent = self.cmd(c.as_bytes()).is_some();
        let ok = sent && self.buf.starts_with(b"ok");
        if self.debug {
            eprintln!("switch all {idx}: sent={sent} reply={:?}", String::from_utf8_lossy(&self.buf));
        }
        ok
    }

    fn apply(&mut self, idx: u8) {
        if self.curidx == Some(idx) {
            return;
        }
        if self.switch_all(idx) {
            self.curidx = Some(idx);
        }
    }

    /// Check which window is focused when the daemon starts
    fn init_focus(&mut self) {
        if self.cmd(b"j/activewindow").is_none() {
            return;
        }
        let key: &[u8] = b"\"class\": \"";
        let Some(p) = find(&self.buf, key) else { return };
        let rest = &self.buf[p + key.len()..];
        let Some(e) = rest.iter().position(|&c| c == b'"') else { return };
        let class = rest[..e].to_vec();
        if class.is_empty() {
            return;
        }
        let id = self.class_id(&class);
        self.cur = Some(id);
        let idx = self.classes[id].1;
        self.apply(idx);
    }

    fn handle(&mut self, line: &[u8]) {
        let Some(p) = find(line, b">>") else { return };
        let (name, data) = (&line[..p], &line[p + 2..]);
        match name {
            // focus changed: data has the format "class,title"
            b"activewindow" => {
                let class = data.split(|&b| b == b',').next().unwrap_or(&[]);
                if class.is_empty() {
                    self.cur = None;
                    if self.debug {
                        eprintln!("focus: no window, data={:?}", String::from_utf8_lossy(data));
                    }
                    if self.layer.is_none() {
                        self.apply(NO_WINDOW_LAYOUT);
                    }
                } else {
                    let id = self.class_id(class);
                    self.cur = Some(id);
                    let idx = self.classes[id].1;
                    if self.debug {
                        eprintln!("focus: class {:?} wants {idx}, now {:?}",
                            String::from_utf8_lossy(class), self.curidx);
                    }
                    if self.layer.is_none() {
                        self.apply(idx); // while a layer is open, it owns the keyboard
                    }
                }
            }
            // an overlay (Rofi, panels) appeared
            b"openlayer" => {
                if self.debug {
                    eprintln!("layer open: {:?}", String::from_utf8_lossy(data));
                }
                if LAYERS.iter().any(|l| l.as_bytes() == data) {
                    let mut key = b"layer:".to_vec();
                    key.extend_from_slice(data);
                    let id = self.class_id(&key);
                    self.layer = Some(id);
                    let idx = self.classes[id].1;
                    self.apply(idx);
                }
            }
            // an overlay disappeared: give the keyboard back to the window below
            b"closelayer" => {
                if self.debug {
                    eprintln!("layer close: {:?}", String::from_utf8_lossy(data));
                }
                if self.layer.is_some() && LAYERS.iter().any(|l| l.as_bytes() == data) {
                    self.layer = None;
                    let idx = match self.cur {
                        Some(id) => self.classes[id].1,
                        None => NO_WINDOW_LAYOUT,
                    };
                    self.apply(idx);
                }
            }
            // layout changed: remember it for whoever owns the keyboard right now
            b"activelayout" => {
                let kb = data.split(|&b| b == b',').next().unwrap_or(&[]);
                if kb != self.keyboard.as_bytes() {
                    return; // ignore other devices
                }
                let Some(q) = self.query_idx() else { return };
                let owner = self.layer.or(self.cur);
                if self.debug {
                    eprintln!("layout event: now {q}, known {:?}, owner {:?}", self.curidx,
                        owner.map(|i| String::from_utf8_lossy(&self.classes[i].0).into_owned()));
                }
                if self.curidx != Some(q) {
                    // user switched manually: sync the remaining devices
                    self.curidx = Some(q);
                    self.switch_all(q);
                }
                if let Some(id) = owner {
                    if self.classes[id].1 != q {
                        self.classes[id].1 = q;
                        self.save();
                    }
                }
            }
            _ => {}
        }
    }
}

fn main() -> std::io::Result<()> {
    let (Ok(rt), Ok(sig)) = (
        env::var("XDG_RUNTIME_DIR"),
        env::var("HYPRLAND_INSTANCE_SIGNATURE"),
    ) else {
        eprintln!("Run this inside a Hyprland session");
        std::process::exit(1);
    };
    let dir = format!("{rt}/hypr/{sig}");

    let mut st = State {
        cmd_path: format!("{dir}/.socket.sock"),
        keyboard: env::var("OWL_KEYBOARD").unwrap_or_else(|_| KEYBOARD.to_string()),
        debug: env::var_os("OWL_DEBUG").is_some(),
        classes: Vec::new(),
        cur: None,
        layer: None,
        curidx: None,
        store: store_path(),
        buf: Vec::with_capacity(16384),
    };

    st.load();
    st.curidx = st.query_idx();
    match st.curidx {
        Some(i) => {
            if st.debug {
                eprintln!("start: keyboard {:?}, layout {i}, remembered classes: {}",
                    st.keyboard, st.classes.len());
            }
            st.switch_all(i); // sync other "keyboards" to match ours
        }
        None => {
            eprintln!("Keyboard {:?} not found. See `hyprctl devices` for device names", st.keyboard);
            std::process::exit(1);
        }
    }
    st.init_focus();

    let mut ev = UnixStream::connect(format!("{dir}/.socket2.sock"))?;
    let mut buf = [0u8; 4096];
    let mut have = 0;
    loop {
        // blocking read: while there are no events, the process sleeps and uses no CPU
        let n = match ev.read(&mut buf[have..]) {
            Ok(0) => break, // Hyprland closed
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        have += n;
        let mut start = 0;
        while let Some(p) = buf[start..have].iter().position(|&b| b == b'\n') {
            st.handle(&buf[start..start + p]);
            start += p + 1;
        }
        buf.copy_within(start..have, 0);
        have -= start;
        if have == buf.len() {
            have = 0; // line longer than buffer, discard it
        }
    }
    Ok(())
}