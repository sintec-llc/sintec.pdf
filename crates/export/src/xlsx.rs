//! Export a PDF ▸ Excel (.xlsx), Sintec.PDF: one worksheet per page, in reading order. Tables
//! detected from the page's layout become cell grids (a cell spanning columns is merged);
//! other paragraphs take a row each in the first column, headings in bold. Images are left out.
//!
//! Numbers become numbers only when they can't be misread: `1 250,50`, `1250.5`, `-3`, `12,5`
//! and `1,250.50` do; `1,250` and `1.250` (a thousand or one and a quarter, depending on the
//! country), dates, percentages, codes with leading zeros and anything over 15 digits (account
//! and card numbers, which a spreadsheet would round) stay text.

use crate::{Item, Page, Zip, allowed, esc, items};

/// More pages than this go into one worksheet, each page under a heading row: a workbook of
/// thousands of sheets is slow to open and to navigate.
const MAX_SHEETS: usize = 250;
/// Excel's limits: rows per sheet and characters per cell.
const MAX_ROWS: usize = 1_048_576;
const MAX_CELL_CHARS: usize = 32_767;
/// Column widths, in characters.
const MIN_WIDTH: usize = 8;
const MAX_WIDTH: usize = 60;
/// Digits a spreadsheet keeps exactly (IEEE double): longer numbers stay text.
const MAX_DIGITS: usize = 15;

/// Cell styles (indices into `cellXfs` in [`STYLES`]).
const PLAIN: u8 = 0;
const BOLD: u8 = 1;
const HEADING: u8 = 2;

/// A cell's value.
#[derive(Clone, Debug, PartialEq)]
enum Value {
    Text(String),
    Number(f64),
}

/// One worksheet being built.
#[derive(Default)]
struct Sheet {
    /// Rows of (column, value, style).
    rows: Vec<Vec<(usize, Value, u8)>>,
    /// Merged ranges: (row, first column, last column), all 0-based.
    merges: Vec<(usize, usize, usize)>,
    /// Widest text per column, in characters.
    widths: Vec<usize>,
}

impl Sheet {
    fn push_row(&mut self, cells: Vec<(usize, Value, u8)>) {
        if self.rows.len() >= MAX_ROWS {
            return;
        }
        for (col, v, _) in &cells {
            let w = match v {
                Value::Text(t) => t.lines().map(|l| l.chars().count()).max().unwrap_or(0),
                Value::Number(n) => format_number(*n).len(),
            };
            if self.widths.len() <= *col {
                self.widths.resize(col + 1, 0);
            }
            if let Some(slot) = self.widths.get_mut(*col) {
                *slot = (*slot).max(w);
            }
        }
        self.rows.push(cells);
    }
}

/// Text as a cell value: a number when it is unambiguously one (see the module notes).
fn value(text: &str) -> Value {
    let clean: String = text.trim().chars().filter(|c| allowed(*c)).collect();
    match number(&clean) {
        Some(n) => Value::Number(n),
        None => Value::Text(clean.chars().take(MAX_CELL_CHARS).collect()),
    }
}

/// `s` as a number, or `None` when it isn't plainly one.
fn number(s: &str) -> Option<f64> {
    let s = s.trim();
    let (negative, body) = match s.strip_prefix(['-', '−', '+']) {
        Some(rest) => (s.starts_with(['-', '−']), rest),
        None => (false, s),
    };
    if body.is_empty() || !body.starts_with(|c: char| c.is_ascii_digit()) || !body.ends_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    if !body.chars().all(|c| c.is_ascii_digit() || matches!(c, ',' | '.' | ' ' | '\u{a0}' | '\u{202f}' | '\u{2009}')) {
        return None;
    }
    // The decimal separator: the last of `,` and `.` when both appear (the other groups
    // thousands); a lone one that leaves exactly three digits after it is ambiguous.
    let commas = body.matches(',').count();
    let dots = body.matches('.').count();
    let decimal = match (commas, dots) {
        (0, 0) => None,
        (_, 0) | (0, _) if commas + dots > 1 => return None,
        (1, 0) | (0, 1) => {
            let sep = if commas == 1 { ',' } else { '.' };
            let after = body.rsplit(sep).next().unwrap_or("");
            if after.len() == 3 {
                return None;
            }
            Some(sep)
        }
        _ => {
            let last = body.rfind([',', '.'])?;
            let sep = body.get(last..)?.chars().next()?;
            let other = if sep == ',' { '.' } else { ',' };
            // The decimal separator once, after every grouping one.
            if body.matches(sep).count() != 1 || body.rfind(other).is_some_and(|o| o > last) {
                return None;
            }
            Some(sep)
        }
    };
    let (int_part, frac) = match decimal {
        Some(sep) => body.rsplit_once(sep)?,
        None => (body, ""),
    };
    if !frac.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    // Grouping (spaces, or the separator that isn't the decimal one): 1-3 digits, then threes.
    let groups: Vec<&str> = int_part.split([' ', '\u{a0}', '\u{202f}', '\u{2009}', ',', '.']).collect();
    if groups.len() > 1 && (groups.first().is_none_or(|g| g.is_empty() || g.len() > 3) || groups.iter().skip(1).any(|g| g.len() != 3)) {
        return None;
    }
    let digits: String = groups.concat();
    if !digits.chars().all(|c| c.is_ascii_digit()) || digits.is_empty() {
        return None;
    }
    // A leading zero before more digits is a code (00123), not a number.
    if digits.len() > 1 && digits.starts_with('0') {
        return None;
    }
    if digits.len() + frac.len() > MAX_DIGITS {
        return None;
    }
    let n: f64 = format!("{digits}.{}", if frac.is_empty() { "0" } else { frac }).parse().ok()?;
    n.is_finite().then_some(if negative { -n } else { n })
}

