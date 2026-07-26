use super::*;
use crate::{RunOptions, run};

#[test]
fn end_to_end_data_then_print() {
    let out = run_det(
        "title 'Essai';\n\
         data a; x = 1; y = 'ab'; run;\n\
         proc print data=a; run;\n",
    );
    assert_eq!(out.exit_code, 0, "log was:\n{}", out.log);
    // Écho numéroté du source.
    assert!(out.log.contains("1     title 'Essai';"), "{}", out.log);
    assert!(out.log.contains("data a; x = 1; y = 'ab'; run;"));
    // NOTEs de l'étape DATA.
    assert!(
        out.log
            .contains("The data set WORK.A has 1 observations and 2 variables.")
    );
    assert!(
        out.log
            .contains("DATA statement used (Total process time):")
    );
    assert!(out.log.contains("real time           0.00 seconds"));
    // PROC PRINT : timing + listing avec titre.
    assert!(
        out.log
            .contains("PROCEDURE PRINT used (Total process time):")
    );
    assert!(out.listing.contains("Essai"), "{}", out.listing);
    assert!(out.listing.contains("Obs"));
}

#[test]
fn filename_then_include_via_executor() {
    // M35.2 — `FILENAME inc '<tmp>'; %include inc;` enregistre le fileref
    // dans un segment, puis un segment ULTÉRIEUR l'inclut. Le fichier inclus
    // pose &n, utilisé par un DATA step. On vérifie l'effet observable.
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let inc = dir.path().join("inc.sas");
    std::fs::File::create(&inc)
        .unwrap()
        .write_all(b"%let n = 7;")
        .unwrap();
    let src = format!(
        "filename inc '{}';\n\
         data a; run;\n\
         %include inc;\n\
         data b; x = &n; run;\n\
         proc print data=b; run;\n",
        inc.display()
    );
    let out = run(
        &src,
        RunOptions {
            work_dir: None,
            base_dir: Some(dir.path().to_path_buf()),
            deterministic: true,
            vectorize: false,
        },
    );
    assert_eq!(out.exit_code, 0, "log was:\n{}", out.log);
    // &n a bien été résolu à 7 dans le DATA step b → x=7 dans le listing.
    assert!(out.listing.contains('7'), "listing was:\n{}", out.listing);
}

#[test]
fn execute_ods_opens_listing_and_html() {
    // ODS LISTING / ODS HTML / ODS CLOSE parsent et s'exécutent sans erreur,
    // et le listing texte reste fonctionnel après bascule.
    let out = run_det(
        "ods listing;\n\
         ods html file='out.html';\n\
         ods html close;\n\
         data a; x = 1; run;\n\
         proc print data=a; run;\n",
    );
    assert_eq!(out.exit_code, 0, "log was:\n{}", out.log);
    // Le listing texte par défaut fonctionne toujours après la bascule ODS.
    assert!(out.listing.contains("Obs"), "{}", out.listing);
}

#[test]
fn execute_ods_rtf_without_file_emits_note() {
    let out = run_det("ods rtf;\n");
    assert_eq!(out.exit_code, 0, "log was:\n{}", out.log);
    assert!(out.log.contains("ODS RTF sans FILE="), "{}", out.log);
}

#[test]
fn execute_global_ods_options_no_warning() {
    // NOCENTER/NODATE/NONUMBER sont reconnues comme options ODS et ne
    // déclenchent pas de WARNING "not yet supported".
    let out = run_det("options nocenter nodate nonumber;\n");
    assert_eq!(out.exit_code, 0, "log was:\n{}", out.log);
    assert!(
        !out.log.contains("is not yet supported"),
        "unexpected warning in log:\n{}",
        out.log
    );
}

#[test]
fn multi_title_end_to_end_renders_centered_in_order() {
    let mut s = run_globals(&["title 'Top';", "title2 'Mid';", "title3 'Bottom';"]);
    s.listing.page_header();
    let out = s.listing.take_string();
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0].trim(), "Top");
    assert_eq!(lines[1].trim(), "Mid");
    assert_eq!(lines[2].trim(), "Bottom");
    assert_eq!(lines[3], "", "single trailing blank after all titles");
}

#[test]
fn title_then_title1_clears_above_end_to_end() {
    // title/title2/title3 puis title 'X' efface 2-3.
    let mut s = run_globals(&["title 'A';", "title2 'B';", "title3 'C';", "title 'X';"]);
    s.listing.page_header();
    let out = s.listing.take_string();
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0].trim(), "X");
    assert_eq!(lines[1], "");
}

