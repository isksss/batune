//! kitty graphics protocol (<https://sw.kovidgoyal.net/kitty/graphics-protocol/>):
//! finding the APCs in the byte stream, parsing their keys, joining chunked
//! transfers and decoding the pixels. What the commands do to the grid is in
//! `term/graphics.rs`.

use crate::vt::image::{Image, MAX_IMAGE_PIXELS};
use crate::vt::{base64, iterm};

const ESC: u8 = 0x1b;

/// Largest decoded transfer: an RGBA image of [`MAX_IMAGE_PIXELS`].
const MAX_DATA: usize = MAX_IMAGE_PIXELS as usize * 4;
/// Largest base64 text of one transfer (or one APC), with room for the keys.
const MAX_ENCODED: usize = MAX_DATA / 3 * 4 + 4096;

/// Follows the byte stream to pick out `ESC _ G <body> ESC \`.
///
/// vte drops APC strings without telling the performer, so the bytes still
/// go through vte (which keeps its state in step) and this scanner only
/// notices where a kitty APC ends. Splitting the stream instead would mean
/// holding back bytes that might start an APC, which vte already does.
#[derive(Debug, Default)]
pub(crate) struct ApcScanner {
    state: State,
    body: Vec<u8>,
    overflow: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum State {
    #[default]
    Ground,
    Esc,
    EscApc,
    Body,
    BodyEsc,
}

impl ApcScanner {
    /// Scans `bytes` up to the end of the first complete kitty APC. Returns
    /// how many bytes were scanned and that APC's body, if one ended.
    pub(crate) fn scan(&mut self, bytes: &[u8]) -> (usize, Option<Vec<u8>>) {
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            i += 1;
            match self.state {
                State::Ground => {
                    if let Some(e) = bytes[i - 1..].iter().position(|&c| c == ESC) {
                        i += e;
                        self.state = State::Esc;
                    } else {
                        return (bytes.len(), None);
                    }
                }
                State::Esc => {
                    self.state = if b == b'_' {
                        State::EscApc
                    } else {
                        self.restart(b)
                    }
                }
                State::EscApc if b == b'G' => {
                    self.state = State::Body;
                    self.body.clear();
                    self.overflow = false;
                }
                State::EscApc => self.state = self.restart(b),
                State::Body => {
                    let start = i - 1;
                    let end = bytes[start..]
                        .iter()
                        .position(|&c| matches!(c, ESC | 0x18 | 0x1a))
                        .map_or(bytes.len(), |e| start + e);
                    self.push_body(&bytes[start..end]);
                    i = end;
                    match bytes.get(end) {
                        Some(&ESC) => self.state = State::BodyEsc,
                        // CAN and SUB cancel the string in vte.
                        Some(_) => self.state = State::Ground,
                        None => {}
                    }
                    i += usize::from(end < bytes.len());
                }
                State::BodyEsc if b == b'\\' => {
                    self.state = State::Ground;
                    // An oversized APC is dropped whole: half a command would
                    // be misread.
                    let body = std::mem::take(&mut self.body);
                    return (i, (!self.overflow).then_some(body));
                }
                // Any other ESC sequence aborts the string and starts anew.
                State::BodyEsc => self.state = self.restart(b),
            }
        }
        (bytes.len(), None)
    }

    fn push_body(&mut self, bytes: &[u8]) {
        if self.body.len() + bytes.len() > MAX_ENCODED {
            self.overflow = true;
            self.body = Vec::new();
        } else if !self.overflow {
            self.body.extend_from_slice(bytes);
        }
    }

    fn restart(&self, b: u8) -> State {
        if b == ESC { State::Esc } else { State::Ground }
    }
}

