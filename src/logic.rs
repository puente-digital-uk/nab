//! Platform-independent helpers: selection geometry, pixels, scaling, text clean-up.
//! Kept separate so they can be unit-tested anywhere with `cargo test`.

/// Normalised rectangle (x, y, w, h) from two drag points, clamped to the screen.
/// Returns None if the selection is too small to be intentional.
pub fn selection(a: (i32, i32), b: (i32, i32), sw: i32, sh: i32) -> Option<(u32, u32, u32, u32)> {
    let x0 = a.0.min(b.0).clamp(0, sw);
    let y0 = a.1.min(b.1).clamp(0, sh);
    let x1 = a.0.max(b.0).clamp(0, sw);
    let y1 = a.1.max(b.1).clamp(0, sh);
    let (w, h) = (x1 - x0, y1 - y0);
    if w < 4 || h < 4 {
        return None;
    }
    Some((x0 as u32, y0 as u32, w as u32, h as u32))
}

/// Turn a BGRA buffer into opaque greys (Rec. 601 luma). ClearType draws text with coloured
/// sub-pixel fringes, which OCR reads worse than plain grey edges; GDI also leaves alpha undefined.
pub fn greyscale(bgra: &mut [u8]) {
    for px in bgra.chunks_exact_mut(4) {
        let y = ((px[2] as u32 * 299 + px[1] as u32 * 587 + px[0] as u32 * 114 + 500) / 1000) as u8;
        px[0] = y;
        px[1] = y;
        px[2] = y;
        px[3] = 255;
    }
}

/// Darken a BGRA buffer in place for the overlay background (`pct` = brightness kept).
pub fn dim(bgra: &mut [u8], pct: u16) {
    for c in bgra.iter_mut() {
        *c = (*c as u16 * pct / 100) as u8;
    }
}

/// Build a "Text\0" UTF-16 buffer for the clipboard (CF_UNICODETEXT). Normalises line endings to CRLF.
pub fn clipboard_utf16(text: &str) -> Vec<u16> {
    let crlf = text.replace("\r\n", "\n").replace('\n', "\r\n");
    crlf.encode_utf16().chain(Some(0)).collect()
}

/// Windows OCR is poor on small text; upscale small captures. Respects the engine's max dimension.
pub fn scale_factor(w: u32, h: u32, max_dim: u32) -> u32 {
    let wanted = if h < 60 { 4 } else if h < 150 { 3 } else if h < 400 { 2 } else { 1 };
    let fits = (max_dim / w.max(h).max(1)).max(1);
    wanted.min(fits)
}

/// How much to shrink an image that is bigger than the OCR engine accepts (1 = fits).
pub fn shrink_factor(w: u32, h: u32, max_dim: u32) -> u32 {
    w.max(h).div_ceil(max_dim.max(1)).max(1)
}

/// Box-filter integer downscale of a BGRA buffer (averages each d×d block).
pub fn downscale(src: &[u8], w: u32, h: u32, d: u32) -> (Vec<u8>, u32, u32) {
    if d <= 1 {
        return (src.to_vec(), w, h);
    }
    let (nw, nh) = ((w / d).max(1), (h / d).max(1));
    let mut out = vec![0u8; (nw * nh * 4) as usize];
    for y in 0..nh {
        for x in 0..nw {
            let mut acc = [0u32; 4];
            let mut n = 0;
            for yy in y * d..((y + 1) * d).min(h) {
                for xx in x * d..((x + 1) * d).min(w) {
                    let i = ((yy * w + xx) * 4) as usize;
                    for c in 0..4 {
                        acc[c] += src[i + c] as u32;
                    }
                    n += 1;
                }
            }
            let o = ((y * nw + x) * 4) as usize;
            for c in 0..4 {
                out[o + c] = (acc[c] / n.max(1)) as u8;
            }
        }
    }
    (out, nw, nh)
}

/// Copy into a fixed, NUL-terminated UTF-16 field, truncating without splitting a surrogate pair.
pub fn fill_utf16(dst: &mut [u16], s: &str) {
    let n = dst.len().saturating_sub(1);
    let mut i = 0;
    for c in s.encode_utf16().take(n) {
        dst[i] = c;
        i += 1;
    }
    if i > 0 && (0xD800..0xDC00).contains(&dst[i - 1]) {
        i -= 1;
    }
    if let Some(end) = dst.get_mut(i) {
        *end = 0;
    }
}