fn format_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 { format!("{}", n as i64) } else { format!("{n}") }
}

/// The spreadsheet column name of a 0-based index: 0 → A, 26 → AA.
fn column_name(mut i: usize) -> String {
    let mut out = Vec::new();
    loop {
        out.push(b'A' + (i % 26) as u8);
        if i < 26 {
            break;
        }
        i = i / 26 - 1;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// A worksheet name: Excel's 31 characters, none of `[]:*?/\`.
fn sheet_name(label: &str, n: usize) -> String {
    let clean: String = label.chars().filter(|c| !matches!(c, '[' | ']' | ':' | '*' | '?' | '/' | '\\') && allowed(*c)).collect();
    let suffix = format!(" {n}");
    let room = 31usize.saturating_sub(suffix.chars().count());
    let base: String = clean.trim().chars().take(room).collect();
    format!("{}{suffix}", if base.is_empty() { "Page" } else { &base })
}

/// The workbook: `pages` in reading order; sheets are named `page_label 1`, `page_label 2`…
pub fn xlsx(pages: &[Page], title: &str, page_label: &str) -> Vec<u8> {
    let one_sheet = pages.len() > MAX_SHEETS;
    let mut sheets: Vec<Sheet> = vec![Sheet::default()];
    let mut page = 1usize;
    if one_sheet {
        sheets[0].push_row(vec![(0, Value::Text(format!("{page_label} 1")), HEADING)]);
    }
    for it in items(pages) {
        let Some(sheet) = sheets.last_mut() else { break };
        match it {
            Item::PageBreak => {
                page += 1;
                if one_sheet {
                    sheet.push_row(Vec::new());
                    sheet.push_row(vec![(0, Value::Text(format!("{page_label} {page}")), HEADING)]);
                } else {
                    sheets.push(Sheet::default());
                }
            }
            Item::Para(b, level) => {
                let text = b.text.split_whitespace().collect::<Vec<_>>().join(" ");
                let style = if level > 0 {
                    HEADING
                } else if b.bold {
                    BOLD
                } else {
                    PLAIN
                };
                // A heading or sentence stays text even when it reads as a number.
                let v = if level > 0 { Value::Text(text.chars().take(MAX_CELL_CHARS).collect()) } else { value(&text) };
                sheet.push_row(vec![(0, v, style)]);
            }
            Item::Table(t) => {
                if !sheet.rows.is_empty() {
                    sheet.push_row(Vec::new());
                }
                for row in &t.rows {
                    let mut cells = Vec::new();
                    let mut col = 0usize;
                    for cell in row {
                        if !cell.text.is_empty() {
                            cells.push((col, value(&cell.text), if cell.bold { BOLD } else { PLAIN }));
                        }
                        if cell.span > 1 {
                            sheet.merges.push((sheet.rows.len(), col, col + cell.span - 1));
                        }
                        col += cell.span.max(1);
                    }
                    sheet.push_row(cells);
                }
                sheet.push_row(Vec::new());
            }
            Item::Img(_) => {}
        }
    }
    let mut z = Zip::default();
    let n = sheets.len();
    let names: Vec<String> = (1..=n).map(|i| sheet_name(page_label, i)).collect();
    let mut types = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/><Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/><Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>",
    );
    for i in 1..=n {
        types.push_str(&format!(
            "<Override PartName=\"/xl/worksheets/sheet{i}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>"
        ));
    }
    types.push_str("</Types>");
    z.add("[Content_Types].xml", types.as_bytes(), true);
    z.add(
        "_rels/.rels",
        b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/></Relationships>",
        true,
    );
    z.add(
        "docProps/core.xml",
        format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>{}</dc:title></cp:coreProperties>", esc(title)).as_bytes(),
        true,
    );
    let mut workbook = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><sheets>",
    );
    let mut rels = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    );
    for (i, name) in names.iter().enumerate() {
        let k = i + 1;
        workbook.push_str(&format!("<sheet name=\"{}\" sheetId=\"{k}\" r:id=\"rId{k}\"/>", esc(name)));
        rels.push_str(&format!(
            "<Relationship Id=\"rId{k}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet{k}.xml\"/>"
        ));
    }
    workbook.push_str("</sheets></workbook>");
    rels.push_str(&format!(
        "<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/></Relationships>",
        n + 1
    ));
    z.add("xl/workbook.xml", workbook.as_bytes(), true);
    z.add("xl/_rels/workbook.xml.rels", rels.as_bytes(), true);
    z.add("xl/styles.xml", STYLES.as_bytes(), true);
    for (i, sheet) in sheets.iter().enumerate() {
        z.add(&format!("xl/worksheets/sheet{}.xml", i + 1), sheet_xml(sheet).as_bytes(), true);
    }
    z.finish()
}

