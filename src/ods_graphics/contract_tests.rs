// ── J02-P6 : contrat ODS GRAPHICS / ODS <destination> ─────────────────────
//
// Base : `ODS GRAPHICS / RESET` et `RESET=` étaient parsés puis ignorés ;
// STYLE= et OPTIONS= d'un statement `ODS <destination>` aussi. Oracle :
// SAS 9.4 ODS User's Guide, ODS GRAPHICS statement
// (https://documentation.sas.com/doc/en/pgmsascdc/9.4_3.5/odsug/p0kroq43yu0lspn16hk1u4c65lti.htm)
// — « RESET … resets all options to their defaults », RESET=option remet une
// option à son défaut, INDEX « resets the index counter that is appended to
// static image files » ; sévérité : CONTRIBUTING §5.

use super::{DEFAULT_HEIGHT, DEFAULT_WIDTH, ImageFmt};
use crate::session::Session;
use crate::source::SourceFile;

/// Runs `src` in `session` and returns the log produced by this run.
fn run_in(session: &mut Session, src: &str) -> String {
    let before = session.log.current_text().len();
    crate::executor::run_program(&SourceFile::new(src), session);
    session.log.current_text()[before..].to_string()
}

/// Direct fix: RESET=WIDTH, RESET=HEIGHT, RESET=OUTPUTFMT (alias IMAGEFMT)
/// and RESET / RESET=ALL put WIDTH=, HEIGHT= and OUTPUTFMT= back to their
/// defaults, in source order within the statement; options that sasrs does
/// not model (ANTIALIAS…) are always at their default, so resetting them is a
/// no-op. The IMAGENAME= prefix and the image index cannot be reset through
/// the statement's AST (`src/ast/global.rs`, outside this unit): WARNING. An
/// unknown name is an ERROR and the statement is not applied.
#[test]
fn ra_j02_p6_ods_graphics_reset() {
    let tmp = tempfile::tempdir().unwrap();
    let mut session = Session::new(None, tmp.path().to_path_buf(), true).unwrap();
    let log = run_in(
        &mut session,
        "ods graphics on / width=1000 height=700 imagefmt=svg imagename='fig';
         ods graphics / reset=width;",
    );
    let g = &session.ods_graphics;
    assert_eq!((g.width, g.height), (DEFAULT_WIDTH, 700), "{log}");
    assert_eq!(g.image_format, ImageFmt::Svg);
    assert_eq!(g.file_stem.as_deref(), Some("fig"));
    assert!(!log.contains("WARNING"), "{log}");

    let log = run_in(
        &mut session,
        "ods graphics / reset=height reset=outputfmt; ods graphics / reset=antialias;",
    );
    let g = &session.ods_graphics;
    assert_eq!(
        (g.width, g.height),
        (DEFAULT_WIDTH, DEFAULT_HEIGHT),
        "{log}"
    );
    assert_eq!(g.image_format, ImageFmt::Png);
    assert!(!log.contains("WARNING"), "{log}");

    // RESET (= RESET=ALL): the prefix and the index are not reset (WARNING).
    let log = run_in(
        &mut session,
        "ods graphics / width=1000 height=700 imagefmt=svg; ods graphics on / reset;",
    );
    let g = &session.ods_graphics;
    assert_eq!(
        (g.width, g.height),
        (DEFAULT_WIDTH, DEFAULT_HEIGHT),
        "{log}"
    );
    assert_eq!(g.image_format, ImageFmt::Png);
    assert!(g.enabled);
    assert_eq!(g.file_stem.as_deref(), Some("fig"));
    assert!(
        log.contains(
            "WARNING: The RESET option of the ODS GRAPHICS statement does not reset the \
             IMAGENAME= prefix and the image index in this build; WIDTH=, HEIGHT= and \
             OUTPUTFMT= are reset to their defaults."
        ),
        "{log}"
    );

    // Source order: an option written after RESET wins, RESET after it resets.
    let log = run_in(
        &mut session,
        "ods graphics / reset=all width=640; ods graphics / height=480 reset=height;",
    );
    let g = &session.ods_graphics;
    assert_eq!((g.width, g.height), (640, DEFAULT_HEIGHT), "{log}");

    let log = run_in(
        &mut session,
        "ods graphics / reset=index; ods graphics / reset=imagename;",
    );
    assert!(
        log.contains(
            "WARNING: RESET=INDEX is not supported in this build; the image index is not reset."
        ),
        "{log}"
    );
    assert!(
        log.contains(
            "WARNING: RESET=IMAGENAME is not supported in this build; the IMAGENAME= prefix is \
             kept."
        ),
        "{log}"
    );

    let errors = session.log.errors;
    let log = run_in(&mut session, "ods graphics / reset=bogus width=1000;");
    assert!(
        log.contains("ERROR: RESET=BOGUS is not a valid ODS GRAPHICS option."),
        "{log}"
    );
    assert_eq!(session.log.errors, errors + 1);
    assert_eq!(session.ods_graphics.width, 640, "statement not applied");
}

/// Base : STYLE= (stocké dans l'AST, appliqué par aucune destination) et
/// OPTIONS= étaient ignorés sans diagnostic. Désormais WARNING d'affichage
/// (code 1) ; la destination est ouverte normalement.
#[test]
fn ra_j02_p6_ods_destination_style_options_warn() {
    let tmp = tempfile::tempdir().unwrap();
    let out = crate::run(
        "ods html file='out.html' style=journal options=x;
         data a; x=1; run;
         proc print data=a; run;
         ods html close;",
        crate::RunOptions {
            deterministic: true,
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        },
    );
    assert_eq!(out.exit_code, 1, "{}", out.log);
    for option in ["STYLE", "OPTIONS"] {
        assert!(
            out.log.contains(&format!(
                "WARNING: The {option}= option of the ODS HTML statement is ignored in this \
                 build; display customization is not supported."
            )),
            "{}",
            out.log
        );
    }
    assert_eq!(out.log.matches("WARNING:").count(), 2, "{}", out.log);
    assert!(tmp.path().join("out.html").exists(), "{}", out.log);
}
