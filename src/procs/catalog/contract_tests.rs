// ── J02-P8 : contrat PROC CATALOG (replis silencieux supprimés) ──────────
//
// Chaque test reproduit l'ancien silence (commentaire « Base : … ») et fige
// le diagnostic ou la correction (texte, code de sortie). Oracle de sévérité :
// CONTRIBUTING §5 ; syntaxe de référence : Base SAS 9.4 Procedures Guide,
// CATALOG Procedure, PROC CATALOG statement
// https://support.sas.com/documentation/cdl/en/proc/65145/HTML/default/p1il9pe2plu3bpn0zza8igp6xue4.htm
// `PROC CATALOG CATALOG=<libref.>catalog <ENTRYTYPE=etype> <FORCE> <KILL>;`
// (CATALOG= « Alias: CAT=, C= » ; ENTRYTYPE= « Alias: ET= ») ; CONTENTS
// statement `CONTENTS <OUT=SAS-data-set> <FILE=fileref>;`.

/// Exécute un programme complet (mode déterministe) : log, listing, code.
fn run_sas(src: &str) -> crate::RunOutcome {
    crate::run(
        src,
        crate::RunOptions {
            deterministic: true,
            ..Default::default()
        },
    )
}

/// A user format F in WORK.FORMATS.
const FMT: &str = "proc format; value f 1='One'; run;\n";

/// Contract ERROR suffix shared by the rejected requests.
const CANNOT: &str =
    "is not supported in PROC CATALOG; it can affect results and cannot be ignored";

fn assert_has(out: &crate::RunOutcome, needle: &str) {
    assert!(out.log.contains(needle), "missing «{needle}»\n{}", out.log);
}

/// Asserts a rejected step: exit code 2, `needle` in the log, no listing.
fn assert_rejected(out: &crate::RunOutcome, needle: &str, ctx: &str) {
    assert_eq!(out.exit_code, 2, "{ctx}: {}", out.log);
    assert!(
        out.log.contains(needle),
        "{ctx}: missing «{needle}»\n{}",
        out.log
    );
    assert!(
        !out.log.contains("Processing catalog"),
        "{ctx}: the step must be rejected before it runs\n{}",
        out.log
    );
    assert!(
        out.listing.trim().is_empty(),
        "{ctx}: no listing expected\n{}",
        out.listing
    );
}

/// Base : `cat=work.formats` était sauté — « NOTE: Processing catalog:
/// (none). » et un listing « Catalog: » sans nom. Correction directe — SAS
/// 9.4 PROC CATALOG statement, CATALOG= : « Alias: CAT=, C= ». Sans
/// CATALOG= (argument requis), ERROR au lieu de « (none) ».
#[test]
fn ra_j02_p8_catalog_cat_alias_is_honored() {
    for opt in ["cat", "c", "catalog"] {
        let out = run_sas(&format!(
            "{FMT}proc catalog {opt}=work.formats; contents; quit;"
        ));
        assert_eq!(out.exit_code, 0, "{opt}: {}", out.log);
        assert_has(&out, "NOTE: Processing catalog: WORK.FORMATS.");
        assert!(!out.log.contains("(none)"), "{opt}: {}", out.log);
        assert!(
            out.listing.contains("Catalog: WORK.FORMATS"),
            "{opt}: {}",
            out.listing
        );
    }

    let out = run_sas(&format!("{FMT}proc catalog; contents; quit;"));
    assert_rejected(
        &out,
        "ERROR: The CATALOG= option is required on the PROC CATALOG statement.",
        "no CATALOG=",
    );
}

/// Base : `et=format` / `entrytype=format` était sauté et la procédure
/// listait toutes les entrées. ENTRYTYPE= « restricts processing of the
/// current PROC CATALOG step to one entry type » : ERROR jusqu'à J11-P6.
#[test]
fn ra_j02_p8_catalog_entrytype_is_error() {
    for opt in ["et=format", "entrytype=formatc", "ET=infmt"] {
        let out = run_sas(&format!(
            "{FMT}proc catalog catalog=work.formats {opt}; contents; quit;"
        ));
        assert_rejected(
            &out,
            &format!("ERROR: The ENTRYTYPE= option {CANNOT} (planned: roadmap-avancee J11-P6)."),
            opt,
        );
    }
}

