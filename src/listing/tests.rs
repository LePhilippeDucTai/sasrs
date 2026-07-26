use super::*;

#[test]
fn table_layout() {
    let mut l = ListingWriter::new(40);
    l.page_header();
    l.write_table(
        &["Obs".into(), "x".into()],
        &[Align::Right, Align::Right],
        &[
            vec!["1".into(), "10".into()],
            vec!["2".into(), "200".into()],
        ],
    );
    let s = l.into_string();
    assert!(s.contains("The SAS System"));
    assert!(s.contains("Obs      x"));
    assert!(s.contains("  1     10"));
    assert!(s.contains("  2    200"));
}

/// Byte-identity guard: a single title plus default rendering is unchanged.
#[test]
fn single_title_byte_identical() {
    let mut l = ListingWriter::new(40);
    l.titles = vec!["My Report".into()];
    l.page_header();
    // pad = (40 - 9) / 2 = 15 spaces, then text, then blank line.
    assert_eq!(l.into_string(), format!("{}My Report\n\n", " ".repeat(15)));
}

/// Three titles render centered, in level order, with one trailing blank.
#[test]
fn three_titles_centered_in_order() {
    let mut l = ListingWriter::new(20);
    l.titles = vec!["A".into(), "BB".into(), "CCC".into()];
    l.page_header();
    let s = l.into_string();
    let lines: Vec<&str> = s.lines().collect();
    // Title order preserved.
    assert_eq!(lines[0].trim(), "A");
    assert_eq!(lines[1].trim(), "BB");
    assert_eq!(lines[2].trim(), "CCC");
    // Centering: pad = (20 - len) / 2.
    assert_eq!(lines[0], format!("{}A", " ".repeat((20 - 1) / 2)));
    assert_eq!(lines[2], format!("{}CCC", " ".repeat((20 - 3) / 2)));
    // Exactly one trailing blank line after all titles (line index 3).
    assert_eq!(lines[3], "");
    assert_eq!(lines.len(), 4);
}

/// Footnotes render centered at the bottom on drain.
#[test]
fn footnotes_centered_on_drain() {
    let mut l = ListingWriter::new(20);
    l.footnotes = vec!["Note1".into(), "Note2".into()];
    l.page_header();
    l.write_line("body");
    let s = l.into_string();
    let lines: Vec<&str> = s.lines().collect();
    // Footnotes appear after the body, centered, preceded by a blank.
    let f1 = lines.iter().position(|x| x.trim() == "Note1").unwrap();
    assert_eq!(lines[f1 - 1], "", "footnotes preceded by a blank separator");
    assert_eq!(lines[f1], format!("{}Note1", " ".repeat((20 - 5) / 2)));
    assert_eq!(lines[f1 + 1].trim(), "Note2");
}

/// No active footnote → no footnote output (byte-identity preserved).
#[test]
fn no_footnote_no_extra_output() {
    let mut l = ListingWriter::new(40);
    l.page_header();
    l.write_line("body");
    let s = l.into_string();
    assert_eq!(
        s,
        format!("{}The SAS System\n\nbody\n", " ".repeat((40 - 14) / 2))
    );
}
