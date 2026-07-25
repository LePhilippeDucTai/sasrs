use super::*;
use crate::ast::GlobalStmt;

// ── ODS ──────────────────────────────────────────────────────────────────

#[test]
fn parse_ods_listing() {
    let stmt = parse("ODS LISTING ;").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::Ods {
            destination: "listing".into(),
            action: OdsAction::Open,
            file: None,
            style: None,
        }
    );
}

#[test]
fn parse_ods_html_open() {
    let stmt = parse("ods html;").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::Ods {
            destination: "html".into(),
            action: OdsAction::Open,
            file: None,
            style: None,
        }
    );
}

#[test]
fn parse_ods_html_with_file() {
    let stmt = parse("ODS HTML FILE='out.html';").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::Ods {
            destination: "html".into(),
            action: OdsAction::Open,
            file: Some("out.html".into()),
            style: None,
        }
    );
}

#[test]
fn parse_ods_html_with_file_and_style() {
    let stmt = parse("ods html file='r.html' style=journal;").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::Ods {
            destination: "html".into(),
            action: OdsAction::Open,
            file: Some("r.html".into()),
            style: Some("journal".into()),
        }
    );
}

#[test]
fn parse_ods_rtf_pdf_excel_stubs() {
    for (src, dest) in [
        ("ods rtf;", "rtf"),
        ("ods pdf;", "pdf"),
        ("ods excel;", "excel"),
    ] {
        let stmt = parse(src).unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Ods {
                destination: dest.into(),
                action: OdsAction::Open,
                file: None,
                style: None,
            }
        );
    }
}

#[test]
fn parse_ods_close_destination_after_name() {
    let stmt = parse("ODS HTML CLOSE;").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::Ods {
            destination: "html".into(),
            action: OdsAction::Close,
            file: None,
            style: None,
        }
    );
}

#[test]
fn parse_ods_close_verb_with_name() {
    let stmt = parse("ods close html;").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::Ods {
            destination: "html".into(),
            action: OdsAction::Close,
            file: None,
            style: None,
        }
    );
}

#[test]
fn parse_ods_close_bare_defaults_listing() {
    let stmt = parse("ODS CLOSE;").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::Ods {
            destination: "listing".into(),
            action: OdsAction::Close,
            file: None,
            style: None,
        }
    );
}

#[test]
fn parse_ods_select_is_deferred_error() {
    let err = parse("ods html select foo;").unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.to_lowercase().contains("select") && msg.contains("M22.3"),
        "unexpected error: {msg}"
    );
}

#[test]
fn parse_ods_unknown_option_is_error() {
    let err = parse("ods html bogus=1;").unwrap_err();
    assert!(!err.to_string().is_empty());
}

#[test]
fn parse_ods_case_insensitive() {
    let stmt = parse("Ods Html Close ;").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::Ods {
            destination: "html".into(),
            action: OdsAction::Close,
            file: None,
            style: None,
        }
    );
}

// ── ODS OUTPUT (M22.3) ───────────────────────────────────────────────────

#[test]
fn parse_ods_output_single_mapping() {
    let stmt = parse("ods output Summary=out;").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::OdsOutput {
            mappings: vec![(
                "Summary".into(),
                DatasetRef {
                    libref: None,
                    name: "out".into(),
                }
            )],
            close: false,
        }
    );
}

#[test]
fn parse_ods_output_two_mappings() {
    let stmt = parse("ods output a=x b=y;").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::OdsOutput {
            mappings: vec![
                ("a".into(), DatasetRef { libref: None, name: "x".into() }),
                ("b".into(), DatasetRef { libref: None, name: "y".into() }),
            ],
            close: false,
        }
    );
}

#[test]
fn parse_ods_output_qualified_target() {
    let stmt = parse("ods output OneWayFreqs=work.freq_out;").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::OdsOutput {
            mappings: vec![(
                "OneWayFreqs".into(),
                DatasetRef {
                    libref: Some("work".into()),
                    name: "freq_out".into(),
                }
            )],
            close: false,
        }
    );
}

#[test]
fn parse_ods_output_close() {
    let stmt = parse("ods output close;").unwrap();
    assert_eq!(
        stmt,
        GlobalStmt::OdsOutput {
            mappings: vec![],
            close: true,
        }
    );
}

#[test]
fn parse_ods_output_requires_equals() {
    let err = parse("ods output Summary;").unwrap_err();
    assert!(!err.to_string().is_empty());
}

#[test]
fn parse_ods_graphics_on() {
    let s = graphics_stmt("ods graphics on;");
    assert_eq!(s.toggle, OdsGraphicsToggle::On);
    assert_eq!(s.width, None);
    assert_eq!(s.height, None);
    assert_eq!(s.imagefmt, None);
}

#[test]
fn parse_ods_graphics_off() {
    let s = graphics_stmt("ODS GRAPHICS OFF;");
    assert_eq!(s.toggle, OdsGraphicsToggle::Off);
}

#[test]
fn parse_ods_graphics_on_with_dims() {
    let s = graphics_stmt("ods graphics on / width=1000 height=700;");
    assert_eq!(s.toggle, OdsGraphicsToggle::On);
    assert_eq!(s.width, Some(1000));
    assert_eq!(s.height, Some(700));
}

#[test]
fn parse_ods_graphics_imagefmt_svg() {
    let s = graphics_stmt("ods graphics on / imagefmt=svg;");
    assert_eq!(s.imagefmt, Some(ImageFmt::Svg));
}

#[test]
fn parse_ods_graphics_imagefmt_png_parenthesized() {
    let s = graphics_stmt("ods graphics on / imagefmt=(png);");
    assert_eq!(s.imagefmt, Some(ImageFmt::Png));
}

#[test]
fn parse_ods_graphics_imagename_and_reset() {
    let s = graphics_stmt("ods graphics / imagename=\"myfig\" reset=index;");
    assert_eq!(s.toggle, OdsGraphicsToggle::None);
    assert_eq!(s.imagename.as_deref(), Some("myfig"));
}

#[test]
fn parse_ods_graphics_reset_bare() {
    let s = graphics_stmt("ods graphics on / reset width=640;");
    assert_eq!(s.toggle, OdsGraphicsToggle::On);
    assert_eq!(s.width, Some(640));
}
