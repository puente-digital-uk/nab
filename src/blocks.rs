//! Turn OCR word boxes into text people actually want to paste:
//! - blocks (paragraphs, headings, labels) and the one under the pointer;
//! - wrapped lines joined back into paragraphs;
//! - tables, as rows and columns (for Excel).
//!
//! Pure logic, testable on any OS.

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Rect {
    pub l: i32,
    pub t: i32,
    pub r: i32,
    pub b: i32,
}

impl Rect {
    pub const fn new(l: i32, t: i32, r: i32, b: i32) -> Self {
        Rect { l, t, r, b }
    }
    pub fn w(&self) -> i32 {
        self.r - self.l
    }
    pub fn h(&self) -> i32 {
        self.b - self.t
    }
    pub fn area(&self) -> i64 {
        self.w().max(0) as i64 * self.h().max(0) as i64
    }
    pub fn union(&self, o: &Rect) -> Rect {
        Rect::new(self.l.min(o.l), self.t.min(o.t), self.r.max(o.r), self.b.max(o.b))
    }
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.l && x < self.r && y >= self.t && y < self.b
    }
    pub fn inflate(&self, d: i32) -> Rect {
        Rect::new(self.l - d, self.t - d, self.r + d, self.b + d)
    }
    pub fn clamp(&self, w: i32, h: i32) -> Rect {
        Rect::new(self.l.clamp(0, w), self.t.clamp(0, h), self.r.clamp(0, w), self.b.clamp(0, h))
    }
    pub fn offset(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(self.l + dx, self.t + dy, self.r + dx, self.b + dy)
    }
}

/// One recognised word. `line` is the OCR engine's line number.
#[derive(Clone, Debug)]
pub struct Word {
    pub rect: Rect,
    pub line: usize,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub rect: Rect,
    pub line_h: i32,
    pub text: String,
}

impl Block {
    /// The area to highlight and to re-read: the text plus a little margin.
    pub fn padded(&self) -> Rect {
        self.rect.inflate((self.line_h * 2 / 5).max(4))
    }
}

/// What a grab produces: the text to paste, and the cells if it was a table.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Grab {
    pub text: String,
    pub table: Option<Vec<Vec<String>>>,
}

/// A run of words on one line, with no big gap inside it.
struct Segment {
    rect: Rect,
    text: String,
    words: usize,
}

fn median(mut v: Vec<i32>) -> i32 {
    if v.is_empty() {
        return 0;
    }
    v.sort_unstable();
    v[v.len() / 2]
}

/// OCR lines can run straight across columns; cut them where the gap between words is large.
fn segments(words: &[Word]) -> Vec<Segment> {
    let mut lines: std::collections::BTreeMap<usize, Vec<&Word>> = Default::default();
    for w in words.iter().filter(|w| w.rect.w() > 0 && w.rect.h() > 0) {
        lines.entry(w.line).or_default().push(w);
    }
    let mut out = vec![];
    for (_, mut ws) in lines {
        ws.sort_by_key(|w| w.rect.l);
        let h = median(ws.iter().map(|w| w.rect.h()).collect()).max(1);
        let mut cur: Option<Segment> = None;
        for w in ws {
            match cur.as_mut() {
                Some(s) if w.rect.l - s.rect.r <= h * 3 / 2 => {
                    s.rect = s.rect.union(&w.rect);
                    s.text.push(' ');
                    s.text.push_str(&w.text);
                    s.words += 1;
                }
                _ => {
                    out.extend(cur.take());
                    cur = Some(Segment { rect: w.rect, text: w.text.clone(), words: 1 });
                }
            }
        }
        out.extend(cur);
    }
    out
}

/// Two segments belong to the same block if they're stacked closely, overlap sideways,
/// and are about the same text size (so a heading and its paragraph stay separate).
fn same_block(a: &Rect, b: &Rect) -> bool {
    let (ha, hb) = (a.h().max(1), b.h().max(1));
    let small = ha.min(hb);
    let size_ok = ha.max(hb) * 10 <= small * 17; // within 1.7x
    let gap = a.t.max(b.t) - a.b.min(b.b); // > 0: space between them vertically
    let overlap = a.r.min(b.r) - a.l.max(b.l); // > 0: they share some columns
    size_ok && gap <= small * 4 / 5 && overlap > 0
}

