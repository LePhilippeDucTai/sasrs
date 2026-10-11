//! ── J02-P9 : contrat des options globales (replis silencieux supprimés) ──
//!
//! Chaque test reproduit l'ancien silence (commentaire « Base : … ») et fige
//! la correction ou le diagnostic (texte, code de sortie). Oracle de
//! sévérité : CONTRIBUTING §5 ; sémantique de référence : doc SAS 9.4
//! publiée —
//! OBS= / FIRSTOBS= System Options
//!   https://documentation.sas.com/doc/en/pgmsascdc/9.4_3.5/lesysoptsref/p0c2av6mvf2zsrn1b48go7vk6bro.htm
//!   (valables pour toute lecture de table, pas seulement l'étape DATA)
//! YEARCUTOFF= System Option
//!   https://documentation.sas.com/doc/en/pgmsascdc/9.4_3.5/lesysoptsref/n0yt2tjgsd5dpzn16wk1m3thcd3c.htm
//! What's New in SAS 9.4 System Options (défaut YEARCUTOFF 1926)
//!   https://documentation.sas.com/doc/en/pgmsascdc/9.4_3.5/lesysoptsref/lesysoptsrefwhatsnew94.htm
//! PROC OPTIONS statement (OPTION=)
//!   https://documentation.sas.com/doc/en/pgmsascdc/9.4_3.5/proc/p1x0pd86qbcpgbn1rxqbe5tftf2x.htm
//! OPTIONS statement (alias `option`)
//!   https://documentation.sas.com/doc/en/pgmsascdc/9.4_3.5/lestmtsref/p18tpv111hszuwn1n0gqxg0lvsbd.htm

use super::*;

/// (1) OBS=/FIRSTOBS= appliqués à toute table lue en entrée de PROC.
/// Base : seuls les SET/MERGE de l'étape DATA honoraient la fenêtre ; les
/// PROC lisaient la table entière en silence (doc SAS 9.4, OBS= System
/// Option : « specifies the last observation of the input data set » pour
/// TOUTE lecture ; ici PROC PRINT / PROC MEANS).
#[test]
fn ra_j02_p9_obs_firstobs_window_proc_inputs() {
    let out = run_det(
        "data a; do i = 1 to 5; output; end; run;\n\
         options firstobs=2 obs=4;\n\
         proc print data=a; run;\n\
         proc means data=a; var i; run;\n",
    );
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    // PROC PRINT rend 3 observations (les valeurs 2, 3, 4 de la fenêtre),
    // pas les 5 de la table : les colonnes i ne contiennent que 2, 3, 4.
    let i_values: Vec<&str> = out
        .listing
        .lines()
        .filter(|l| {
            l.trim_start()
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit())
        })
        .filter_map(|l| l.split_whitespace().last())
        .collect();
    // Les 3 lignes de PROC PRINT (Obs 1..3) portent 2, 3, 4 — jamais 1 ni 5.
    assert!(
        i_values.contains(&"2") && i_values.contains(&"3") && i_values.contains(&"4"),
        "expected window values 2,3,4 in listing:\n{}",
        out.listing
    );
    assert!(
        !i_values.contains(&"1") && !i_values.contains(&"5"),
        "window not applied to PROC inputs:\n{}",
        out.listing
    );
    // PROC MEANS analyse 3 observations : moyenne de 2,3,4 = 3, N=3.
    assert!(
        out.listing.contains("The MEANS Procedure"),
        "listing:\n{}",
        out.listing
    );
    let means_line = out
        .listing
        .lines()
        .find(|l| l.trim_start().starts_with("i "))
        .unwrap_or_else(|| panic!("MEANS row not found:\n{}", out.listing));
    let nums: Vec<&str> = means_line.split_whitespace().collect();
    assert_eq!(nums[1], "3", "N must be 3:\n{}", out.listing);
    assert_eq!(nums[2], "3", "mean of 2,3,4 must be 3:\n{}", out.listing);
}

