//! Text recognition with the OCR engine built into Windows 10/11.

use windows::core::{Error, Result, HSTRING};
use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::{OcrEngine, OcrResult};
use windows::Storage::Streams::DataWriter;
use windows::Win32::Foundation::{E_FAIL, LPARAM};
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
use windows::Win32::UI::Shell::NIM_MODIFY;

use crate::blocks::{self, Grab, Rect, Word};
use crate::{logic, sys};

type Outcome = std::result::Result<Grab, String>;

/// How many results have been reported (lets tests wait for one).
#[cfg(test)]
pub static REPORTS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Reads in flight. A person can't start more than a couple; a flood of forged drag events could.
static READS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
const MAX_READS: usize = 2;

/// Both reads a person can have going at once are in flight.
pub fn busy() -> bool {
    READS.load(std::sync::atomic::Ordering::SeqCst) >= MAX_READS
}

/// Read an area in the background, then hand the result to the UI thread (which owns the clipboard).
/// `fallback` is what the screen scan already found there, used if the closer read finds nothing.
pub fn spawn(bgra: Vec<u8>, w: u32, h: u32, fallback: Option<Grab>) {
    use std::sync::atomic::Ordering::SeqCst;
    if READS.fetch_add(1, SeqCst) >= MAX_READS {
        READS.fetch_sub(1, SeqCst);
        crate::mailbox::post::<Outcome>(crate::WM_OCR_DONE, Err("Still reading the last one — try again in a moment.".into()));
        return;
    }
    let spawned = std::thread::Builder::new().name("nab-read".into()).spawn(move || {
        struct Done;
        impl Drop for Done {
            fn drop(&mut self) {
                READS.fetch_sub(1, SeqCst);
            }
        }
        let _done = Done;
        // Tables only for dragged boxes: a click (which has a fallback) is about one block.
        let tables = fallback.is_none();
        let res = match (read(&bgra, w, h, tables), fallback) {
            (Ok(g), _) if !g.text.is_empty() => Ok(g),
            (_, Some(f)) if !f.text.is_empty() => Ok(f),
            (Ok(_), _) => Err("No text found there. Try a slightly bigger box.".to_string()),
            (Err(e), _) => Err(e.message()),
        };
        // Each result travels in its own message, so overlapping grabs can't overwrite each other.
        crate::mailbox::post::<Outcome>(crate::WM_OCR_DONE, res);
    });
    if spawned.is_err() {
        READS.fetch_sub(1, SeqCst);
        crate::mailbox::post::<Outcome>(crate::WM_OCR_DONE, Err("Couldn't start a background task — try again.".into()));
    }
}

/// Notification title and body for a successful grab, with what it saved (see `logic::saving`).
pub fn summary(g: &Grab) -> (String, String) {
    let chars = g.text.chars().count();
    let s = logic::saving(g.table.is_some());
    let saved = format!(" · ~{} saved", logic::format_saved(s.seconds));
    match &g.table {
        Some(rows) => {
            let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
            (
                format!("Copied a table · {} × {}{saved}", rows.len(), cols),
                format!("Paste into Excel, Word or an email — rows and columns are kept. {} steps skipped.", s.steps),
            )
        }
        None => (format!("Copied {chars} character{}{saved}", if chars == 1 { "" } else { "s" }), logic::preview(&g.text, 180)),
    }
}

/// Runs on the UI thread: copy and notify. A closed module (see `ext`) may take the grab over
/// instead; with none installed the clipboard path below is the only one there is.
pub fn report(lp: LPARAM) {
    let Some(res) = crate::mailbox::take::<Outcome>(crate::WM_OCR_DONE, lp) else { return };
    #[cfg(test)]
    REPORTS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let res = res.and_then(|g| match crate::ext::pro().deliver(&g) {
        Some(handled) => handled.map(|h| (g, Some(h))),
        None => {
            let html = g.table.as_deref().map(logic::html_table);
            sys::set_clipboard(&g.text, html.as_deref()).map(|_| (g, None))
        }
    });
    match res {
        Ok((g, handled)) => {
            sys::record_nab(g.table.is_some());
            crate::home::refresh();
            let (title, body) = match handled {
                Some(h) => (h.title, h.body),
                None => summary(&g),
            };
            sys::tray(NIM_MODIFY, Some((&title, &body)));
        }
        Err(e) => sys::tray(NIM_MODIFY, Some(("Nab", &e))),
    }
}