#[test]
fn footnote_end_to_end_renders_after_content() {
    let mut s = run_globals(&["footnote 'Bye';"]);
    s.listing.page_header();
    s.listing.write_line("body");
    let out = s.listing.take_string();
    let lines: Vec<&str> = out.lines().collect();
    let pos = lines.iter().position(|x| x.trim() == "Bye").unwrap();
    let body = lines.iter().position(|x| x.trim() == "body").unwrap();
    assert!(pos > body, "footnote should follow body content");
}

#[test]
fn ods_graphics_on_sets_enabled() {
    let s = run_graphics_stmt("ods graphics on;");
    assert!(s.ods_graphics.enabled);
}

#[test]
fn ods_graphics_off_clears_enabled() {
    // ON puis OFF sur la même session : l'état doit finir à false.
    use crate::parser::StatementStream;
    use crate::parser::global::parse_global;
    use crate::source::SourceFile;
    let mut session = crate::session::Session::new(None, std::env::temp_dir(), true).unwrap();
    for src in ["ods graphics on;", "ods graphics off;"] {
        let sf = SourceFile::new(src);
        let mut ts = StatementStream::new(&sf).unwrap();
        let stmt = parse_global(&mut ts).unwrap();
        super::super::exec_global(&stmt, &mut session);
    }
    assert!(!session.ods_graphics.enabled);
}

#[test]
fn ods_graphics_width_height_update_fields() {
    let s = run_graphics_stmt("ods graphics on / width=1000 height=700;");
    assert!(s.ods_graphics.enabled);
    assert_eq!(s.ods_graphics.width, 1000);
    assert_eq!(s.ods_graphics.height, 700);
}

#[test]
fn ods_graphics_imagefmt_svg_updates_field() {
    let s = run_graphics_stmt("ods graphics on / imagefmt=svg;");
    assert_eq!(
        s.ods_graphics.image_format,
        crate::ods_graphics::ImageFmt::Svg
    );
}

#[test]
fn ods_graphics_imagefmt_png_updates_field() {
    let s = run_graphics_stmt("ods graphics on / imagefmt=png;");
    assert_eq!(
        s.ods_graphics.image_format,
        crate::ods_graphics::ImageFmt::Png
    );
}

#[test]
fn ods_graphics_default_state() {
    let session = crate::session::Session::new(None, std::env::temp_dir(), true).unwrap();
    assert!(!session.ods_graphics.enabled);
    assert_eq!(session.ods_graphics.width, 800);
    assert_eq!(session.ods_graphics.height, 600);
    assert_eq!(
        session.ods_graphics.image_format,
        crate::ods_graphics::ImageFmt::Png
    );
}

#[test]
fn ods_graphics_note_dims_only_when_specified() {
    // 4e ON est nu (sans dims) même si une session a déjà width=1000 :
    // la NOTE ne montre les dims que pour le statement qui les porte.
    let out = run_det(
        "ods graphics on;\n\
         ods graphics on / width=1000 height=700;\n\
         ods graphics off;\n\
         ods graphics on;\n\
         ods graphics off;\n",
    );
    assert_eq!(out.exit_code, 0, "log was:\n{}", out.log);
    assert!(out.log.contains("NOTE: ODS GRAPHICS ON."), "{}", out.log);
    assert!(
        out.log
            .contains("NOTE: ODS GRAPHICS ON (width=1000, height=700)."),
        "{}",
        out.log
    );
    assert!(out.log.contains("NOTE: ODS GRAPHICS OFF."), "{}", out.log);
}

#[test]
fn error_recovery_continues_session() {
    let out = run_det(
        "frobnicate;\n\
         data a; x = 1; run;\n",
    );
    assert_eq!(out.exit_code, 2);
    assert!(
        out.log
            .contains("ERROR: Statement 'FROBNICATE' is not valid")
    );
    // L'étape suivante s'exécute malgré l'erreur.
    assert!(
        out.log
            .contains("The data set WORK.A has 1 observations and 1 variables.")
    );
}

#[test]
fn unknown_proc_errors_and_continues() {
    let out = run_det(
        "proc nosuchproc data=a; run;\n\
         data b; x = 1; run;\n",
    );
    assert_eq!(out.exit_code, 2);
    assert!(out.log.contains("ERROR: Procedure NOSUCHPROC not found."));
    assert!(
        out.log
            .contains("The data set WORK.B has 1 observations and 1 variables.")
    );
}

#[test]
fn missing_input_dataset_stops_step_with_notes() {
    let out = run_det("data a; set nosuch; run;");
    assert_eq!(out.exit_code, 2);
    assert!(
        out.log
            .contains("ERROR: File WORK.NOSUCH.DATA does not exist.")
    );
    assert!(
        out.log
            .contains("The SAS System stopped processing this step because of errors.")
    );
    // Timing imprimé malgré l'erreur.
    assert!(
        out.log
            .contains("DATA statement used (Total process time):")
    );
}