/// (1) La fenêtre OBS= ne touche PAS PROC CONTENTS : il rapporte le compte
/// d'observations STOCKÉ de la table (métadonnées, doc SAS 9.4, CONTENTS
/// statement : « Observations » est un attribut du member).
#[test]
fn ra_j02_p9_obs_window_spares_proc_contents_metadata() {
    let out = run_det(
        "data a; do i = 1 to 5; output; end; run;\n\
         options obs=2;\n\
         proc contents data=a; run;\n\
         proc print data=a; run;\n",
    );
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    assert!(
        out.log.contains("has 5 observations") || out.listing.contains("Observations:  5"),
        "CONTENTS must report the STORED count:\nlog:\n{}\nlisting:\n{}",
        out.log,
        out.listing
    );
}

/// (2) YEARCUTOFF= défaut 1926 : informats à année sur 2 chiffres.
/// Base : défaut 1900 côté session/fonctions, et informats codés en dur
/// 2000–2099 (src/formats/builtin/informat.rs). Correction directe — doc
/// SAS 9.4 « What's New in SAS 9.4 System Options » : « The default value
/// of the YEARCUTOFF value has been changed from 1920 to 1926 » ; page
/// YEARCUTOFF= : « The YEARCUTOFF= value is the default that is used by
/// various date and datetime informats and functions ».
/// Avec le défaut 1926 : yy=26 → 1926 (la fenêtre COMMENCE à 1926),
/// yy=15 → 2015 (première moitié de la seconde centaine). L'ancien
/// informat codait en dur 2000+yy : '01JAN15' lisait 2015 aussi, mais
/// '01JAN26' lisait 2026 — la frontière de la fenêtre est maintenant
/// celle de SAS. input('01JAN26', date7.) → 1926-01-01 = SAS -12418.
#[test]
fn ra_j02_p9_yearcutoff_default_1926_informats() {
    let out = run_det(
        "data a; d = input('01JAN26', date7.); run;\n\
         proc print data=a; run;\n",
    );
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    // 1926-01-01 = SAS -12418 (l'ancien code en dur rendait 2026 → 24107).
    assert!(
        out.listing.contains("-12418"),
        "expected SAS date -12418 (1926-01-01):\n{}",
        out.listing
    );
    // Une année à 4 chiffres reste littérale : 01/26/2026 = 26 janvier 2026
    // → SAS 24132 (la fenêtre YEARCUTOFF= ne s'applique pas).
    let out = run_det(
        "data a; m = input('01/26/2026', mmddyy10.); run;\n\
         proc print data=a; run;\n",
    );
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    assert!(
        out.listing.contains("24132"),
        "expected SAS date 24132 (2026-01-26, 4-digit year):\n{}",
        out.listing
    );
    // La fenêtre suit YEARCUTOFF= posé à la session : 1950 → yy=30 donne
    // 2030 (1930 < 1950 → +100) = SAS 25568 (l'ancien code en dur
    // rendait 2000+30 = 2030 aussi : ce cas fige le SUIVI de l'option).
    let out = run_det(
        "options yearcutoff=1950;\n\
         data a; d = input('01JAN30', date7.); run;\n\
         proc print data=a; run;\n",
    );
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    assert!(
        out.listing.contains("25568"),
        "expected SAS date 25568 (2030-01-01) with YEARCUTOFF=1950:\n{}",
        out.listing
    );
    // Frontière de la fenêtre 1950 : yy=50 → 1950 (pas 2050).
    let out = run_det(
        "options yearcutoff=1950;\n\
         data a; d = input('01JAN50', date7.); run;\n\
         proc print data=a; run;\n",
    );
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    // 1950-01-01 = SAS -3652.
    assert!(
        out.listing.contains("-3652"),
        "expected SAS date -3652 (1950-01-01) with YEARCUTOFF=1950:\n{}",
        out.listing
    );
}

/// (2) DATEJUL (fonction) à année 2 chiffres sous le défaut 1926.
/// Base : datejul(7365) lisait 1907-12-31 (défaut 1900). Correction : yy=07
/// → 2007-12-31 = SAS 17531 (fenêtre 1926).
#[test]
fn ra_j02_p9_datejul_two_digit_year_default() {
    let out = run_det("data a; d = datejul(7365); put d=; run;");
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    assert!(
        out.log.contains("d=17531"),
        "expected d=17531 (2007-12-31), got:\n{}",
        out.log
    );
}