/// Scale for the whole-screen scan that finds text blocks: 2x on ordinary-resolution
/// monitors so small UI text is found, 1x on big/high-DPI ones (already sharp, and faster).
pub fn scan_scale(w: u32, h: u32, max_dim: u32) -> u32 {
    if h <= 1200 && w.max(h) * 2 <= max_dim {
        2
    } else {
        1
    }
}

/// Screen point <-> LPARAM, keeping negative coordinates (monitors left of / above the main one).
pub fn pack_point(x: i32, y: i32) -> isize {
    ((y as u16 as u32) << 16 | x as u16 as u32) as isize
}

pub fn unpack_point(v: isize) -> (i32, i32) {
    ((v & 0xffff) as u16 as i16 as i32, ((v >> 16) & 0xffff) as u16 as i16 as i32)
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// An HTML table: Excel, Word and Outlook paste this as real cells.
pub fn html_table(rows: &[Vec<String>]) -> String {
    let mut out = String::from("<table border=\"1\" cellpadding=\"4\" style=\"border-collapse:collapse\">");
    for row in rows {
        out.push_str("<tr>");
        for cell in row {
            out.push_str("<td>");
            out.push_str(&escape_html(cell));
            out.push_str("</td>");
        }
        out.push_str("</tr>");
    }
    out.push_str("</table>");
    out
}

/// Wrap an HTML fragment in the Windows "HTML Format" clipboard envelope (byte offsets, UTF-8).
pub fn cf_html(fragment: &str) -> String {
    const HEADER: &str = "Version:0.9\r\nStartHTML:{0}\r\nEndHTML:{1}\r\nStartFragment:{2}\r\nEndFragment:{3}\r\n";
    let header_len = HEADER.len() - 4 * 3 + 4 * 10; // each {n} becomes a 10-digit number
    let pre = "<html><head><meta charset=\"utf-8\"></head><body>\r\n<!--StartFragment-->"; // or Excel reads £ as Â£
    let post = "<!--EndFragment-->\r\n</body></html>";
    let start_html = header_len;
    let start_frag = start_html + pre.len();
    let end_frag = start_frag + fragment.len();
    let end_html = end_frag + post.len();
    let header = HEADER
        .replace("{0}", &format!("{start_html:010}"))
        .replace("{1}", &format!("{end_html:010}"))
        .replace("{2}", &format!("{start_frag:010}"))
        .replace("{3}", &format!("{end_frag:010}"));
    format!("{header}{pre}{fragment}{post}")
}

/// Bilinear integer upscale of a BGRA buffer. Smooth edges matter: blown-up blocky pixels make
/// Windows OCR misread small text ("w" as "vv", "two" as "Evo").
pub fn upscale(src: &[u8], w: u32, h: u32, s: u32) -> Vec<u8> {
    if s <= 1 || w == 0 || h == 0 {
        return src.to_vec();
    }
    // For each output column/row: the two source samples and the weight (0..=256) of the second.
    let taps = |n: u32| -> Vec<(usize, usize, u32)> {
        (0..n * s)
            .map(|o| {
                // Pixel centres line up: output o sits at source (o + 0.5) / s - 0.5, clamped to the edges.
                let pos = (((2 * o + 1) * 256 + s) / (2 * s)).saturating_sub(128).min((n - 1) * 256);
                let i = (pos / 256) as usize;
                (i, (i + 1).min(n as usize - 1), pos % 256)
            })
            .collect()
    };
    let (xs, ys) = (taps(w), taps(h));
    let (w, nw) = (w as usize, w as usize * s as usize);
    let mut out = vec![0u8; nw * ys.len() * 4];
    for (oy, &(y0, y1, fy)) in ys.iter().enumerate() {
        for (ox, &(x0, x1, fx)) in xs.iter().enumerate() {
            let (a, b, c, d) = ((y0 * w + x0) * 4, (y0 * w + x1) * 4, (y1 * w + x0) * 4, (y1 * w + x1) * 4);
            let o = (oy * nw + ox) * 4;
            for k in 0..4 {
                let top = src[a + k] as u32 * (256 - fx) + src[b + k] as u32 * fx;
                let bottom = src[c + k] as u32 * (256 - fx) + src[d + k] as u32 * fx;
                out[o + k] = ((top * (256 - fy) + bottom * fy + (1 << 15)) >> 16) as u8;
            }
        }
    }
    out
}

/// Characters that may follow a number without being part of it ("9%," or "(4%)").
const TRAILING: [char; 10] = ['.', ',', ';', ':', ')', ']', '?', '!', '"', '\''];

/// Ways OCR misreads "%". With a letter or degree sign they never occur in real text after a number.
const PERCENT_SURE: [&str; 11] = ["o/o", "O/O", "°/o", "°/0", "º/o", "º/0", "0/o", "o/0", "O/0", "°/O", "º/O"];
/// All-digit forms: "90/0" could be "9%", but "150/0" is also a cricket score. Only fixed with context.
const PERCENT_MAYBE: [&str; 2] = ["0/0", "0/O"];

/// `word` with a misread percent sign after a number restored, if it ends in one of `forms`.
fn restore_percent(word: &str, forms: &[&str]) -> Option<String> {
    let core = word.trim_end_matches(TRAILING);
    let tail = &word[core.len()..];
    forms.iter().find_map(|f| core.strip_suffix(f).filter(|n| is_number(n)).map(|n| format!("{n}%{tail}")))
}

/// Fix one OCR word where the reading is unambiguous: "12°/o" → "12%", "5o/o," → "5%,".
pub fn clean_word(word: &str) -> String {
    restore_percent(word, &PERCENT_SURE).unwrap_or_else(|| word.to_string())
}

/// A number with a percent sign: "12%", "-3.5%," …
fn is_percent(word: &str) -> bool {
    word.trim_end_matches(TRAILING).strip_suffix('%').is_some_and(is_number)
}

/// Within one line or one table column: if some cell is already a percentage, read "90/0" in the
/// others as "9%" too. Returns whether anything changed.
pub fn fix_percents(cells: &mut [String]) -> bool {
    if !cells.iter().any(|c| is_percent(c)) {
        return false;
    }
    let mut changed = false;
    for c in cells.iter_mut() {
        if let Some(fixed) = restore_percent(c, &PERCENT_MAYBE) {
            *c = fixed;
            changed = true;
        }
    }
    changed
}

/// "9", "-3", "12.5", "1,200": an optional sign, then digits with single separators, ending in a digit.
fn is_number(s: &str) -> bool {
    let body = s.strip_prefix(['-', '+', '(']).unwrap_or(s);
    let mut prev_digit = false;
    for c in body.chars() {
        match c {
            '0'..='9' => prev_digit = true,
            '.' | ',' if prev_digit => prev_digit = false,
            _ => return false,
        }
    }
    prev_digit
}

/// What one nab saves compared with the usual route for text you can't select: screenshot it,
/// paste it into an OCR or AI tool, wait, copy the result, come back (and for a table, ask for a
/// table and fix the columns in Excel). Nab is one step. Conservative round figures.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Saving {
    pub seconds: u64,
    pub steps: u64,
}

