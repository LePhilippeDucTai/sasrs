// ── J02-P8 : contrat PROC FORMAT (replis silencieux supprimés) ───────────
//
// Chaque test reproduit l'ancien silence (commentaire « Base : … ») et fige
// le diagnostic (texte, code de sortie). Oracle de sévérité : CONTRIBUTING
// §5 ; syntaxe de référence : Base SAS 9.4 Procedures Guide, FORMAT
// Procedure, PROC FORMAT statement
// https://support.sas.com/documentation/cdl/en/proc/65145/HTML/default/n1c16dxnndwfzyn14o1kb8a4312m.htm
// (options CNTLIN=, CNTLOUT=, FMTLIB, LIBRARY=, LOCALE, MAXLABLEN=,
// MAXSELEN=, NOREPLACE, PAGE ; statements EXCLUDE, INVALUE, PICTURE, SELECT,
// VALUE).

use super::run_det;

/// Contract ERROR suffix shared by the rejected requests.
const CANNOT: &str = "it can affect results and cannot be ignored";

/// Runs `step` after a first PROC FORMAT that defines F (1 → 'One'), then
/// prints `put(1, f.)` to the log : the definition in force after the step.
fn after_definition(step: &str) -> crate::RunOutcome {
    run_det(&format!(
        "proc format; value f 1='One'; run;
         {step}
         data _null_; x = put(1, f.); put x=; run;"
    ))
}

/// Base : `proc format foo=bar; value f 1='Two'; run;` — option inconnue
/// sautée jeton par jeton, F redéfini, code 0. Désormais ERROR « Unexpected
/// option » : l'étape est rejetée, F garde sa définition précédente.
#[test]
fn ra_j02_p8_format_unknown_header_option_is_error() {
    let out = after_definition("proc format foo=bar; value f 1='Two'; run;");
    assert_eq!(out.exit_code, 2, "{}", out.log);
    assert!(
        out.log
            .contains("ERROR: Unexpected option 'FOO' on PROC FORMAT statement."),
        "{}",
        out.log
    );
    assert_eq!(
        out.log.matches("NOTE: Format F has been output.").count(),
        1,
        "only the first step defines F\n{}",
        out.log
    );
    assert!(out.log.contains("x=One"), "{}", out.log);
}

/// Base : NOREPLACE, LOCALE, PAGE, MAXLABLEN= et MAXSELEN= (options SAS 9.4
/// valides) étaient sautés sans diagnostic. NOREPLACE (« prevents a new
/// informat or format from replacing an existing one ») et LOCALE (catalogue
/// nommé d'après la locale) changent les définitions stockées → ERROR ;
/// PAGE (mise en page de FMTLIB) → WARNING d'affichage ; MAXLABLEN= et
/// MAXSELEN= règlent le nombre de caractères écrits « in the CNTLOUT= data
/// set or in the output of the FMTLIB option » → ERROR avec CNTLOUT=,
/// WARNING d'affichage sinon.
#[test]
fn ra_j02_p8_format_valid_header_options_are_diagnosed() {
    for opt in ["noreplace", "locale"] {
        let out = after_definition(&format!("proc format {opt}; value f 1='Two'; run;"));
        assert_eq!(out.exit_code, 2, "{opt}: {}", out.log);
        let msg = format!(
            "ERROR: The {} option is not supported in PROC FORMAT; {CANNOT}.",
            opt.to_uppercase()
        );
        assert!(
            out.log.contains(&msg),
            "{opt}: missing «{msg}»\n{}",
            out.log
        );
        assert!(out.log.contains("x=One"), "{opt}: {}", out.log);
    }

    for (opt, name) in [
        ("page", "PAGE"),
        ("maxlablen=5", "MAXLABLEN="),
        ("maxselen=4 fmtlib", "MAXSELEN="),
    ] {
        let out = after_definition(&format!("proc format {opt}; value f 1='Two'; run;"));
        assert_eq!(out.exit_code, 1, "{opt}: {}", out.log);
        let msg = format!(
            "WARNING: The {name} option is ignored in PROC FORMAT; display customization is \
             not supported."
        );
        assert!(
            out.log.contains(&msg),
            "{opt}: missing «{msg}»\n{}",
            out.log
        );
        assert!(
            out.log.contains("x=Two"),
            "{opt}: the step runs\n{}",
            out.log
        );
    }

    for opt in ["maxlablen=5 cntlout=ctl", "cntlout=ctl maxselen=4"] {
        let out = after_definition(&format!("proc format {opt}; value f 1='Two'; run;"));
        assert_eq!(out.exit_code, 2, "{opt}: {}", out.log);
        assert!(
            out.log
                .contains("option is not supported in PROC FORMAT with CNTLOUT=; it can affect"),
            "{opt}: {}",
            out.log
        );
        assert!(out.log.contains("x=One"), "{opt}: {}", out.log);
    }
}

/// Base : SELECT et EXCLUDE (instructions SAS 9.4 de PROC FORMAT, qui
/// choisissent les entrées écrites par CNTLOUT= et listées par FMTLIB)
/// étaient signalées « 180-322: Statement 'SELECT' is not valid … ».
/// Message du catalogue du contrat ; une instruction inventée garde 180-322.
#[test]
fn ra_j02_p8_format_select_exclude_are_not_supported() {
    for stmt in ["select", "exclude"] {
        let out = after_definition(&format!("proc format cntlout=ctl; {stmt} f; run;"));
        assert_eq!(out.exit_code, 2, "{stmt}: {}", out.log);
        let msg = format!(
            "ERROR: The {} statement is not supported in PROC FORMAT; {CANNOT}.",
            stmt.to_uppercase()
        );
        assert!(
            out.log.contains(&msg),
            "{stmt}: missing «{msg}»\n{}",
            out.log
        );
        assert!(!out.log.contains("180-322"), "{stmt}: {}", out.log);
    }

    let out = run_det("proc format; invented f; run;");
    assert_eq!(out.exit_code, 2, "{}", out.log);
    assert!(
        out.log.contains(
            "ERROR: 180-322: Statement 'INVENTED' is not valid or it is used out of proper \
             order in PROC FORMAT."
        ),
        "{}",
        out.log
    );
}