/// (2 bis) Constantes date 'ddMONyy'd : fenêtre YEARCUTOFF= au parsing.
/// Base : '01JAN26'd était lu comme l'an 26 de notre ère (résultat faux en
/// silence : SAS date -684576). Correction directe — doc SAS 9.4, SAS
/// System Concepts, « SAS Date, Time, and Datetime Values » : les
/// constantes date suivent YEARCUTOFF= ; SAS les fige à la compilation de
/// l'étape, donc à la valeur en vigueur au parsing du segment.
#[test]
fn ra_j02_p9_date_literal_two_digit_year_window() {
    // Défaut 1926 : '01JAN26'd → 1926-01-01 = SAS -12418 (la fenêtre
    // COMMENCE à 1926 ; l'ancien code lisait l'an 26 de notre ère,
    // SAS date -684576, résultat faux en silence).
    let out = run_det("data a; d = '01JAN26'd; put d=; run;");
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    assert!(
        out.log.contains("d=-12418"),
        "expected d=-12418 (1926-01-01), got:\n{}",
        out.log
    );
    // Seconde moitié de la fenêtre : '01JAN15'd → 2015-12-31 non :
    // 2015-01-01 = SAS 20089.
    let out = run_det("data a; d = '01JAN15'd; put d=; run;");
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    assert!(
        out.log.contains("d=20089"),
        "expected d=20089 (2015-01-01), got:\n{}",
        out.log
    );
    // YEARCUTOFF=1950 posé AVANT l'étape : '01JAN30'd → 2030-01-01 = 25568.
    let out = run_det(
        "options yearcutoff=1950;\n\
         data a; d = '01JAN30'd; put d=; run;\n",
    );
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    assert!(
        out.log.contains("d=25568"),
        "expected d=25568 (2030-01-01) with YEARCUTOFF=1950, got:\n{}",
        out.log
    );
    // Une année à 4 chiffres reste littérale : '01JAN1960'd = 0.
    let out = run_det("data a; d = '01JAN1960'd; put d=; run;");
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    assert!(out.log.contains("d=0"), "expected d=0, got:\n{}", out.log);
}

/// (2 bis) La valeur de SESSION est transmise au parseur segment par
/// segment : un OPTIONS YEARCUTOFF= dans un SEGMENT ANTÉRIEUR s'applique à
/// la constante du segment suivant (le `run;` ouvre un nouveau segment).
#[test]
fn ra_j02_p9_date_literal_yearcutoff_set_between_segments() {
    let out = run_det(
        "data _null_; run;\n\
         options yearcutoff=1950;\n\
         data a; d = '01JAN40'd; put d=; run;\n",
    );
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    // yearcutoff=1950 : 40 → 1940 >= 1950 ? non, 1940 < 1950 → 2040.
    // 2040-01-01 = SAS 29220.
    assert!(
        out.log.contains("d=29220"),
        "expected d=29220 (2040-01-01) with YEARCUTOFF=1950, got:\n{}",
        out.log
    );
}

/// (3/3 bis) NOCENTER/DATE/NUMBER stockés mais pas appliqués au listing →
/// WARNING ; CENTER/NODATE/NONUMBER silencieux. Base : les six formes
/// étaient acceptées sans un mot (stockées, jamais lues).
#[test]
fn ra_j02_p9_ods_options_stored_but_not_applied_warn() {
    // NOCENTER : WARNING + code 1 (le listing est toujours centré).
    let out = run_det("options nocenter;\n");
    assert_eq!(out.exit_code, 1, "log:\n{}", out.log);
    assert!(
        out.log.contains(
            "The NOCENTER option is stored but not applied to the listing; \
             the listing is always centered in this build (planned: roadmap-avancee J11-P5)."
        ),
        "log:\n{}",
        out.log
    );
    // DATE : WARNING + code 1 (le listing n'imprime jamais de date).
    let out = run_det("options date;\n");
    assert_eq!(out.exit_code, 1, "log:\n{}", out.log);
    assert!(
        out.log.contains(
            "The DATE option is stored but not applied to the listing; \
             the listing never prints a date header in this build (planned: roadmap-avancee J11-P5)."
        ),
        "log:\n{}",
        out.log
    );
    // NUMBER : WARNING + code 1 (le listing n'imprime jamais de numéro).
    let out = run_det("options number;\n");
    assert_eq!(out.exit_code, 1, "log:\n{}", out.log);
    assert!(
        out.log.contains(
            "The NUMBER option is stored but not applied to the listing; \
             the listing never prints a page number in this build (planned: roadmap-avancee J11-P5)."
        ),
        "log:\n{}",
        out.log
    );
    // CENTER / NODATE / NONUMBER : correspondent au listing → silence.
    let out = run_det("options center nodate nonumber;\n");
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    assert!(
        !out.log.contains("not applied to the listing"),
        "unexpected warning:\n{}",
        out.log
    );
}