fn engine() -> Result<OcrEngine> {
    if let Ok(e) = OcrEngine::TryCreateFromUserProfileLanguages() {
        return Ok(e);
    }
    // Fall back to any installed OCR language (English first).
    if let Ok(e) = Language::CreateLanguage(&HSTRING::from("en-US")).and_then(|l| OcrEngine::TryCreateFromLanguage(&l)) {
        return Ok(e);
    }
    for lang in OcrEngine::AvailableRecognizerLanguages()? {
        if let Ok(e) = OcrEngine::TryCreateFromLanguage(&lang) {
            return Ok(e);
        }
    }
    Err(Error::new(E_FAIL, "Windows has no OCR language installed. Add one in Settings › Time & language › Language."))
}

/// The OCR language in use and the ones installed, e.g. "en-GB (installed: en-GB, es-ES)".
#[cfg(test)]
pub fn language() -> String {
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }
    let tag = |l: &Language| l.LanguageTag().map(|t| t.to_string()).unwrap_or_default();
    let using = engine().and_then(|e| e.RecognizerLanguage()).map(|l| tag(&l)).unwrap_or_else(|e| e.message());
    let installed: Vec<String> = OcrEngine::AvailableRecognizerLanguages().map(|v| v.into_iter().map(|l| tag(&l)).collect()).unwrap_or_default();
    format!("{using} (installed: {})", installed.join(", "))
}

fn run(engine: &OcrEngine, img: &[u8], w: u32, h: u32) -> Result<OcrResult> {
    let writer = DataWriter::new()?;
    writer.WriteBytes(img)?;
    let bmp = SoftwareBitmap::CreateCopyFromBuffer(&writer.DetachBuffer()?, BitmapPixelFormat::Bgra8, w as i32, h as i32)?;
    engine.RecognizeAsync(&bmp)?.get()
}

/// Every word in a top-down BGRA image, with its box in the image's pixel coordinates.
/// `upscale` picks how much to enlarge the image first (small text reads better bigger).
fn words_with(bgra: &[u8], w: u32, h: u32, upscale: fn(u32, u32, u32) -> u32) -> Result<Vec<Word>> {
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }
    let engine = engine()?;
    let max = OcrEngine::MaxImageDimension()?;
    // Too big for the engine (e.g. a box across several monitors)? Shrink. Otherwise maybe enlarge.
    let mut grey = bgra.to_vec();
    logic::greyscale(&mut grey);
    let (img, iw, ih, up, down) = match logic::shrink_factor(w, h, max) {
        1 => {
            let s = upscale(w, h, max);
            (logic::upscale(&grey, w, h, s), w * s, h * s, s as f32, 1.0)
        }
        d => {
            let (img, iw, ih) = logic::downscale(&grey, w, h, d);
            (img, iw, ih, 1.0, d as f32)
        }
    };
    let result = run(&engine, &img, iw, ih)?;
    let back = |v: f32| (v * down / up).round() as i32;
    let mut out = vec![];
    for (i, line) in result.Lines()?.into_iter().enumerate() {
        for word in line.Words()? {
            let r = word.BoundingRect()?;
            out.push(Word {
                rect: Rect::new(back(r.X), back(r.Y), back(r.X + r.Width), back(r.Y + r.Height)),
                line: i,
                text: logic::clean_word(&word.Text()?.to_string()),
            });
        }
    }
    percents_by_line(&mut out);
    Ok(out)
}

