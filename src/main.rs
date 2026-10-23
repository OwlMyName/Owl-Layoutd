// per-window-layout: раскладка клавиатуры на каждое окно для Hyprland (как в KDE).
// Только стандартная библиотека, ни одной зависимости.

use std::env;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

/// Стартовая раскладка НОВОГО окна по его классу: 0=us 1=ru 2=ua 3=cz.
/// Если класса тут нет, окно стартует с 0.
const RULES: &[(&str, u8)] = &[
    ("wezterm-test7", 0),
    ("com.visualstudio.CodeOSS", 0),
    ("com.ayugram.desktop", 1),
];

struct State {
    cmd_path: String,
    wins: Vec<(u64, u8)>, // (адрес окна, его раскладка)
    cur: Option<u64>,     // окно в фокусе
    curidx: Option<u8>,   // текущая раскладка основной клавиатуры
    pending: u32,         // сколько эхо-событий от собственных переключений ждём
    mainkb: Vec<u8>,      // имя основной клавиатуры
    buf: Vec<u8>,         // переиспользуемый буфер под ответы Хайпра
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn rfind(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).rposition(|w| w == needle)
}

/// "55a8d2b3c4e0" -> число (адрес окна)
fn hex(s: &[u8]) -> Option<u64> {
    let s = s.strip_prefix(b"0x").unwrap_or(s);
    if s.is_empty() || s.len() > 16 {
        return None;
    }
    s.iter()
        .try_fold(0u64, |acc, &b| Some((acc << 4) | (b as char).to_digit(16)? as u64))
}

impl State {
    /// Отправить команду в .socket.sock (как это делает hyprctl); ответ попадает в self.buf
    fn cmd(&mut self, c: &[u8]) -> Option<()> {
        let mut s = UnixStream::connect(&self.cmd_path).ok()?;
        s.set_read_timeout(Some(Duration::from_millis(500))).ok()?;
        s.write_all(c).ok()?;
        self.buf.clear();
        s.take(65536).read_to_end(&mut self.buf).ok()?;
        Some(())
    }

    /// Узнать текущую раскладку основной клавиатуры (и заодно её имя)
    fn query_idx(&mut self) -> Option<u8> {
        self.cmd(b"j/devices")?;
        let b = &self.buf;
        let m = find(b, b"\"main\": true")?;
        let key: &[u8] = b"\"active_layout_index\": ";
        let k = rfind(&b[..m], key)?;
        let digits = &b[k + key.len()..];
        let n = digits.iter().take_while(|c| c.is_ascii_digit()).count();
        let idx: u8 = std::str::from_utf8(&digits[..n]).ok()?.parse().ok()?;

        let nkey: &[u8] = b"\"name\": \"";
        let nk = rfind(&b[..k], nkey)?;
        let rest = &b[nk + nkey.len()..k];
        let end = rest.iter().position(|&c| c == b'"')?;
        self.mainkb.clear();
        self.mainkb.extend_from_slice(&rest[..end]);
        Some(idx)
    }

    /// Переключить все клавиатуры на раскладку idx (если она уже такая, ничего не делаем)
    fn apply(&mut self, idx: u8) {
        if self.curidx == Some(idx) {
            return;
        }
        let c = format!("/switchxkblayout all {idx}");
        if self.cmd(c.as_bytes()).is_some() && self.buf.starts_with(b"ok") {
            self.curidx = Some(idx);
            self.pending += 1; // ждём эхо от самих себя
        }
    }

    fn handle(&mut self, line: &[u8]) {
        let Some(p) = find(line, b">>") else { return };
        let (name, data) = (&line[..p], &line[p + 2..]);
        match name {
            // сменился фокус: вспоминаем раскладку нового окна
            b"activewindowv2" => {
                self.cur = hex(data);
                if let Some(a) = self.cur {
                    let idx = match self.wins.iter().find(|w| w.0 == a) {
                        Some(w) => w.1,
                        None => {
                            self.wins.push((a, 0));
                            0
                        }
                    };
                    self.apply(idx);
                }
            }
            // сменилась раскладка: запоминаем её для окна в фокусе
            b"activelayout" => {
                let kb = data.split(|&b| b == b',').next().unwrap_or(&[]);
                if !self.mainkb.is_empty() && kb != &self.mainkb[..] {
                    return; // чужие "клавиатуры" (кнопки, наушники, ydotool) игнорируем
                }
                if self.pending > 0 {
                    self.pending -= 1; // это эхо нашего же переключения
                    return;
                }
                if let Some(q) = self.query_idx() {
                    self.curidx = Some(q);
                    if let Some(a) = self.cur {
                        if let Some(w) = self.wins.iter_mut().find(|w| w.0 == a) {
                            w.1 = q;
                        }
                    }
                }
            }
            // новое окно: формат "адрес,воркспейс,класс,заголовок"
            b"openwindow" => {
                let mut it = data.splitn(4, |&b| b == b',');
                let (Some(a), Some(_ws), Some(class)) = (it.next(), it.next(), it.next()) else {
                    return;
                };
                let Some(a) = hex(a) else { return };
                let idx = RULES
                    .iter()
                    .find(|r| r.0.as_bytes() == class)
                    .map_or(0, |r| r.1);
                match self.wins.iter_mut().find(|w| w.0 == a) {
                    Some(w) => w.1 = idx,
                    None => self.wins.push((a, idx)),
                }
            }
            // окно закрыто: забываем
            b"closewindow" => {
                if let Some(a) = hex(data) {
                    if let Some(i) = self.wins.iter().position(|w| w.0 == a) {
                        self.wins.swap_remove(i);
                    }
                    if self.cur == Some(a) {
                        self.cur = None;
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
        eprintln!("Запускай внутри Hyprland");
        std::process::exit(1);
    };
    let dir = format!("{rt}/hypr/{sig}");

    let mut st = State {
        cmd_path: format!("{dir}/.socket.sock"),
        wins: Vec::new(),
        cur: None,
        curidx: None,
        pending: 0,
        mainkb: Vec::new(),
        buf: Vec::with_capacity(16384),
    };

    // на старте выравниваем все устройства по основной клавиатуре
    st.curidx = st.query_idx();
    if let Some(i) = st.curidx {
        let _ = st.cmd(format!("/switchxkblayout all {i}").as_bytes());
    }

    let mut ev = UnixStream::connect(format!("{dir}/.socket2.sock"))?;
    let mut buf = [0u8; 4096];
    let mut have = 0;
    loop {
        // блокирующее чтение: пока событий нет, процесс спит и не ест CPU
        let n = match ev.read(&mut buf[have..]) {
            Ok(0) => break, // Хайпр закрылся
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
            have = 0; // строка длиннее буфера, выкидываем
        }
    }
    Ok(())
}