fn sheet_xml(sheet: &Sheet) -> String {
    let mut x = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">",
    );
    if !sheet.widths.is_empty() {
        x.push_str("<cols>");
        for (i, w) in sheet.widths.iter().enumerate() {
            let w = (*w + 2).clamp(MIN_WIDTH, MAX_WIDTH);
            x.push_str(&format!("<col min=\"{0}\" max=\"{0}\" width=\"{w}\" customWidth=\"1\"/>", i + 1));
        }
        x.push_str("</cols>");
    }
    x.push_str("<sheetData>");
    for (r, row) in sheet.rows.iter().enumerate() {
        if row.is_empty() {
            continue;
        }
        x.push_str(&format!("<row r=\"{}\">", r + 1));
        for (col, v, style) in row {
            let reference = format!("{}{}", column_name(*col), r + 1);
            let s = if *style == PLAIN { String::new() } else { format!(" s=\"{style}\"") };
            match v {
                Value::Number(n) => x.push_str(&format!("<c r=\"{reference}\"{s}><v>{}</v></c>", format_number(*n))),
                Value::Text(t) => {
                    x.push_str(&format!("<c r=\"{reference}\"{s} t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c>", esc(t)))
                }
            }
        }
        x.push_str("</row>");
    }
    x.push_str("</sheetData>");
    if !sheet.merges.is_empty() {
        x.push_str(&format!("<mergeCells count=\"{}\">", sheet.merges.len()));
        for (r, a, b) in &sheet.merges {
            x.push_str(&format!("<mergeCell ref=\"{}{2}:{}{2}\"/>", column_name(*a), column_name(*b), r + 1));
        }
        x.push_str("</mergeCells>");
    }
    x.push_str("</worksheet>");
    x
}