/// "9%" misread as "90/0": fix it on lines that already have a percentage (see `logic::fix_percents`).
fn percents_by_line(words: &mut [Word]) {
    let mut lines: Vec<usize> = words.iter().map(|w| w.line).collect();
    lines.sort_unstable();
    lines.dedup();
    for line in lines {
        let idx: Vec<usize> = (0..words.len()).filter(|&i| words.get(i).is_some_and(|w| w.line == line)).collect();
        let mut texts: Vec<String> = idx.iter().filter_map(|&i| words.get(i).map(|w| w.text.clone())).collect();
        if logic::fix_percents(&mut texts) {
            for (&i, t) in idx.iter().zip(texts) {
                if let Some(w) = words.get_mut(i) {
                    w.text = t;
                }
            }
        }
    }
}

/// Down each column of a table: the same percentage fix, plus the two number repairs that need
/// the column for evidence — a thousands separator misread ("1,533" as "1.533") and a decimal
/// point lost in a scan ("12.4" as "12 4").
fn columns_cleaned(g: &mut Grab) {
    let Some(rows) = g.table.as_mut() else { return };
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut changed = false;
    for c in 0..cols {
        let mut col: Vec<String> = rows.iter().map(|r| r.get(c).cloned().unwrap_or_default()).collect();
        let fixed = logic::fix_percents(&mut col) | logic::fix_separators(&mut col) | logic::fix_lost_points(&mut col);
        if fixed {
            changed = true;
            for (r, cell) in rows.iter_mut().zip(col) {
                if let Some(slot) = r.get_mut(c) {
                    *slot = cell;
                }
            }
        }
    }
    if changed {
        g.text = blocks::tsv(rows);
    }
}

/// Word positions for finding blocks across a whole monitor (fast, moderate enlargement).
pub fn words(bgra: &[u8], w: u32, h: u32) -> Result<Vec<Word>> {
    words_with(bgra, w, h, logic::scan_scale)
}

/// Read a selected area carefully: a table if it's laid out as one (when `tables`), otherwise clean paragraphs.
pub fn read(bgra: &[u8], w: u32, h: u32, tables: bool) -> Result<Grab> {
    let mut g = blocks::render(&words_with(bgra, w, h, logic::scale_factor)?, tables);
    columns_cleaned(&mut g);
    Ok(g)
}

/// Just the text of an area.
#[cfg(test)]
pub fn recognise(bgra: &[u8], w: u32, h: u32) -> Result<String> {
    read(bgra, w, h, true).map(|g| g.text)
}

#[cfg(test)]
mod tests {
    //! End-to-end: draw known text with GDI, read it back with Windows OCR. Needs a real Windows box.
    use super::*;
    use windows::core::w;
    use windows::Win32::Foundation::COLORREF;
    use windows::Win32::Graphics::Gdi::*;

    /// Render `lines` (dark on light, or inverted) into a BGRA buffer, double-spaced.
    fn render(lines: &[&str], font_px: i32, dark_mode: bool) -> (Vec<u8>, u32, u32) {
        render_pitch(lines, font_px, dark_mode, 200)
    }

    /// Render text at a true font size: `em_px` is the em height, the way apps size text (Windows'
    /// standard 9pt UI text is 12px at 100% scaling). `render` instead takes GDI's cell height,
    /// which for Segoe UI is 1.33x the em, so `render(.., 12, ..)` draws ~9px (6.75pt) text.
    fn render_em(lines: &[&str], em_px: i32, dark_mode: bool) -> (Vec<u8>, u32, u32) {
        render_pitch(lines, -em_px, dark_mode, 200)
    }