pub fn saving(table: bool) -> Saving {
    if table {
        Saving { seconds: 60, steps: 7 }
    } else {
        Saving { seconds: 20, steps: 5 }
    }
}

/// "45 s", "12 min", "3 h 05 min", "4 d 7 h", "212 d": the unit grows with the total, so it never
/// shows more than three digits.
pub fn format_saved(secs: u64) -> String {
    let (d, h, m) = (secs / 86_400, secs % 86_400 / 3600, secs % 3600 / 60);
    match secs {
        0..=59 => format!("{secs} s"),
        60..=3599 => format!("{m} min"),
        3600..=86_399 => format!("{h} h {m:02} min"),
        _ if d < 100 => format!("{d} d {h} h"),
        _ => format!("{d} d"),
    }
}

/// 52310 → "52,310".
pub fn thousands(n: u64) -> String {
    let digits = n.to_string(); // ASCII, so byte slicing is safe
    let mut groups = vec![];
    let mut end = digits.len();
    while end > 3 {
        groups.push(&digits[end - 3..end]);
        end -= 3;
    }
    groups.push(&digits[..end]);
    groups.reverse();
    groups.join(",")
}

/// Short single-line preview for the notification balloon.
pub fn preview(text: &str, max_chars: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max_chars {
        flat
    } else {
        flat.chars().take(max_chars.saturating_sub(1)).collect::<String>() + "…"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_any_drag_direction() {
        let want = Some((10, 20, 90, 80));
        assert_eq!(selection((10, 20), (100, 100), 1920, 1080), want);
        assert_eq!(selection((100, 100), (10, 20), 1920, 1080), want);
        assert_eq!(selection((10, 100), (100, 20), 1920, 1080), want);
        assert_eq!(selection((100, 20), (10, 100), 1920, 1080), want);
    }

    #[test]
    fn selection_clamped_to_screen() {
        assert_eq!(selection((-50, -50), (2000, 1200), 1920, 1080), Some((0, 0, 1920, 1080)));
        assert_eq!(selection((1900, 1060), (5000, 5000), 1920, 1080), Some((1900, 1060, 20, 20)));
    }

    #[test]
    fn selection_rejects_clicks_and_slivers() {
        assert_eq!(selection((5, 5), (5, 5), 100, 100), None);
        assert_eq!(selection((5, 5), (8, 50), 100, 100), None);
        assert_eq!(selection((5, 5), (50, 7), 100, 100), None);
        assert_eq!(selection((-100, -100), (-10, -10), 100, 100), None);
        assert_eq!(selection((0, 0), (4, 4), 100, 100), Some((0, 0, 4, 4)));
    }

    fn pattern(w: u32, h: u32) -> Vec<u8> {
        (0..w * h).flat_map(|i| [i as u8, (i >> 8) as u8, 7, 0]).collect()
    }

    #[test]
    fn greyscale_makes_opaque_greys() {
        // BGRA: white, black, pure red, pure green, pure blue, a ClearType-ish fringe; alpha is junk.
        let mut px = vec![255, 255, 255, 0, 0, 0, 0, 7, 0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 0, 0, 200, 40, 10, 0];
        greyscale(&mut px);
        let got: Vec<[u8; 4]> = px.chunks(4).map(|p| [p[0], p[1], p[2], p[3]]).collect();
        assert_eq!(got, vec![[255; 4], [0, 0, 0, 255], [76, 76, 76, 255], [150, 150, 150, 255], [29, 29, 29, 255], [49, 49, 49, 255]]);
    }

    #[test]
    fn dim_scales_channels() {
        let mut px = vec![0, 100, 200, 255];
        dim(&mut px, 45);
        assert_eq!(px, vec![0, 45, 90, 114]);
        let mut same = vec![9, 99, 255];
        dim(&mut same, 100);
        assert_eq!(same, vec![9, 99, 255]);
    }

    #[test]
    fn clipboard_text_is_crlf_and_nul_terminated() {
        let to = |s: &str| String::from_utf16(&clipboard_utf16(s)).unwrap();
        assert_eq!(to("a\nb"), "a\r\nb\0");
        assert_eq!(to("a\r\nb"), "a\r\nb\0");
        assert_eq!(to(""), "\0");
        assert_eq!(to("日本\n€"), "日本\r\n€\0");
        assert_eq!(clipboard_utf16("😀").len(), 3); // surrogate pair + NUL
    }

    #[test]
    fn scale_factor_rules() {
        assert_eq!(scale_factor(200, 20, 10_000), 4);
        assert_eq!(scale_factor(200, 100, 10_000), 3);
        assert_eq!(scale_factor(800, 300, 10_000), 2);
        assert_eq!(scale_factor(1920, 1080, 10_000), 1);
        // wide strip: 4x would exceed the max dimension
        assert_eq!(scale_factor(3000, 20, 10_000), 3);
        assert_eq!(scale_factor(9000, 20, 10_000), 1);
        assert_eq!(scale_factor(20_000, 20, 10_000), 1);
        assert_eq!(scale_factor(0, 0, 10_000), 4);
    }

    #[test]
    fn html_table_escapes_cells() {
        let rows = vec![vec!["A&B".to_string(), "<1>".to_string()], vec!["\"q\"".to_string(), "".to_string()]];
        let h = html_table(&rows);
        assert!(h.contains("<tr><td>A&amp;B</td><td>&lt;1&gt;</td></tr><tr><td>&quot;q&quot;</td><td></td></tr>"), "{h}");
        assert!(h.starts_with("<table") && h.ends_with("</table>"));
    }

    #[test]
    fn cf_html_offsets_point_at_the_right_bytes() {
        for frag in ["<table><tr><td>x</td></tr></table>", "<b>£19.99 ✓ 日本</b>", ""] {
            let doc = cf_html(frag);
            let num = |key: &str| -> usize {
                let i = doc.find(key).unwrap() + key.len();
                doc[i..i + 10].parse().unwrap()
            };
            let b = doc.as_bytes();
            assert_eq!(&b[num("StartFragment:")..num("EndFragment:")], frag.as_bytes());
            assert!(doc[num("StartHTML:")..].starts_with("<html>"));
            assert_eq!(num("EndHTML:"), b.len());
            assert!(doc.starts_with("Version:0.9\r\n"));
        }
    }

    #[test]
    fn scan_scale_rules() {
        assert_eq!(scan_scale(1920, 1080, 10_000), 2);
        assert_eq!(scan_scale(1366, 768, 10_000), 2);
        assert_eq!(scan_scale(2560, 1440, 10_000), 1);
        assert_eq!(scan_scale(3840, 2160, 10_000), 1);
        assert_eq!(scan_scale(1920, 1080, 2600), 1); // 2x wouldn't fit the engine
    }

    #[test]
    fn points_survive_packing() {
        for (x, y) in [(0, 0), (1, 2), (-1, -1), (-3840, 1200), (7679, -2160), (32767, -32768)] {
            assert_eq!(unpack_point(pack_point(x, y)), (x, y));
        }
    }

    #[test]
    fn shrink_factor_rules() {
        assert_eq!(shrink_factor(800, 600, 2600), 1);
        assert_eq!(shrink_factor(2600, 10, 2600), 1);
        assert_eq!(shrink_factor(2601, 10, 2600), 2);
        assert_eq!(shrink_factor(11520, 2160, 2600), 5);
        assert_eq!(shrink_factor(10, 30000, 10000), 3);
        assert_eq!(shrink_factor(5, 5, 0), 5);
    }

    #[test]
    fn downscale_averages_blocks() {
        // 4x2 image -> 2x1 at d=2
        let src: Vec<u8> = [0u8, 100, 50, 150, 200, 0, 10, 20].iter().flat_map(|&v| [v, v, v, 255]).collect();
        let (out, w, h) = downscale(&src, 4, 2, 2);
        assert_eq!((w, h), (2, 1));
        assert_eq!(out[0], 75); // (0 + 100 + 200 + 0) / 4
        assert_eq!(out[4], 57); // (50 + 150 + 10 + 20) / 4
        assert_eq!(out[3], 255);
    }

    #[test]
    fn downscale_handles_ragged_edges_and_identity() {
        let src = pattern(7, 5);
        let (out, w, h) = downscale(&src, 7, 5, 3);
        assert_eq!((w, h), (2, 1));
        assert_eq!(out.len(), 2 * 4);
        assert_eq!(downscale(&src, 7, 5, 1).0, src);
        let (tiny, tw, th) = downscale(&src, 7, 5, 50);
        assert_eq!((tw, th, tiny.len()), (1, 1, 4));
    }

    #[test]
    fn fill_utf16_truncates_and_terminates() {
        let mut buf = [1u16; 5];
        fill_utf16(&mut buf, "abcdefgh");
        assert_eq!(String::from_utf16(&buf[..4]).unwrap(), "abcd");
        assert_eq!(buf[4], 0);
        fill_utf16(&mut buf, "hi");
        assert_eq!(&buf[..3], &[b'h' as u16, b'i' as u16, 0]);
        fill_utf16(&mut buf, "");
        assert_eq!(buf[0], 0);
        fill_utf16(&mut [], "no room"); // must not panic
    }

    #[test]
    fn fill_utf16_never_splits_a_surrogate_pair() {
        let mut buf = [1u16; 4];
        fill_utf16(&mut buf, "ab😀");
        assert_eq!(String::from_utf16(&buf[..2]).unwrap(), "ab");
        assert_eq!(buf[2], 0);
        let mut fits = [1u16; 5];
        fill_utf16(&mut fits, "ab😀");
        assert_eq!(String::from_utf16(&fits[..4]).unwrap(), "ab😀");
    }

    #[test]
    fn upscale_identity_and_size() {
        let src = pattern(7, 3);
        assert_eq!(upscale(&src, 7, 3, 1), src);
        assert_eq!(upscale(&src, 7, 3, 0), src);
        assert_eq!(upscale(&[], 0, 0, 3), Vec::<u8>::new());
        for s in 2..=4 {
            assert_eq!(upscale(&src, 7, 3, s).len(), src.len() * (s * s) as usize);
        }
        // A single pixel just becomes a bigger pixel.
        assert_eq!(upscale(&[9, 8, 7, 255], 1, 1, 3), [9, 8, 7, 255].repeat(9));
    }

    #[test]
    fn upscale_keeps_flat_areas_flat() {
        let src = [200u8, 100, 50, 255].repeat(5 * 4);
        let big = upscale(&src, 5, 4, 4);
        assert!(big.chunks(4).all(|p| p == [200, 100, 50, 255]));
    }

    #[test]
    fn upscale_edges_are_smooth_and_never_overshoot() {
        // A black | white step, 2x1: the enlarged row ramps up instead of jumping, and its ends stay put.
        let src = vec![0, 0, 0, 255, 255, 255, 255, 255];
        for s in 2..=4u32 {
            let big = upscale(&src, 2, 1, s);
            let row: Vec<u8> = big.chunks(4).take((2 * s) as usize).map(|p| p[0]).collect();
            assert_eq!(row[0], 0, "{row:?}");
            assert_eq!(*row.last().unwrap(), 255, "{row:?}");
            assert!(row.windows(2).all(|p| p[0] <= p[1]), "not monotonic: {row:?}");
            assert!(row.iter().any(|&v| v > 0 && v < 255), "no in-between greys: {row:?}");
            // Every output row is the same (the image is one pixel tall).
            assert!(big.chunks((8 * s) as usize).all(|r| r == &big[..(8 * s) as usize]));
            assert!(big.chunks(4).all(|p| p[3] == 255));
        }
        // Exact values at 2x: samples sit a quarter and three quarters of the way across.
        let row: Vec<u8> = upscale(&src, 2, 1, 2).chunks(4).take(4).map(|p| p[0]).collect();
        assert_eq!(row, [0, 64, 191, 255]);
        // Symmetric: the middle of a 3-wide black-white-black line is its brightest part. At an odd
        // scale one output pixel sits exactly on the source centre and keeps its full value.
        let line = [[0u8, 0, 0, 255], [255; 4], [0, 0, 0, 255]].concat();
        for s in 2..=4u32 {
            let row: Vec<u8> = upscale(&line, 3, 1, s).chunks(4).take(3 * s as usize).map(|p| p[0]).collect();
            assert_eq!(row, row.iter().rev().copied().collect::<Vec<_>>(), "{row:?}");
            let peak = *row.iter().max().unwrap();
            assert_eq!(row[(3 * s as usize) / 2], peak, "{row:?}");
            // The nearest sample is at most a quarter of a source pixel from the centre.
            assert!(peak as u32 >= 255 - 255 / (2 * s) - 1, "{row:?}");
            if s % 2 == 1 {
                assert_eq!(peak, 255, "{row:?}");
            }
        }
    }

    #[test]
    fn upscale_values_stay_within_their_neighbours() {
        let (w, h, s) = (7u32, 3u32, 3u32);
        let src = pattern(w, h);
        let big = upscale(&src, w, h, s);
        for y in 0..h * s {
            for x in 0..w * s {
                // Bilinear output is bounded by the 2x2 source pixels around it.
                let (sx, sy) = ((x / s) as i64, (y / s) as i64);
                for k in 0..4 {
                    let mut near = vec![];
                    for yy in (sy - 1).max(0)..=(sy + 1).min(h as i64 - 1) {
                        for xx in (sx - 1).max(0)..=(sx + 1).min(w as i64 - 1) {
                            near.push(src[((yy as u32 * w + xx as u32) * 4) as usize + k]);
                        }
                    }
                    let v = big[((y * w * s + x) * 4) as usize + k];
                    assert!(v >= *near.iter().min().unwrap() && v <= *near.iter().max().unwrap(), "({x},{y})[{k}] = {v}");
                }
            }
        }
    }

    #[test]
    fn percent_signs_misread_by_ocr_are_restored() {
        for (seen, want) in [
            ("12o/o", "12%"),
            ("-3o/o", "-3%"),
            ("+4O/0", "+4%"),
            ("12.5°/o", "12.5%"),
            ("7º/o", "7%"),
            ("1,200o/0", "1,200%"),
            ("50/o", "5%"),
            ("9o/o,", "9%,"),
            ("9o/o?", "9%?"),
            ("(4o/o)", "(4%)"),
        ] {
            assert_eq!(clean_word(seen), want, "{seen}");
        }
        // Left alone: already right, all-digit forms (no context here), fractions, dates, scores, paths.
        for same in ["9%", "90/0", "150/0", "10/0", "o/o", "0/0", "1/0", "24/7", "50/50", "10/10/0", "0.0.0.0/0", "Gi1/0/0", "abco/o", "1.o/o", "Growth", "", "-", "1.", "°/o"] {
            assert_eq!(clean_word(same), same, "{same}");
        }
    }

    #[test]
    fn digit_percent_misreads_need_a_percentage_nearby() {
        let v = |c: &[&str]| c.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // A column of percentages: the misread one is fixed, the header and others untouched.
        let mut col = v(&["Growth", "12%", "4%", "90/0"]);
        assert!(fix_percents(&mut col));
        assert_eq!(col, v(&["Growth", "12%", "4%", "9%"]));
        let mut line = v(&["up", "9%", "from", "40/0,"]);
        assert!(fix_percents(&mut line));
        assert_eq!(line, v(&["up", "9%", "from", "4%,"]));
        // No percentage nearby: a cricket score or a ratio stays as it is.
        let mut score = v(&["England", "150/0", "after", "20", "overs"]);
        assert!(!fix_percents(&mut score));
        assert_eq!(score, v(&["England", "150/0", "after", "20", "overs"]));
        // "%" on its own, or in a word, isn't a percentage.
        let mut odd = v(&["%", "abc%", "90/0"]);
        assert!(!fix_percents(&mut odd));
        assert_eq!(odd[2], "90/0");
        assert!(!fix_percents(&mut []));
        // Already fine: nothing to change.
        let mut fine = v(&["12%", "4%"]);
        assert!(!fix_percents(&mut fine));
    }

    #[test]
    fn savings_are_fixed_per_nab() {
        assert_eq!(saving(false), Saving { seconds: 20, steps: 5 });
        assert_eq!(saving(true), Saving { seconds: 60, steps: 7 });
        assert!(saving(true).seconds > saving(false).seconds && saving(true).steps > saving(false).steps);
    }

    #[test]
    fn thousands_separators() {
        for (n, s) in [(0, "0"), (7, "7"), (999, "999"), (1000, "1,000"), (52310, "52,310"), (1234567, "1,234,567")] {
            assert_eq!(thousands(n), s);
        }
        assert_eq!(thousands(u64::MAX), "18,446,744,073,709,551,615");
    }

    #[test]
    fn time_saved_grows_its_unit() {
        assert_eq!(format_saved(0), "0 s");
        assert_eq!(format_saved(59), "59 s");
        assert_eq!(format_saved(60), "1 min");
        assert_eq!(format_saved(3599), "59 min");
        assert_eq!(format_saved(3600), "1 h 00 min");
        assert_eq!(format_saved(3 * 3600 + 5 * 60 + 59), "3 h 05 min");
        assert_eq!(format_saved(86_399), "23 h 59 min");
        assert_eq!(format_saved(86_400), "1 d 0 h");
        assert_eq!(format_saved(4 * 86_400 + 7 * 3600 + 1234), "4 d 7 h");
        assert_eq!(format_saved(99 * 86_400 + 23 * 3600), "99 d 23 h");
        assert_eq!(format_saved(212 * 86_400 + 5000), "212 d");
        assert_eq!(format_saved(u64::MAX), format!("{} d", u64::MAX / 86_400));
        // Never more than three digits before the unit, for any total under 999 days.
        for secs in (0..999 * 86_400).step_by(7919) {
            let lead: String = format_saved(secs).chars().take_while(|c| c.is_ascii_digit()).collect();
            assert!(lead.len() <= 3, "{secs} -> {}", format_saved(secs));
        }
    }

    #[test]
    fn preview_truncates_on_char_boundaries() {
        assert_eq!(preview("a\n b\tc", 50), "a b c");
        assert_eq!(preview("abcdefghij", 5), "abcd…");
        assert_eq!(preview("ñandú ñandú ñandú", 6), "ñandú…");
        assert_eq!(preview("", 5), "");
    }
}
