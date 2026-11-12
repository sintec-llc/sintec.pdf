//! Excel and Word export of tables (Sintec.PDF): a table of aligned text and a table drawn with
//! lines come out as cells, merges and borders.

use std::sync::Arc;

use crate::{Session, compare};

/// A one-page PDF with `content` and Helvetica as /F1.
fn pdf_with(content: &str) -> Vec<u8> {
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".into(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
    ];
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        pdf.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    pdf
}

#[test]
fn exporting_to_excel_puts_a_table_into_cells() {
    let mut content = String::from("BT /F1 16 Tf 72 740 Td (Price list) Tj ET ");
    for (i, (item, qty, price)) in
        [("Item", "Qty", "Price"), ("Paper", "10", "1250.50"), ("Toner", "2", "3400"), ("Pens", "100", "12.5")].iter().enumerate()
    {
        let y = 700 - i * 20;
        content.push_str(&format!(
            "BT /F1 11 Tf 72 {y} Td ({item}) Tj ET BT /F1 11 Tf 200 {y} Td ({qty}) Tj ET BT /F1 11 Tf 300 {y} Td ({price}) Tj ET "
        ));
    }
    let pdf = pdf_with(&content);
    let mut s = Session::new();
    let id = s.open("prices.pdf", None, Arc::new(pdf), None).unwrap();
    let d = s.get(id).unwrap();
    assert_eq!(compare::OfficeFormat::from_extension("XLSX"), Some(compare::OfficeFormat::Xlsx));
    let xlsx = d.export_office_named(compare::OfficeFormat::Xlsx, "Страница");
    // SINTEC_XLSX_OUT=file keeps the workbook, to open in Excel by hand.
    if let Ok(out) = std::env::var("SINTEC_XLSX_OUT") {
        std::fs::write(out, &xlsx).unwrap();
    }
    assert!(xlsx.starts_with(b"PK\x03\x04"));
    // The sheet, inflated: the zip's first sheet entry.
    let sheet = zip_entry(&xlsx, "xl/worksheets/sheet1.xml");
    for cell in ["<v>1250.5</v>", "<v>3400</v>", "<v>12.5</v>", "<v>100</v>", ">Paper<", ">Price list<"] {
        assert!(sheet.contains(cell), "{cell} missing from {sheet}");
    }
    // Item / Qty / Price sit in columns A, B, C of one row.
    assert!(sheet.contains("r=\"A") && sheet.contains("r=\"B") && sheet.contains("r=\"C"), "{sheet}");
    assert!(zip_entry(&xlsx, "xl/workbook.xml").contains("name=\"Страница 1\""));
}

/// Sintec.PDF: a table drawn with lines (stroked edges and thin filled bars, as office suites
/// print them) comes out as a bordered grid, a header over two columns merged, a cell with two
/// lines of text kept as one cell.
#[test]
fn exporting_a_ruled_table_keeps_its_grid() {
    // Columns at x 72 / 200 / 300 / 420; rows at y 720 / 700 / 680 / 660.
    let mut content = String::from("0.5 w 72 660 348 60 re S 200 659.75 0.5 60.25 re f 300 660 m 300 700 l S ");
    content.push_str("72 700 m 420 700 l S 72 680 m 420 680 l S ");
    content.push_str("BT /F1 10 Tf 76 706 Td (Item) Tj ET BT /F1 10 Tf 204 706 Td (Amount) Tj ET ");
    content.push_str("BT /F1 10 Tf 76 686 Td (Paper) Tj ET BT /F1 10 Tf 204 686 Td (10) Tj ET BT /F1 10 Tf 304 686 Td (1250.50) Tj ET ");
    content.push_str("BT /F1 10 Tf 76 671 Td (Toner,) Tj ET BT /F1 10 Tf 76 662 Td (black) Tj ET BT /F1 10 Tf 204 666 Td (2) Tj ET BT /F1 10 Tf 304 666 Td (3400) Tj ET ");
    content.push_str("BT /F1 10 Tf 72 600 Td (Thank you for your order.) Tj ET");
    let pdf = pdf_with(&content);
    let mut s = Session::new();
    let id = s.open("ruled.pdf", None, Arc::new(pdf), None).unwrap();
    let d = s.get(id).unwrap();
    let xlsx = d.export_office(compare::OfficeFormat::Xlsx);
    if let Ok(out) = std::env::var("SINTEC_XLSX_OUT") {
        std::fs::write(out, &xlsx).unwrap();
    }
    let sheet = zip_entry(&xlsx, "xl/worksheets/sheet1.xml");
    // The header row: "Amount" over columns B and C, merged; the rows below in their columns.
    assert!(sheet.contains("<mergeCell ref=\"B1:C1\"/>"), "{sheet}");
    for cell in [">Item<", ">Amount<", ">Paper<", "<v>10</v>", "<v>1250.5</v>", "<v>3400</v>", ">Toner,\nblack<"] {
        assert!(sheet.contains(cell), "{cell} missing from {sheet}");
    }
    assert!(sheet.contains("s=\"3\""), "bordered cells: {sheet}");
    assert!(sheet.contains(">Thank you for your order.<"), "{sheet}");
    let word = zip_entry(&d.export_office(compare::OfficeFormat::Docx), "word/document.xml");
    assert!(word.contains("<w:insideV w:val=\"single\""), "{word}");
    assert_eq!(word.matches(">Paper</w:t>").count(), 1, "{word}");
}

/// An entry of a zip archive, inflated, as text (test helper).
fn zip_entry(zip: &[u8], name: &str) -> String {
    use std::io::Read;
    let mut at = 0usize;
    let u16_at = |i: usize| u16::from_le_bytes([zip[i], zip[i + 1]]) as usize;
    let u32_at = |i: usize| u32::from_le_bytes([zip[i], zip[i + 1], zip[i + 2], zip[i + 3]]) as usize;
    while zip.get(at..at + 4) == Some(b"PK\x03\x04".as_slice()) {
        let (method, size, name_len, extra) = (u16_at(at + 8), u32_at(at + 18), u16_at(at + 26), u16_at(at + 28));
        let entry = String::from_utf8_lossy(&zip[at + 30..at + 30 + name_len]).into_owned();
        let start = at + 30 + name_len + extra;
        if entry == name {
            let data = &zip[start..start + size];
            let mut s = String::new();
            if method == 8 {
                flate2::read::DeflateDecoder::new(data).read_to_string(&mut s).unwrap();
            } else {
                s = String::from_utf8_lossy(data).into_owned();
            }
            return s;
        }
        at = start + size;
    }
    panic!("{name} not in the archive");
}