    /// Same, with line pitch as a percentage of the font size (130 ≈ normal paragraph spacing).
    /// A negative `size` is an em height (as `CreateFontW` takes it); layout uses the cell height.
    fn render_pitch(lines: &[&str], size: i32, dark_mode: bool, pitch: i32) -> (Vec<u8>, u32, u32) {
        let font_px = if size < 0 { -size * 4 / 3 } else { size }; // Segoe UI: cell = 1.33 em
        let step = font_px * pitch / 100;
        let (w, h) = (1100i32, font_px + step * lines.len().max(1) as i32);
        unsafe {
            let screen = GetDC(None);
            let dc = CreateCompatibleDC(screen);
            let bmp = CreateCompatibleBitmap(screen, w, h);
            ReleaseDC(None, screen);
            SelectObject(dc, bmp);
            let _ = PatBlt(dc, 0, 0, w, h, if dark_mode { BLACKNESS } else { WHITENESS });
            let font = CreateFontW(size, 0, 0, 0, 400, 0, 0, 0, DEFAULT_CHARSET.0 as u32, 0, 0, CLEARTYPE_QUALITY.0 as u32, 0, w!("Segoe UI"));
            SelectObject(dc, font);
            SetTextColor(dc, if dark_mode { COLORREF(0xFFFFFF) } else { COLORREF(0) });
            SetBkMode(dc, TRANSPARENT);
            for (i, line) in lines.iter().enumerate() {
                let wide: Vec<u16> = line.encode_utf16().collect();
                let _ = TextOutW(dc, 12, font_px / 2 + i as i32 * step, &wide);
            }
            let mut bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut px = vec![0u8; (w * h * 4) as usize];
            GetDIBits(dc, bmp, 0, h as u32, Some(px.as_mut_ptr() as _), &mut bmi, DIB_RGB_COLORS);
            let _ = DeleteObject(font);
            let _ = DeleteObject(bmp);
            let _ = DeleteDC(dc);
            (px, w as u32, h as u32)
        }
    }

    fn norm(s: &str) -> String {
        s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase()
    }

    fn check(lines: &[&str], font_px: i32, dark: bool) {
        let (px, w, h) = render_em(lines, font_px, dark);
        let got = recognise(&px, w, h).expect("OCR failed");
        println!("{font_px}px em, dark={dark}: {got:?}");
        assert_eq!(norm(&got), norm(&lines.concat()), "font {font_px}px em, dark={dark}: got {got:?}; OCR language: {}", language());
    }

    #[test]
    fn words_come_back_with_positions() {
        let (px, w, h) = render(&["Alpha Beta Gamma"], 28, false);
        let ws = words(&px, w, h).unwrap();
        let texts: Vec<_> = ws.iter().map(|w| norm(&w.text)).collect();
        assert_eq!(texts, ["alpha", "beta", "gamma"], "{ws:?}");
        // Left to right, inside the image, sensible heights, all on one line.
        assert!(ws.windows(2).all(|p| p[0].rect.r <= p[1].rect.l), "{ws:?}");
        assert!(ws.iter().all(|w| w.rect.l >= 0 && w.rect.r <= 1100 && w.rect.h() >= 10 && w.rect.h() <= 50), "{ws:?}");
        assert!(ws.iter().all(|w| w.line == ws[0].line));
        // Fed through block detection it's one block with the right text.
        let b = crate::blocks::blocks(&ws);
        assert_eq!(b.len(), 1);
        assert_eq!(norm(&b[0].text), "alphabetagamma");
    }

    #[test]
    fn two_paragraphs_become_two_blocks() {
        let (px, w, h) = render_pitch(&["First paragraph line one", "first paragraph line two", "", "", "Second paragraph"], 18, false, 130);
        let b = crate::blocks::blocks(&words(&px, w, h).unwrap());
        assert_eq!(b.len(), 2, "{b:#?}");
        assert_eq!(norm(&b[0].text), "firstparagraphlineonefirstparagraphlinetwo", "wrapped lines joined: {:?}", b[0].text);
        assert!(!b[0].text.contains('\n'));
        assert!(norm(&b[1].text).starts_with("second"));
    }