/// Base : `kill` était sauté (« NOTE: Processing catalog: WORK.FORMATS. »,
/// code 0, entrées conservées). KILL « deletes all entries in a SAS
/// catalog » : ERROR jusqu'à J03-P5 ; le format reste défini. Toute autre
/// option inconnue était sautée : ERROR « Unexpected option ». FORCE est
/// accepté (aucun autre environnement ne tient le catalogue ouvert).
#[test]
fn ra_j02_p8_catalog_kill_and_unknown_options_are_errors() {
    let out = run_sas(&format!(
        "{FMT}proc catalog catalog=work.formats kill; quit;
         data _null_; x = put(1, f.); put x=; run;"
    ));
    assert_eq!(out.exit_code, 2, "{}", out.log);
    assert_has(
        &out,
        &format!("ERROR: The KILL option {CANNOT} (planned: roadmap-avancee J03-P5)."),
    );
    assert!(!out.log.contains("Processing catalog"), "{}", out.log);
    assert_has(&out, "x=One");

    let out = run_sas(&format!(
        "{FMT}proc catalog catalog=work.formats nosuch; contents; quit;"
    ));
    assert_rejected(
        &out,
        "ERROR: Unexpected option 'NOSUCH' on PROC CATALOG statement.",
        "unknown option",
    );

    let out = run_sas(&format!(
        "{FMT}proc catalog catalog=work.formats force; contents; quit;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(
        out.listing.contains("Catalog: WORK.FORMATS"),
        "{}",
        out.listing
    );
}

/// Base : CHANGE, EXCHANGE, EXCLUDE, MODIFY, SAVE et SELECT (instructions
/// SAS 9.4 de PROC CATALOG) étaient signalées « 180-322 … not valid » ;
/// `contents out=x;` lisait `out=x` comme une instruction (« 180-322:
/// Statement 'OUT' is not valid »). Message du catalogue du contrat ;
/// DELETE et COPY nomment désormais J03-P5, OUT= de CONTENTS J11-P6.
#[test]
fn ra_j02_p8_catalog_valid_statements_are_not_supported() {
    for (stmt, what, unit) in [
        ("change f=g", "The CHANGE statement", None),
        ("exchange f=g", "The EXCHANGE statement", None),
        (
            "copy out=work.other; exclude f",
            "The COPY statement",
            Some("J03-P5"),
        ),
        ("exclude f", "The EXCLUDE statement", None),
        ("modify f (description='x')", "The MODIFY statement", None),
        ("save f", "The SAVE statement", None),
        ("select f", "The SELECT statement", None),
        ("delete f", "The DELETE statement", Some("J03-P5")),
        (
            "contents out=work.entries",
            "The OUT= option of the CONTENTS statement",
            Some("J11-P6"),
        ),
        (
            "contents file=myref",
            "The FILE= option of the CONTENTS statement",
            None,
        ),
    ] {
        let out = run_sas(&format!(
            "{FMT}proc catalog catalog=work.formats; {stmt}; quit;"
        ));
        let planned = unit
            .map(|u| format!(" (planned: roadmap-avancee {u})"))
            .unwrap_or_default();
        assert_rejected(&out, &format!("ERROR: {what} {CANNOT}{planned}."), stmt);
        assert!(!out.log.contains("180-322"), "{stmt}: {}", out.log);
    }

    let out = run_sas(&format!(
        "{FMT}proc catalog catalog=work.formats; contents bogus; quit;"
    ));
    assert_rejected(
        &out,
        "ERROR: Unexpected option 'BOGUS' on the CONTENTS statement of PROC CATALOG.",
        "CONTENTS option",
    );

    // An invented statement keeps the 180-322 diagnostic.
    let out = run_sas(&format!(
        "{FMT}proc catalog catalog=work.formats; invented x; quit;"
    ));
    assert_rejected(
        &out,
        "ERROR: 180-322: Statement 'INVENTED' is not valid or it is used out of proper order \
         in PROC CATALOG.",
        "invented",
    );
}