/// Fonts: the body, bold, a bold heading. Cell formats 0, 1, 2 use them in that order.
const STYLES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><fonts count=\"3\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font><font><b/><sz val=\"11\"/><name val=\"Calibri\"/></font><font><b/><sz val=\"14\"/><name val=\"Calibri\"/></font></fonts><fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill></fills><borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs><cellXfs count=\"3\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/><xf numFmtId=\"0\" fontId=\"1\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyFont=\"1\"/><xf numFmtId=\"0\" fontId=\"2\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyFont=\"1\"/></cellXfs><cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles></styleSheet>";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Block;

    fn block(text: &str, rect: [f64; 4], bold: bool) -> Block {
        Block { text: text.into(), rect, size: 11.0, bold, italic: false, color: [0.0; 3], rotation: 0.0 }
    }

    #[test]
    fn numbers_only_when_unambiguous() {
        for (s, n) in [
            ("1250", 1250.0),
            ("1 250,50", 1250.5),
            ("1\u{a0}250\u{a0}000", 1_250_000.0),
            ("1250.5", 1250.5),
            ("12,5", 12.5),
            ("1,250.50", 1250.5),
            ("1.250,50", 1250.5),
            ("-3", -3.0),
            ("−42,25", -42.25),
            ("0", 0.0),
            ("0,75", 0.75),
        ] {
            assert_eq!(number(s), Some(n), "{s}");
        }
        for s in [
            "1,250",
            "1.250",
            "01.02.2026",
            "12%",
            "00123",
            "40702810900000012345",
            "1 2 3",
            "12 34",
            "1,2,3",
            "1.2.3",
            "abc",
            "",
            "-",
            "1e5",
            "₽100",
            "1 250,50 руб",
            "1,250,5",
            "12.",
        ] {
            assert_eq!(number(s), None, "{s}");
        }
    }

    #[test]
    fn columns_and_sheet_names() {
        assert_eq!([0, 25, 26, 27, 51, 52, 701, 702].map(column_name), ["A", "Z", "AA", "AB", "AZ", "BA", "ZZ", "AAA"]);
        assert_eq!(sheet_name("Страница", 3), "Страница 3");
        assert_eq!(sheet_name("a[b]:c*d?e/f\\g", 1), "abcdefg 1");
        assert_eq!(sheet_name("", 2), "Page 2");
        assert!(sheet_name(&"x".repeat(100), 1000).chars().count() <= 31);
    }

    #[test]
    fn a_table_becomes_cells_with_numbers_and_merged_headers() {
        // A header band over two columns, then three rows of three columns.
        let mut blocks = vec![block("Invoice 15", [50.0, 760.0, 300.0, 772.0], true)];
        blocks.push(block("Items", [50.0, 700.0, 280.0, 712.0], true));
        for (i, (name, qty, price)) in [("Paper", "10", "1 250,50"), ("Toner", "2", "3 400,00"), ("Pens", "100", "12,5")].iter().enumerate() {
            let y = 680.0 - i as f64 * 20.0;
            blocks.push(block(name, [50.0, y, 120.0, y + 12.0], false));
            blocks.push(block(qty, [150.0, y, 180.0, y + 12.0], false));
            blocks.push(block(price, [250.0, y, 300.0, y + 12.0], false));
        }
        let page = Page { width: 612.0, height: 792.0, blocks, images: Vec::new() };
        let bytes = xlsx(&[page.clone(), page], "Report", "Page");
        assert!(bytes.starts_with(b"PK\x03\x04"));
        let sheets = crate::tests_support::unzip(&bytes);
        let sheet = sheets.iter().find(|(n, _)| n == "xl/worksheets/sheet1.xml").map(|(_, x)| x.clone()).unwrap_or_default();
        assert!(sheet.contains("<t xml:space=\"preserve\">Paper</t>"), "{sheet}");
        assert!(sheet.contains("<v>1250.5</v>") && sheet.contains("<v>3400</v>") && sheet.contains("<v>12.5</v>"), "{sheet}");
        assert!(sheets.iter().any(|(n, _)| n == "xl/worksheets/sheet2.xml"), "one sheet per page");
        let workbook = sheets.iter().find(|(n, _)| n == "xl/workbook.xml").map(|(_, x)| x.clone()).unwrap_or_default();
        assert!(workbook.contains("name=\"Page 1\"") && workbook.contains("name=\"Page 2\""), "{workbook}");
    }

    #[test]
    fn long_documents_share_one_sheet_and_odd_text_is_safe() {
        let pages: Vec<Page> = (0..MAX_SHEETS + 1)
            .map(|i| Page {
                width: 100.0,
                height: 100.0,
                blocks: vec![block(&format!("p{i} \u{1}<&>"), [0.0, 0.0, 50.0, 10.0], false)],
                images: Vec::new(),
            })
            .collect();
        let files = crate::tests_support::unzip(&xlsx(&pages, "t", "Page"));
        assert!(files.iter().filter(|(n, _)| n.starts_with("xl/worksheets/")).count() == 1);
        let sheet = &files.iter().find(|(n, _)| n == "xl/worksheets/sheet1.xml").unwrap_or(&files[0]).1;
        assert!(sheet.contains("Page 251") && sheet.contains("p0 &lt;&amp;&gt;") && !sheet.contains('\u{1}'));
        // No pages still makes a valid workbook with one sheet.
        let empty = crate::tests_support::unzip(&xlsx(&[], "t", "Page"));
        assert!(empty.iter().any(|(n, _)| n == "xl/worksheets/sheet1.xml"));
    }
}