    /// Draw a grid of cells at fixed column positions.
    fn render_table(rows: &[&[&str]], font_px: i32) -> (Vec<u8>, u32, u32) {
        let (w, h) = (1100i32, font_px * 2 * (rows.len() as i32 + 1));
        unsafe {
            let screen = GetDC(None);
            let dc = CreateCompatibleDC(screen);
            let bmp = CreateCompatibleBitmap(screen, w, h);
            ReleaseDC(None, screen);
            SelectObject(dc, bmp);
            let _ = PatBlt(dc, 0, 0, w, h, WHITENESS);
            let font = CreateFontW(font_px, 0, 0, 0, 400, 0, 0, 0, DEFAULT_CHARSET.0 as u32, 0, 0, CLEARTYPE_QUALITY.0 as u32, 0, w!("Segoe UI"));
            SelectObject(dc, font);
            SetBkMode(dc, TRANSPARENT);
            for (r, row) in rows.iter().enumerate() {
                for (c, cell) in row.iter().enumerate() {
                    let wide: Vec<u16> = cell.encode_utf16().collect();
                    let _ = TextOutW(dc, 20 + c as i32 * 300, font_px / 2 + r as i32 * font_px * 2, &wide);
                }
            }
            let mut bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut px = vec![0u8; (w * h * 4) as usize];
            GetDIBits(dc, bmp, 0, h as u32, Some(px.as_mut_ptr() as _), &mut bmi, DIB_RGB_COLORS);
            let _ = DeleteObject(font);
            let _ = DeleteObject(bmp);
            let _ = DeleteDC(dc);
            (px, w as u32, h as u32)
        }
    }

    #[test]
    fn reads_a_table_as_rows_and_columns() {
        let rows: &[&[&str]] = &[&["Region", "Sales", "Growth"], &["North", "1200", "12%"], &["South", "980", "4%"], &["West", "770", "9%"]];
        let (px, w, h) = render_table(rows, 22);
        let g = read(&px, w, h, true).unwrap();
        let t = g.table.clone().unwrap_or_else(|| panic!("not seen as a table: {:?}", g.text));
        assert_eq!(t.len(), 4, "{t:?}");
        for (got, want) in t.iter().zip(rows) {
            let got: Vec<String> = got.iter().map(|c| norm(c)).collect();
            let want: Vec<String> = want.iter().map(|c| norm(c)).collect();
            assert_eq!(got, want);
        }
        assert_eq!(g.text.lines().count(), 4);
        assert!(g.text.lines().all(|l| l.matches('\t').count() == 2), "{:?}", g.text);
        let (title, _) = summary(&g);
        assert!(title.starts_with("Copied a table · 4 × 3"), "{title}");
    }

    // ---- Preprocessing experiment (not part of the normal run) ------------------------------
    // cargo test ocr_tuning -- --ignored --nocapture
    // Reads small text through a dozen preprocessing variants and prints which ones get it right.

    type Img = (Vec<u8>, u32, u32);