/// The keys of a graphics command. Keys this terminal does not use are
/// ignored, as kitty does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Keys {
    pub(crate) action: u8,
    pub(crate) quiet: u32,
    pub(crate) format: u32,
    pub(crate) medium: u8,
    pub(crate) compression: Option<u8>,
    pub(crate) more: bool,
    /// Pixel size of raw data (`s`, `v`).
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) id: Option<u32>,
    pub(crate) number: Option<u32>,
    pub(crate) placement: Option<u32>,
    /// Source rectangle in pixels (`x`, `y`, `w`, `h`; 0 = to the edge).
    pub(crate) source: [u32; 4],
    /// Size in cells (`c`, `r`; 0 = from the pixel size).
    pub(crate) cols: u32,
    pub(crate) rows: u32,
    /// `C=1`: leave the cursor where it is.
    pub(crate) cursor_stay: bool,
    pub(crate) z: i32,
    pub(crate) delete: u8,
}

impl Default for Keys {
    fn default() -> Self {
        Self {
            action: b't',
            quiet: 0,
            format: 32,
            medium: b'd',
            compression: None,
            more: false,
            width: 0,
            height: 0,
            id: None,
            number: None,
            placement: None,
            source: [0; 4],
            cols: 0,
            rows: 0,
            cursor_stay: false,
            z: 0,
            delete: b'a',
        }
    }
}

/// Splits `k=v,k=v;payload`. `None` when a key or value is malformed.
pub(crate) fn parse(body: &[u8]) -> Option<(Keys, &[u8])> {
    let (control, payload) = match body.iter().position(|&b| b == b';') {
        Some(i) => (&body[..i], &body[i + 1..]),
        None => (body, &[][..]),
    };
    let mut keys = Keys::default();
    for kv in control.split(|&b| b == b',').filter(|kv| !kv.is_empty()) {
        let [k, b'=', v @ ..] = kv else {
            return None;
        };
        let int = || std::str::from_utf8(v).ok()?.parse::<u32>().ok();
        let char = || match v {
            [c] if c.is_ascii_alphabetic() => Some(*c),
            _ => None,
        };
        match k {
            b'a' => keys.action = char()?,
            b't' => keys.medium = char()?,
            b'o' => keys.compression = Some(char()?),
            b'd' => keys.delete = char()?,
            b'q' => keys.quiet = int()?,
            b'f' => keys.format = int()?,
            b'm' => keys.more = int()? == 1,
            b's' => keys.width = int()?,
            b'v' => keys.height = int()?,
            b'i' => keys.id = Some(int()?),
            b'I' => keys.number = Some(int()?),
            b'p' => keys.placement = Some(int()?),
            b'x' => keys.source[0] = int()?,
            b'y' => keys.source[1] = int()?,
            b'w' => keys.source[2] = int()?,
            b'h' => keys.source[3] = int()?,
            b'c' => keys.cols = int()?,
            b'r' => keys.rows = int()?,
            b'C' => keys.cursor_stay = int()? == 1,
            b'z' => keys.z = std::str::from_utf8(v).ok()?.parse().ok()?,
            _ if k.is_ascii_alphabetic() => {}
            _ => return None,
        }
    }
    Some((keys, payload))
}

/// A command whose data (if any) has fully arrived.
#[derive(Debug)]
pub(crate) struct Command {
    pub(crate) keys: Keys,
    /// The decoded payload, or the error to report for it.
    pub(crate) data: Result<Vec<u8>, &'static str>,
}

/// Joins chunked transfers (`m=1`). Later chunks carry only `m` (and maybe
/// `q`); the keys of the first chunk apply to the whole transfer.
#[derive(Debug, Default)]
pub(crate) struct Assembler {
    pending: Option<(Keys, Vec<u8>, bool)>,
}

impl Assembler {
    /// `None` while more chunks are expected or when the APC is malformed.
    pub(crate) fn push(&mut self, body: &[u8]) -> Option<Command> {
        let Some((keys, payload)) = parse(body) else {
            // Its id is unknown, so there is nothing to report against: drop
            // any transfer in progress with it.
            self.pending = None;
            return None;
        };
        let (mut first, mut text, mut too_big) = match self.pending.take() {
            Some((mut first, text, too_big)) => {
                first.quiet = first.quiet.max(keys.quiet);
                (first, text, too_big)
            }
            None => (keys.clone(), Vec::new(), false),
        };
        if text.len() + payload.len() > MAX_ENCODED {
            too_big = true;
            text = Vec::new();
        } else if !too_big {
            text.extend_from_slice(payload);
        }
        if keys.more {
            self.pending = Some((first, text, too_big));
            return None;
        }
        first.more = false;
        let data = if too_big {
            Err("EFBIG:image data too large")
        } else {
            decode_payload(&first, &text)
        };
        Some(Command { keys: first, data })
    }
}