/// (4) PROC OPTIONS imprime les valeurs RÉELLES de la session.
/// Base : PAGESIZE=60, NOCENTER, NODATE codés en dur (src/procs/options.rs)
/// quelle que soit la session. Correction directe — doc SAS 9.4, PROC
/// OPTIONS : « displays the value of the specified system option ».
#[test]
fn ra_j02_p9_proc_options_prints_real_session_values() {
    let out = run_det(
        "options ps=33 yearcutoff=1950;\n\
         proc options; run;\n\
         proc options option=pagesize; run;\n",
    );
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    // PAGESIZE réel (33), pas le 60 codé en dur.
    assert!(out.log.contains("PAGESIZE=33"), "log:\n{}", out.log);
    // YEARCUTOFF réel de la session.
    assert!(out.log.contains("YEARCUTOFF=1950"), "log:\n{}", out.log);
    // CENTER et DATE (défauts SAS, stockés dans ods_options).
    assert!(out.log.contains("CENTER"), "log:\n{}", out.log);
    assert!(out.log.contains("DATE"), "log:\n{}", out.log);
    assert!(out.log.contains("NUMBER"), "log:\n{}", out.log);
    // Après un NODATE explicite, PROC OPTIONS imprime NODATE (valeur réelle).
    let out = run_det("options nodate;\nproc options; run;\n");
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    assert!(out.log.contains("NODATE"), "log:\n{}", out.log);
}

/// (4) « OPTION=nom » : la valeur de l'option DEMANDÉE, pas un WARNING
/// trompeur sur « OPTION ». Base : `proc options option=pagesize;`
/// produisait « WARNING: Option OPTION is not a recognized SAS option. »
/// (le `=pagesize` était avalé). Doc SAS 9.4, PROC OPTIONS statement :
/// `OPTION=option-list`.
#[test]
fn ra_j02_p9_proc_options_option_name_requests_the_option() {
    let out = run_det(
        "options ps=44;\n\
         proc options option=pagesize; run;\n",
    );
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    assert!(
        out.log.contains("PAGESIZE=44"),
        "expected PAGESIZE=44, log:\n{}",
        out.log
    );
    assert!(
        !out.log.contains("not a recognized SAS option"),
        "the old bogus warning on OPTION must be gone:\n{}",
        out.log
    );
}

/// (5) Instruction OPTION (singulier) : alias SAS de OPTIONS.
/// Base : `option ls=80;` était rejeté (« Statement 'OPTION' is not valid
/// or it is used out of proper order »). Correction directe — doc SAS 9.4,
/// OPTIONS statement : « Aliases: option ».
#[test]
fn ra_j02_p9_option_singular_statement_is_accepted() {
    let out = run_det(
        "option ls=80;\n\
         data a; x=1; run;\n\
         proc print data=a; run;\n",
    );
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    assert!(
        !out.log.contains("Statement 'OPTION' is not valid"),
        "log:\n{}",
        out.log
    );
    // L'option est réellement appliquée : la ligne du listing s'étale sur
    // LS=80 (au lieu du défaut 96).
    let out96 = run_det(
        "data a; x=1; run;\n\
         proc print data=a; run;\n",
    );
    assert_ne!(out.listing, out96.listing);
}

/// Le WARNING « not yet supported » des options inconnues ne doit PAS
/// toucher les constructions corrigées (garde de non-régression du
/// contrat J02).
#[test]
fn ra_j02_p9_no_regressions_on_corrected_options() {
    let out = run_det("options firstobs=1 obs=max yearcutoff=1926 center nodate nonumber;\n");
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    assert!(
        !out.log.contains("is not yet supported"),
        "regression: corrected options must not warn:\n{}",
        out.log
    );
}