    fn nearest(src: &[u8], w: u32, h: u32, s: u32) -> Img {
        let (nw, nh) = (w * s, h * s);
        let mut out = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                let si = (((y / s) * w + x / s) * 4) as usize;
                let di = ((y * nw + x) * 4) as usize;
                out[di..di + 4].copy_from_slice(&src[si..si + 4]);
            }
        }
        (out, nw, nh)
    }

    fn bilinear(src: &[u8], w: u32, h: u32, s: u32) -> Img {
        (logic::upscale(src, w, h, s), w * s, h * s)
    }

    /// Catmull-Rom bicubic (sharper than bilinear). Input is grey, so only channel 0 is used.
    fn bicubic(src: &[u8], w: u32, h: u32, s: u32) -> Img {
        let (nw, nh) = (w * s, h * s);
        let px = |x: i64, y: i64| src[((y.clamp(0, h as i64 - 1) as u32 * w + x.clamp(0, w as i64 - 1) as u32) * 4) as usize] as f32;
        let k = |t: f32| {
            let t = t.abs();
            if t < 1.0 {
                1.5 * t * t * t - 2.5 * t * t + 1.0
            } else if t < 2.0 {
                -0.5 * t * t * t + 2.5 * t * t - 4.0 * t + 2.0
            } else {
                0.0
            }
        };
        let mut out = vec![255u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            let fy = (y as f32 + 0.5) / s as f32 - 0.5;
            let iy = fy.floor() as i64;
            for x in 0..nw {
                let fx = (x as f32 + 0.5) / s as f32 - 0.5;
                let ix = fx.floor() as i64;
                let mut v = 0.0;
                for dy in -1..=2 {
                    let wy = k(fy - (iy + dy) as f32);
                    for dx in -1..=2 {
                        v += px(ix + dx, iy + dy) * wy * k(fx - (ix + dx) as f32);
                    }
                }
                let v = v.round().clamp(0.0, 255.0) as u8;
                let o = ((y * nw + x) * 4) as usize;
                out[o..o + 3].copy_from_slice(&[v, v, v]);
            }
        }
        (out, nw, nh)
    }

    fn map(img: Img, f: impl Fn(u8) -> u8) -> Img {
        let (mut px, w, h) = img;
        for p in px.as_chunks_mut::<4>().0 {
            let v = f(p[0]);
            p[..3].copy_from_slice(&[v, v, v]);
        }
        (px, w, h)
    }

    /// Stretch the darkest pixel to black and the lightest to white.
    fn stretch(img: Img) -> Img {
        let lo = img.0.chunks(4).map(|p| p[0]).min().unwrap_or(0) as u32;
        let hi = img.0.chunks(4).map(|p| p[0]).max().unwrap_or(255) as u32;
        let span = (hi - lo).max(1);
        map(img, |v| ((v as u32 - lo) * 255 / span) as u8)
    }

    /// Black and white at Otsu's threshold.
    fn binarise(img: Img) -> Img {
        let mut hist = [0u64; 256];
        for p in img.0.chunks(4) {
            hist[p[0] as usize] += 1;
        }
        let total: u64 = hist.iter().sum();
        let sum: f64 = hist.iter().enumerate().map(|(i, &n)| i as f64 * n as f64).sum();
        let (mut wb, mut sb, mut best, mut t) = (0f64, 0f64, 0f64, 128u8);
        for (i, &n) in hist.iter().enumerate() {
            wb += n as f64;
            if wb == 0.0 || wb == total as f64 {
                continue;
            }
            sb += i as f64 * n as f64;
            let (mb, mf) = (sb / wb, (sum - sb) / (total as f64 - wb));
            let between = wb * (total as f64 - wb) * (mb - mf) * (mb - mf);
            if between > best {
                best = between;
                t = i as u8;
            }
        }
        map(img, |v| if v > t { 255 } else { 0 })
    }

    /// Unsharp mask with a 3x3 box blur.
    fn sharpen(img: Img, amount: f32) -> Img {
        let (px, w, h) = img;
        let at = |x: i64, y: i64| px[((y.clamp(0, h as i64 - 1) as u32 * w + x.clamp(0, w as i64 - 1) as u32) * 4) as usize] as f32;
        let mut out = px.clone();
        for y in 0..h as i64 {
            for x in 0..w as i64 {
                let mut blur = 0.0;
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        blur += at(x + dx, y + dy);
                    }
                }
                let v = (at(x, y) + amount * (at(x, y) - blur / 9.0)).round().clamp(0.0, 255.0) as u8;
                let o = ((y as u32 * w + x as u32) * 4) as usize;
                out[o..o + 3].copy_from_slice(&[v, v, v]);
            }
        }
        (out, w, h)
    }

    #[test]
    #[ignore]
    fn ocr_tuning() {
        unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        }
        let engine = engine().unwrap();
        type Variant = (&'static str, fn(&[u8], u32, u32) -> Img);
        let variants: &[Variant] = &[
            ("as shipped (bilinear, auto scale)", |p, w, h| bilinear(p, w, h, logic::scale_factor(w, h, OcrEngine::MaxImageDimension().unwrap_or(2600)))),
            ("nearest x4", |p, w, h| nearest(p, w, h, 4)),
            ("bilinear x2", |p, w, h| bilinear(p, w, h, 2)),
            ("bilinear x3", |p, w, h| bilinear(p, w, h, 3)),
            ("bilinear x4", |p, w, h| bilinear(p, w, h, 4)),
            ("bilinear x6", |p, w, h| bilinear(p, w, h, 6)),
            ("bilinear x8", |p, w, h| bilinear(p, w, h, 8)),
            ("bicubic x3", |p, w, h| bicubic(p, w, h, 3)),
            ("bicubic x4", |p, w, h| bicubic(p, w, h, 4)),
            ("bicubic x6", |p, w, h| bicubic(p, w, h, 6)),
            ("bilinear x4 + stretch", |p, w, h| stretch(bilinear(p, w, h, 4))),
            ("bilinear x4 + sharpen", |p, w, h| sharpen(bilinear(p, w, h, 4), 1.5)),
            ("bilinear x4 + binarise", |p, w, h| binarise(bilinear(p, w, h, 4))),
            ("bicubic x4 + binarise", |p, w, h| binarise(bicubic(p, w, h, 4))),
            ("bicubic x6 + binarise", |p, w, h| binarise(bicubic(p, w, h, 6))),
            ("stretch + bicubic x4 + sharpen", |p, w, h| sharpen(bicubic(&stretch((p.to_vec(), w, h)).0, w, h, 4), 1.0)),
        ];
        let samples: &[(&str, i32, bool)] = &[
            ("Order 7731 shipped", 10, false),
            ("Settings saved at 14:05", 11, false),
            ("Invoice number 48213 due Friday", 12, false),
            ("Invoice number 48213 due Friday", 12, true),
            ("Quarterly growth 9% up from 4%", 13, false),
            ("Open the file menu and choose Export", 9, false),
        ];
        let inputs: Vec<(String, Vec<u8>, u32, u32)> = samples
            .iter()
            .map(|&(text, px, dark)| {
                let (mut img, w, h) = render_em(&[text], px, dark);
                logic::greyscale(&mut img);
                (text.to_string(), img, w, h)
            })
            .collect();
        println!("\n{:<34} score  results by em size (✓ exact, else what OCR read)", "variant");
        for (name, f) in variants {
            let mut score = 0;
            let mut notes = vec![];
            for ((text, img, w, h), (_, px, dark)) in inputs.iter().zip(samples) {
                let (out, ow, oh) = f(img, *w, *h);
                let got = run(&engine, &out, ow, oh).and_then(|r| r.Text()).map(|t| t.to_string()).unwrap_or_else(|e| format!("ERR {}", e.message()));
                if norm(&got) == norm(text) {
                    score += 1;
                    notes.push(format!("{px}px{}✓", if *dark { "d" } else { "" }));
                } else {
                    notes.push(format!("{px}px{}:{got:?}", if *dark { "d" } else { "" }));
                }
            }
            println!("{name:<34} {score}/{}   {}", samples.len(), notes.join("  "));
        }
    }

    #[test]
    fn numbers_fixed_down_a_column_and_in_the_pasted_text() {
        let row = |c: &[&str]| c.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let rows = vec![row(&["Region", "Growth", "Score"]), row(&["North", "12%", "150/0"]), row(&["West", "90/0", "20/1"])];
        let mut g = Grab { text: blocks::tsv(&rows), table: Some(rows) };
        columns_cleaned(&mut g);
        assert_eq!(g.text, "Region\tGrowth\tScore\nNorth\t12%\t150/0\nWest\t9%\t20/1");
        assert_eq!(g.table.as_ref().map(|t| t[2][1].as_str()), Some("9%"));
        // Not a table: nothing to do.
        let mut plain = Grab { text: "90/0".into(), table: None };
        columns_cleaned(&mut plain);
        assert_eq!(plain.text, "90/0");

        // The number repairs run down the same columns, and the pasted text is rebuilt with them.
        // Two commas outvote the one dot (one each would be no evidence either way).
        let rows = vec![row(&["District", "Mains", "Losses"]), row(&["Eastbrook", "1,284", "12 4"]), row(&["Pinner Reach", "1.533", "15.6"]), row(&["Oldcastle", "968", "9.2"]), row(&["Ashford", "2,047", "11.8"])];
        let mut g = Grab { text: blocks::tsv(&rows), table: Some(rows) };
        columns_cleaned(&mut g);
        let t = g.table.as_ref().unwrap();
        assert_eq!((t[2][1].as_str(), t[1][2].as_str()), ("1,533", "12.4"));
        assert!(g.text.contains("Eastbrook\t1,284\t12.4"), "{}", g.text);
    }

    #[test]
    fn notifications_say_what_was_saved() {
        let text = |t: &str| Grab { text: t.into(), table: None };
        assert_eq!(summary(&text("hello")).0, "Copied 5 characters · ~20 s saved");
        assert_eq!(summary(&text("x")).0, "Copied 1 character · ~20 s saved");
        assert_eq!(summary(&text("a b")).1, "a b");
        let rows = vec![vec!["Region".to_string(), "Sales".into()], vec!["North".into(), "1200".into()]];
        let t = Grab { text: blocks::tsv(&rows), table: Some(rows) };
        let (title, body) = summary(&t);
        assert_eq!(title, "Copied a table · 2 × 2 · ~1 min saved");
        assert!(body.ends_with("7 steps skipped."), "{body}");
    }

    #[test]
    fn engine_available() {
        engine().expect("no OCR engine on this machine");
        println!("OCR language: {}", language());
    }

    #[test]
    fn reads_normal_text() {
        check(&["The quick brown fox jumps over the lazy dog"], 28, false);
    }

    #[test]
    fn reads_small_ui_text() {
        check(&["Invoice number 48213 due Friday"], 12, false);
    }

    #[test]
    fn reads_tiny_text() {
        check(&["Order 7731 shipped"], 10, false);
    }

    #[test]
    fn reads_dark_mode_text() {
        check(&["Light text on a dark background"], 16, true);
    }

    #[test]
    fn keeps_multiple_lines_in_order() {
        let (px, w, h) = render(&["First line here", "Second line here", "Third line here"], 20, false);
        let got = recognise(&px, w, h).unwrap();
        let lines: Vec<_> = got.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 3, "{got:?}");
        assert!(lines[0].to_lowercase().starts_with("first"), "{got:?}");
        assert!(lines[2].to_lowercase().starts_with("third"), "{got:?}");
    }

    #[test]
    fn blank_area_gives_empty_string() {
        let (px, w, h) = render(&[""], 20, false);
        assert_eq!(recognise(&px, w, h).unwrap(), "");
    }

    #[test]
    fn minimum_selection_does_not_crash() {
        let px = vec![255u8; 4 * 4 * 4];
        assert!(recognise(&px, 4, 4).is_ok());
    }

    #[test]
    fn huge_selection_is_shrunk_not_rejected() {
        // Wider than any OCR limit, e.g. a box across three 4K monitors.
        let (w, h) = (11520u32, 400u32);
        let px = vec![255u8; (w * h * 4) as usize];
        assert_eq!(recognise(&px, w, h).unwrap(), "");
    }

    #[test]
    fn max_dimension_is_sane() {
        assert!(OcrEngine::MaxImageDimension().unwrap() >= 1000);
    }

    #[test]
    fn many_recognitions_in_parallel() {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                std::thread::spawn(|| {
                    let (px, w, h) = render(&["Parallel run"], 24, false);
                    norm(&recognise(&px, w, h).unwrap())
                })
            })
            .collect();
        for h in handles {
            assert_eq!(h.join().unwrap(), "parallelrun");
        }
    }
}