fn find(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

pub fn blocks(words: &[Word]) -> Vec<Block> {
    blocks_from(&segments(words))
}

fn blocks_from(segs: &[Segment]) -> Vec<Block> {
    let n = segs.len();
    let mut parent: Vec<usize> = (0..n).collect();
    for i in 0..n {
        for j in i + 1..n {
            if same_block(&segs[i].rect, &segs[j].rect) {
                let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                parent[a] = b;
            }
        }
    }
    let mut groups: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
    for i in 0..n {
        let root = find(&mut parent, i);
        groups.entry(root).or_default().push(i);
    }
    let mut out: Vec<Block> = groups
        .into_values()
        .map(|mut ids| {
            // Reading order: top to bottom, then left to right.
            ids.sort_by_key(|&i| ((segs[i].rect.t + segs[i].rect.b) / 2, segs[i].rect.l));
            let rect = ids.iter().skip(1).fold(segs[ids[0]].rect, |r, &i| r.union(&segs[i].rect));
            let line_h = median(ids.iter().map(|&i| segs[i].rect.h()).collect());
            let text = reflow(&ids.iter().map(|&i| (segs[i].rect, segs[i].text.as_str())).collect::<Vec<_>>());
            Block { rect, line_h, text }
        })
        .collect();
    out.sort_by_key(|b| (b.rect.t, b.rect.l));
    out
}

/// Does this line start a list item ("• Milk", "- Eggs", "1. Intro", "b) Other")?
fn is_list_item(s: &str) -> bool {
    let first = s.split_whitespace().next().unwrap_or("");
    const BULLETS: [char; 16] = ['•', '·', '▪', '◦', '‣', '●', '○', '■', '□', '–', '—', '-', '*', '►', '✓', '✔'];
    if first.chars().count() == 1 && first.starts_with(BULLETS) {
        return s.split_whitespace().nth(1).is_some();
    }
    if first.starts_with(['•', '●', '▪', '◦', '■', '►']) {
        return true; // bullet glued to the word
    }
    let Some(head) = first.strip_suffix(['.', ')']) else { return false };
    let numbered = !head.is_empty() && head.len() <= 3 && head.chars().all(|c| c.is_ascii_digit());
    // "a." or "B)" yes; "J." no (that's an initial: "J. Smith said").
    let paren = first.ends_with(')');
    let lettered = head.chars().count() == 1 && head.chars().all(|c| c.is_lowercase() || (paren && c.is_alphabetic()));
    (numbered || lettered) && s.split_whitespace().nth(1).is_some()
}

/// Join lines that were only broken because the text wrapped. A line break is kept when:
/// the next line's first word would have fitted on this line (so it wasn't a wrap — addresses,
/// short lines, poetry), the next line is a list item or indented differently, or this line ends in ':'.
/// Words hyphenated across a wrap ("infor-" + "mation") are re-joined.
pub fn reflow(lines: &[(Rect, &str)]) -> String {
    let maxw = lines.iter().map(|l| l.0.w()).max().unwrap_or(0);
    // Average character width across the block (one line alone is too noisy).
    let total_chars: usize = lines.iter().map(|l| l.1.trim().chars().count()).sum();
    let char_w = lines.iter().map(|l| l.0.w().max(0)).sum::<i32>() / total_chars.max(1) as i32;
    let mut out = String::new();
    for (i, (r, t)) in lines.iter().enumerate() {
        let t = t.trim();
        if i == 0 {
            out.push_str(t);
            continue;
        }
        let (pr, pt) = (lines[i - 1].0, lines[i - 1].1.trim());
        let first_w = char_w * t.split_whitespace().next().map_or(0, |w| w.chars().count()) as i32;
        let would_fit = (pr.w() + char_w + first_w) * 100 <= maxw * 95;
        let indent = (r.l - pr.l).abs() > r.h().max(pr.h());
        let hard = would_fit || indent || is_list_item(t) || pt.ends_with(':');
        let hyphen = pt.ends_with('-') && pt.chars().rev().nth(1).is_some_and(char::is_alphabetic) && t.chars().next().is_some_and(char::is_lowercase);
        if hard {
            out.push('\n');
        } else if hyphen {
            out.pop();
        } else {
            out.push(' ');
        }
        out.push_str(t);
    }
    out
}

/// If these segments are laid out as a table, its cells (rows of columns).
///
/// Rows: segments whose vertical centre falls inside the row's first segment. Columns: the
/// x-ranges of the fullest rows, merged where they overlap, then neighbouring columns that no
/// row ever uses together are merged too (a left-aligned header over right-aligned numbers).
/// Side-by-side *paragraphs* also look like rows and columns, so cells must be short and most
/// rows must use the same columns.
fn table(segs: &[Segment]) -> Option<Vec<Vec<String>>> {
    if segs.len() < 4 {
        return None;
    }
    let mut order: Vec<usize> = (0..segs.len()).collect();
    order.sort_by_key(|&i| (segs[i].rect.t + segs[i].rect.b, segs[i].rect.l));
    let mut rows: Vec<(Rect, Vec<usize>)> = vec![];
    for i in order {
        let r = segs[i].rect;
        let cy = (r.t + r.b) / 2;
        match rows.last_mut() {
            Some((anchor, ids)) if cy >= anchor.t && cy < anchor.b => ids.push(i),
            _ => rows.push((r, vec![i])),
        }
    }
    for (_, ids) in rows.iter_mut() {
        ids.sort_by_key(|&i| segs[i].rect.l);
    }
    let multi = rows.iter().filter(|r| r.1.len() >= 2).count();
    if multi < 2 || multi * 2 < rows.len() {
        return None;
    }

    // Columns from the fullest rows, so a label spanning several columns can't chain them together.
    let most = rows.iter().map(|r| r.1.len()).max().unwrap_or(0);
    let mut spans: Vec<(i32, i32)> = rows.iter().filter(|r| r.1.len() == most).flat_map(|r| r.1.iter().map(|&i| (segs[i].rect.l, segs[i].rect.r))).collect();
    spans.sort_unstable();
    let mut cols: Vec<(i32, i32)> = vec![];
    for (l, r) in spans {
        match cols.last_mut() {
            Some(c) if l <= c.1 => c.1 = c.1.max(r),
            _ => cols.push((l, r)),
        }
    }

    // Every segment goes in the column it overlaps most (or the nearest one).
    let column_of = |r: &Rect| -> usize {
        (0..cols.len())
            .max_by_key(|&c| {
                let (l, rr) = cols[c];
                let overlap = r.r.min(rr) - r.l.max(l);
                let dist = ((r.l + r.r) - (l + rr)).abs();
                (overlap > 0, overlap, -dist, -(c as i32))
            })
            .unwrap_or(0)
    };
    let mut placed: Vec<Vec<(usize, usize)>> = rows.iter().map(|(_, ids)| ids.iter().map(|&i| (i, column_of(&segs[i].rect))).collect()).collect();

    // Merge neighbouring columns that no row ever uses together.
    let mut c = 0;
    while c + 1 < cols.len() {
        let together = placed.iter().any(|row| row.iter().any(|p| p.1 == c) && row.iter().any(|p| p.1 == c + 1));
        if together {
            c += 1;
            continue;
        }
        cols[c] = (cols[c].0.min(cols[c + 1].0), cols[c].1.max(cols[c + 1].1));
        cols.remove(c + 1);
        for p in placed.iter_mut().flatten() {
            if p.1 > c {
                p.1 -= 1;
            }
        }
    }
    if cols.len() < 2 {
        return None;
    }

    // Most multi-cell rows should use the same columns (a chat or a random dialog won't).
    let mut patterns: std::collections::HashMap<Vec<bool>, usize> = Default::default();
    for row in placed.iter().filter(|r| r.len() >= 2) {
        let mut used = vec![false; cols.len()];
        for p in row {
            used[p.1] = true;
        }
        *patterns.entry(used).or_default() += 1;
    }
    let common = patterns.values().copied().max().unwrap_or(0);
    let words = median(segs.iter().map(|s| s.words as i32).collect());
    if common * 10 < multi * 7 || !(words <= 4 || (cols.len() >= 3 && words <= 6)) {
        return None;
    }

    let grid = placed
        .iter()
        .map(|row| {
            let mut cells = vec![String::new(); cols.len()];
            for &(i, col) in row {
                if !cells[col].is_empty() {
                    cells[col].push(' ');
                }
                cells[col].push_str(&segs[i].text);
            }
            cells
        })
        .collect();
    Some(grid)
}

/// Tab-separated rows: what Excel expects as plain text.
pub fn tsv(rows: &[Vec<String>]) -> String {
    rows.iter()
        .map(|r| r.iter().map(|c| c.replace(['\t', '\n', '\r'], " ")).collect::<Vec<_>>().join("\t"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Everything in a selected area, as the text to paste: a table if it's laid out as one (and
/// `tables` is on — only for areas the user dragged), otherwise its blocks in reading order,
/// each with wrapped lines joined back up.
pub fn render(words: &[Word], tables: bool) -> Grab {
    let segs = segments(words);
    if let Some(rows) = table(&segs).filter(|_| tables) {
        return Grab { text: tsv(&rows), table: Some(rows) };
    }
    let text = blocks_from(&segs).into_iter().map(|b| b.text).filter(|t| !t.is_empty()).collect::<Vec<_>>().join("\n\n");
    Grab { text, table: None }
}

/// The block under (x, y): the smallest one whose padded area contains the point.
pub fn pick(blocks: &[Block], x: i32, y: i32) -> Option<usize> {
    blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| b.padded().contains(x, y))
        .min_by_key(|(_, b)| b.rect.area())
        .map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lay out a line of words of height `h` starting at (x, y). Words are `ch` px per character, 0.3h apart.
    fn line(words: &mut Vec<Word>, id: usize, x: i32, y: i32, h: i32, text: &str) {
        let ch = h / 2;
        let mut x = x;
        for t in text.split(' ') {
            let w = ch * t.chars().count() as i32;
            words.push(Word { rect: Rect::new(x, y, x + w, y + h), line: id, text: t.into() });
            x += w + h * 3 / 10;
        }
    }

    #[test]
    fn paragraph_lines_join_and_paragraphs_split() {
        let mut w = vec![];
        line(&mut w, 0, 100, 100, 20, "The quick brown fox");
        line(&mut w, 1, 100, 128, 20, "jumps over the");
        line(&mut w, 2, 100, 156, 20, "lazy dog.");
        line(&mut w, 3, 100, 230, 20, "Second paragraph here");
        let b = blocks(&w);
        assert_eq!(b.len(), 2, "{b:#?}");
        assert_eq!(b[0].text, "The quick brown fox jumps over the lazy dog."); // wrapped lines joined
        assert_eq!(b[1].text, "Second paragraph here");
        assert_eq!(b[0].line_h, 20);
        assert_eq!(b[0].rect.t, 100);
        assert_eq!(b[0].rect.b, 176);
    }

    #[test]
    fn columns_on_the_same_ocr_line_are_separated() {
        let mut w = vec![];
        // The OCR engine reports each row as one line spanning both columns.
        for (i, y) in [100, 126, 152].into_iter().enumerate() {
            line(&mut w, i, 50, y, 18, "left column text");
            line(&mut w, i, 600, y, 18, "right column text");
        }
        let b = blocks(&w);
        assert_eq!(b.len(), 2, "{b:#?}");
        assert!(b.iter().all(|b| b.text.matches("column").count() == 3), "{b:#?}");
        assert!(b.iter().any(|b| b.text.starts_with("left")));
        assert!(b.iter().any(|b| b.text.starts_with("right")));
    }

    #[test]
    fn heading_is_its_own_block() {
        let mut w = vec![];
        line(&mut w, 0, 100, 60, 40, "Big Heading");
        line(&mut w, 1, 100, 110, 16, "Body text right under the heading");
        line(&mut w, 2, 100, 132, 16, "continues on a second line");
        let b = blocks(&w);
        assert_eq!(b.len(), 2, "{b:#?}");
        assert_eq!(b[0].text, "Big Heading");
        assert_eq!(b[1].text, "Body text right under the heading continues on a second line");
    }

    #[test]
    fn ordering_does_not_depend_on_input_order() {
        let mut w = vec![];
        line(&mut w, 7, 100, 156, 20, "third");
        line(&mut w, 2, 100, 100, 20, "first line");
        line(&mut w, 5, 100, 128, 20, "second line");
        w.reverse();
        let b = blocks(&w);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].text, "first line second line third");
    }

    #[test]
    fn justified_wide_spaces_stay_in_one_segment() {
        let mut w = vec![];
        line(&mut w, 0, 0, 0, 20, "a b");
        w[1].rect = w[1].rect.offset(23, 0); // gap 29px, just under 1.5 line heights → one segment
        assert_eq!(blocks(&w).len(), 1);
        w[1].rect = w[1].rect.offset(10, 0); // now clearly a column gap
        assert_eq!(blocks(&w).len(), 2);
    }

    #[test]
    fn empty_and_degenerate_input() {
        assert!(blocks(&[]).is_empty());
        let w = vec![Word { rect: Rect::new(5, 5, 5, 20), line: 0, text: "x".into() }];
        assert!(blocks(&w).is_empty());
    }

    #[test]
    fn pick_finds_block_under_pointer() {
        let mut w = vec![];
        line(&mut w, 0, 100, 100, 20, "first block");
        line(&mut w, 1, 100, 300, 20, "second block");
        let b = blocks(&w);
        assert_eq!(pick(&b, 110, 110), Some(0));
        assert_eq!(pick(&b, 110, 305), Some(1));
        assert_eq!(pick(&b, 105, 96), Some(0)); // just above the text: inside the padding
        assert_eq!(pick(&b, 110, 200), None); // between blocks
        assert_eq!(pick(&b, 5000, 5000), None);
        assert_eq!(pick(&[], 1, 1), None);
    }

    #[test]
    fn pick_prefers_the_smallest_containing_block() {
        let big = Block { rect: Rect::new(0, 0, 1000, 1000), line_h: 40, text: "big".into() };
        let small = Block { rect: Rect::new(100, 100, 200, 120), line_h: 20, text: "small".into() };
        assert_eq!(pick(&[big.clone(), small.clone()], 150, 110), Some(1));
        assert_eq!(pick(&[small, big], 150, 110), Some(0));
    }

    fn lines<'a>(spec: &[(i32, &'a str)]) -> Vec<(Rect, &'a str)> {
        spec.iter().enumerate().map(|(i, (w, t))| (Rect::new(0, i as i32 * 20, *w, i as i32 * 20 + 16), *t)).collect()
    }

    #[test]
    fn reflow_joins_wrapped_lines() {
        let l = lines(&[(400, "Nab reads the text under your"), (390, "pointer and copies it to the"), (120, "clipboard.")]);
        assert_eq!(reflow(&l), "Nab reads the text under your pointer and copies it to the clipboard.");
    }

    #[test]
    fn reflow_rejoins_hyphenated_words_only() {
        let l = lines(&[(400, "this is important infor-"), (400, "mation for everyone in the"), (400, "Anglo-"), (400, "Saxon world")]);
        // "infor-mation" rejoined; "Anglo-" + capitalised word keeps its hyphen.
        assert_eq!(reflow(&l), "this is important information for everyone in the\nAnglo- Saxon world".replace("\n", " "));
    }

    #[test]
    fn reflow_keeps_lists_short_lines_and_labels() {
        let l = lines(&[(400, "Shopping for the weekend:"), (100, "• Milk"), (110, "• Eggs"), (400, "1. Then go to the bank")]);
        assert_eq!(reflow(&l), "Shopping for the weekend:\n• Milk\n• Eggs\n1. Then go to the bank");
        // ~15px per character: each next line's first word would have fitted, so these aren't wraps.
        let addr = lines(&[(210, "Puente Digital"), (120, "Boscombe"), (405, "Bournemouth, United Kingdom")]);
        assert_eq!(reflow(&addr), "Puente Digital\nBoscombe\nBournemouth, United Kingdom");
        // Indented lines (code, nested text) keep their breaks.
        let mut code = lines(&[(400, "fn main() {"), (400, "println!(hello world);"), (400, "}")]);
        code[1].0 = code[1].0.offset(40, 0);
        assert_eq!(reflow(&code), "fn main() {\nprintln!(hello world);\n}");
        assert_eq!(reflow(&[]), "");
        assert_eq!(reflow(&lines(&[(50, "  solo  ")])), "solo");
    }

    #[test]
    fn list_items_detected() {
        for s in ["• Milk", "- Eggs", "* Bread", "1. Intro", "12) Twelve", "a. First", "B) Second", "•Glued", "– Dash item", "c) third"] {
            assert!(is_list_item(s), "{s}");
        }
        for s in ["-", "1.", "Hello world", "2026. A year", "e.g. this", "-5 degrees", "Mr. Smith", "J. Smith said", "A. Jones"] {
            assert!(!is_list_item(s), "{s}");
        }
    }

    /// A grid of short cells, like a spreadsheet in a PDF.
    fn grid(words: &mut Vec<Word>, rows: &[&[&str]], x0: i32, y0: i32, col_w: i32, h: i32) {
        for (r, row) in rows.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                if !cell.is_empty() {
                    line(words, r, x0 + c as i32 * col_w, y0 + r as i32 * (h * 2), h, cell);
                }
            }
        }
    }

    #[test]
    fn table_becomes_rows_and_columns() {
        let mut w = vec![];
        grid(&mut w, &[&["Region", "Q1", "Q2"], &["North", "1,200", "1,350"], &["South", "980", "1,010"], &["West", "", "770"]], 100, 100, 220, 16);
        let g = render(&w, true);
        let t = g.table.expect("should be a table");
        assert_eq!(t, vec![vec!["Region", "Q1", "Q2"], vec!["North", "1,200", "1,350"], vec!["South", "980", "1,010"], vec!["West", "", "770"]]);
        assert_eq!(g.text, "Region\tQ1\tQ2\nNorth\t1,200\t1,350\nSouth\t980\t1,010\nWest\t\t770");
    }

    #[test]
    fn table_with_multiword_cells_and_a_title_row() {
        let mut w = vec![];
        line(&mut w, 99, 100, 60, 16, "Quarterly results"); // its own OCR line
        grid(&mut w, &[&["Product name", "Units sold", "Unit price"], &["Wireless mouse", "120", "£19.99"], &["USB-C hub", "45", "£34.50"]], 100, 100, 300, 16);
        let t = render(&w, true).table.expect("table");
        assert_eq!(t.len(), 4);
        assert_eq!(t[0][0], "Quarterly results");
        assert_eq!(t[2], vec!["Wireless mouse", "120", "£19.99"]);
    }

    #[test]
    fn two_columns_of_prose_are_not_a_table() {
        let mut w = vec![];
        for (i, y) in [100, 124, 148, 172].into_iter().enumerate() {
            line(&mut w, i, 50, y, 16, "the quick brown fox jumps over the lazy dog");
            line(&mut w, i, 700, y, 16, "pack my box with five dozen liquor jugs now");
        }
        let g = render(&w, true);
        assert!(g.table.is_none());
        let parts: Vec<_> = g.text.split("\n\n").collect();
        assert_eq!(parts.len(), 2, "{:?}", g.text);
        assert!(parts.iter().all(|p| !p.contains('\n')), "each column reflowed into one paragraph");
    }

    #[test]
    fn a_paragraph_and_tiny_inputs_are_not_tables() {
        let mut w = vec![];
        line(&mut w, 0, 100, 100, 20, "Just one line of text");
        assert_eq!(render(&w, true), Grab { text: "Just one line of text".into(), table: None });
        assert_eq!(render(&[], true), Grab::default());
        let mut two = vec![];
        grid(&mut two, &[&["a", "b"]], 0, 0, 200, 16); // one row is not a table
        assert!(render(&two, true).table.is_none());
    }

    #[test]
    fn render_separates_blocks_with_a_blank_line() {
        let mut w = vec![];
        line(&mut w, 0, 100, 100, 20, "First paragraph");
        line(&mut w, 1, 100, 200, 20, "Second paragraph");
        assert_eq!(render(&w, true).text, "First paragraph\n\nSecond paragraph");
    }

    #[test]
    fn right_aligned_numbers_stay_under_their_header() {
        // Excel style: text header left-aligned, numbers right-aligned — they don't overlap.
        let mut w = vec![];
        let rows = [("Region", "Sales"), ("North", "1,200"), ("South", "980"), ("West", "12,770")];
        for (r, (a, b)) in rows.iter().enumerate() {
            let y = 100 + r as i32 * 32;
            line(&mut w, r, 100, y, 16, a);
            let bw = 8 * b.chars().count() as i32;
            if r == 0 {
                line(&mut w, r, 400, y, 16, b); // header at the left of the column
            } else {
                line(&mut w, r, 520 - bw, y, 16, b); // values flush right at x=520
            }
        }
        let t = render(&w, true).table.expect("table");
        assert_eq!(t.iter().map(|r| r.len()).max(), Some(2), "{t:?}");
        assert_eq!(t[0], vec!["Region", "Sales"]);
        assert_eq!(t[3], vec!["West", "12,770"]);
    }

    #[test]
    fn a_spanning_total_row_does_not_merge_columns() {
        let mut w = vec![];
        grid(&mut w, &[&["Item", "Qty", "Price", "Total"], &["Pens", "2", "1.00", "2.00"], &["Paper", "1", "4.50", "4.50"]], 100, 100, 200, 16);
        line(&mut w, 3, 100, 100 + 3 * 32, 16, "Grand total including all items");
        line(&mut w, 3, 700, 100 + 3 * 32, 16, "6.50");
        let t = render(&w, true).table.expect("table");
        assert!(t.iter().all(|r| r.len() == 4), "{t:?}");
        assert_eq!(t[3][3], "6.50");
    }

    #[test]
    fn tables_only_when_asked() {
        let mut w = vec![];
        grid(&mut w, &[&["a", "b", "c"], &["1", "2", "3"], &["4", "5", "6"]], 0, 0, 200, 16);
        assert!(render(&w, true).table.is_some());
        assert!(render(&w, false).table.is_none());
    }

    #[test]
    fn scattered_ui_labels_are_not_a_table() {
        // A dialog: rows with labels in different places each time.
        let mut w = vec![];
        line(&mut w, 0, 100, 100, 16, "Name");
        line(&mut w, 0, 400, 100, 16, "Pablo");
        line(&mut w, 1, 600, 132, 16, "Edit");
        line(&mut w, 1, 900, 132, 16, "Delete");
        line(&mut w, 2, 250, 164, 16, "OK");
        line(&mut w, 2, 1100, 164, 16, "Cancel");
        assert!(render(&w, true).table.is_none());
    }

    #[test]
    fn tsv_escapes_cell_breaks() {
        let rows = vec![vec!["a\tb".to_string(), "c\nd".to_string()], vec!["".into(), "e".into()]];
        assert_eq!(tsv(&rows), "a b\tc d\n\te");
    }

    #[test]
    fn padded_and_rect_helpers() {
        let b = Block { rect: Rect::new(10, 10, 110, 30), line_h: 20, text: String::new() };
        assert_eq!(b.padded(), Rect::new(2, 2, 118, 38));
        let tiny = Block { rect: Rect::new(10, 10, 20, 15), line_h: 5, text: String::new() };
        assert_eq!(tiny.padded(), Rect::new(6, 6, 24, 19));
        assert_eq!(Rect::new(-5, -5, 50, 2000).clamp(40, 100), Rect::new(0, 0, 40, 100));
        assert_eq!(Rect::new(0, 0, 10, 10).area(), 100);
        assert_eq!(Rect::new(10, 10, 0, 0).area(), 0);
    }
}