fn decode_payload(keys: &Keys, text: &[u8]) -> Result<Vec<u8>, &'static str> {
    let data = base64::decode(text).ok_or("EINVAL:bad base64 payload")?;
    match keys.compression {
        None => Ok(data),
        Some(b'z') => miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&data, MAX_DATA)
            .map_err(|_| "EINVAL:bad zlib data or image data too large"),
        Some(_) => Err("EINVAL:unknown compression"),
    }
}

/// Turns transmitted data into an image.
pub(crate) fn decode_image(keys: &Keys, data: &[u8]) -> Result<Image, &'static str> {
    // Reading files or shared memory named by the application would let it
    // show (and probe for) anything the terminal can read.
    if keys.medium != b'd' {
        return Err("EINVAL:only direct transmission (t=d) is supported");
    }
    let bpp = match keys.format {
        100 => return iterm::decode(data).ok_or("EBADPNG:cannot decode the PNG image"),
        24 => 3,
        32 => 4,
        _ => return Err("EINVAL:unknown format"),
    };
    let (w, h) = (keys.width, keys.height);
    if w == 0 || h == 0 {
        return Err("EINVAL:s and v are required for raw pixels");
    }
    let pixels = u64::from(w) * u64::from(h);
    if pixels > MAX_IMAGE_PIXELS {
        return Err("EFBIG:image has too many pixels");
    }
    if (data.len() as u64) < pixels * bpp {
        return Err("ENODATA:insufficient image data");
    }
    let data = &data[..(pixels * bpp) as usize];
    let rgba = if bpp == 4 {
        data.to_vec()
    } else {
        data.as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 0xff])
            .collect()
    };
    Image::from_rgba(w, h, rgba).ok_or("EINVAL:bad image size")
}

/// The response to a command, or `None` when none is due: kitty answers
/// only commands that carry an id or number, `q=1` silences OK and `q=2`
/// errors too.
pub(crate) fn reply(keys: &Keys, message: &str) -> Option<String> {
    if keys.id.is_none() && keys.number.is_none() {
        return None;
    }
    if keys.quiet >= 2 || (keys.quiet == 1 && message == "OK") {
        return None;
    }
    let mut ids = Vec::new();
    if let Some(i) = keys.id {
        ids.push(format!("i={i}"));
    }
    if let Some(n) = keys.number {
        ids.push(format!("I={n}"));
    }
    if let Some(p) = keys.placement {
        ids.push(format!("p={p}"));
    }
    Some(format!("\x1b_G{};{message}\x1b\\", ids.join(",")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan_all(scanner: &mut ApcScanner, mut bytes: &[u8]) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        while !bytes.is_empty() {
            let (n, body) = scanner.scan(bytes);
            out.extend(body);
            bytes = &bytes[n..];
        }
        out
    }

    #[test]
    fn scanner_finds_kitty_apcs_split_across_chunks() {
        let stream = b"ab\x1b_Ga=q,i=1;AAAA\x1b\\cd\x1b_Gi=2\x1b\\";
        for cut in 0..stream.len() {
            let mut s = ApcScanner::default();
            let mut got = scan_all(&mut s, &stream[..cut]);
            got.extend(scan_all(&mut s, &stream[cut..]));
            assert_eq!(got, [&b"a=q,i=1;AAAA"[..], b"i=2"], "cut at {cut}");
        }
    }

    #[test]
    fn scanner_stops_right_after_each_apc() {
        let mut s = ApcScanner::default();
        let (n, body) = s.scan(b"x\x1b_Gi=1\x1b\\rest");
        assert_eq!((n, body.as_deref()), (9, Some(&b"i=1"[..])));
    }

    #[test]
    fn scanner_ignores_other_apcs_and_aborted_strings() {
        let mut s = ApcScanner::default();
        assert!(scan_all(&mut s, b"\x1b_Xi=1\x1b\\\x1b_Gi=1\x1b[m\x1b_Gi=1\x18").is_empty());
        assert_eq!(scan_all(&mut s, b"\x1b_Gi=1\x1b\x1b_Gi=2\x1b\\"), [b"i=2"]);
    }

    #[test]
    fn parse_reads_keys_and_payload() {
        let (keys, payload) = parse(b"a=T,f=24,s=2,v=3,i=7,z=-5,C=1;QUJD").unwrap();
        assert_eq!(payload, b"QUJD");
        assert_eq!(
            (keys.action, keys.format, keys.width, keys.height),
            (b'T', 24, 2, 3)
        );
        assert_eq!((keys.id, keys.z, keys.cursor_stay), (Some(7), -5, true));
        assert_eq!(keys.medium, b'd');
    }

    #[test]
    fn parse_rejects_malformed_keys() {
        assert!(parse(b"a=TT").is_none());
        assert!(parse(b"i=x").is_none());
        assert!(parse(b"i").is_none());
        assert!(parse(b"i=-1").is_none());
        assert!(parse(b"Z=1").is_some(), "unknown keys are ignored");
    }

    #[test]
    fn chunks_are_joined_with_the_keys_of_the_first() {
        let mut a = Assembler::default();
        assert!(a.push(b"a=T,f=24,s=1,v=1,i=3,m=1;AQ").is_none());
        let cmd = a.push(b"m=0;ID").unwrap();
        assert_eq!((cmd.keys.action, cmd.keys.id), (b'T', Some(3)));
        assert_eq!(cmd.data, Ok(vec![1, 2, 3]));
    }

    #[test]
    fn a_malformed_chunk_drops_the_transfer() {
        let mut a = Assembler::default();
        assert!(a.push(b"i=3,m=1;AQID").is_none());
        assert!(a.push(b"m=?").is_none());
        let cmd = a.push(b"i=4;").unwrap();
        assert_eq!(cmd.keys.id, Some(4));
    }

    #[test]
    fn zlib_payloads_are_inflated() {
        let packed = miniz_oxide::deflate::compress_to_vec_zlib(&[9; 12], 6);
        let body = format!("o=z;{}", base64::encode(&packed));
        let cmd = Assembler::default().push(body.as_bytes()).unwrap();
        assert_eq!(cmd.data, Ok(vec![9; 12]));
    }

    #[test]
    fn raw_rgb_gets_an_opaque_alpha() {
        let keys = parse(b"f=24,s=1,v=2").unwrap().0;
        let image = decode_image(&keys, &[1, 2, 3, 4, 5, 6]).unwrap();
        assert_eq!(&image.rgba()[..], [1, 2, 3, 255, 4, 5, 6, 255]);
    }

    #[test]
    fn bad_images_are_reported() {
        let keys = |k: &[u8]| parse(k).unwrap().0;
        assert_eq!(
            decode_image(&keys(b"t=f,s=1,v=1"), &[0; 4]).unwrap_err(),
            "EINVAL:only direct transmission (t=d) is supported"
        );
        assert!(decode_image(&keys(b"s=2,v=2"), &[0; 4]).is_err());
        assert!(decode_image(&keys(b"f=32"), &[0; 4]).is_err());
        assert!(decode_image(&keys(b"f=100"), b"not a png").is_err());
    }

    #[test]
    fn replies_name_the_ids_and_honor_quiet() {
        let keys = |k: &[u8]| parse(k).unwrap().0;
        assert_eq!(
            reply(&keys(b"i=1,p=2"), "OK").as_deref(),
            Some("\x1b_Gi=1,p=2;OK\x1b\\")
        );
        assert_eq!(reply(&keys(b"a=T"), "OK"), None);
        assert_eq!(reply(&keys(b"i=1,q=1"), "OK"), None);
        assert!(reply(&keys(b"i=1,q=1"), "ENOENT:x").is_some());
        assert_eq!(reply(&keys(b"i=1,q=2"), "ENOENT:x"), None);
    }
}
