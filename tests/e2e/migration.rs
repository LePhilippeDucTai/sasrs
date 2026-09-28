//! Scénario de migration partagé des tests E2E (`tests/e2e.rs`).
//!
//! Un programme SAS réaliste — import CSV + import/export XLSX → étape DATA
//! → SORT → MEANS NWAY avec `OUTPUT` → TRANSPOSE → COMPARE contre une table
//! attendue → ODS OUTPUT → EXPORT XLSX — est exécuté deux fois : par le
//! binaire `sasrs` (contrat CLI, cf. `tests/cli.rs`) et via la façade
//! `sasrs::api`. Les attendus sont calculés à la main sur le jeu de données
//! (formule BMI publiée : 703 × lb / in², cf. la documentation CDC/SAS) et
//! les diagnostics attendus sont verrouillés par la log.

use std::fs;
use std::path::Path;

/// Jeu de données de migration : 8 patients, 5 femmes / 3 hommes.
/// Colonnes importées telles quelles par `PROC IMPORT DBMS=CSV`.
pub const PATIENTS_CSV: &str = "\
Name,Sex,Age,Height,Weight
Alfred,M,14,69.0,112.5
Alice,F,13,56.5,84.0
Barbara,F,13,65.3,98.0
Carol,F,14,62.8,102.5
Henry,M,14,63.5,102.5
James,M,12,57.3,83.0
Jane,F,12,59.8,84.5
Janet,F,15,62.5,112.5
";

/// Moyennes BMI attendues (BMI = 703 × weight / height²), groupe par groupe :
///   F : Alice 18.498551 + Barbara 16.156788 + Carol 18.270898
///       + Jane 16.611531 + Janet 20.246400  → 17.956833843761384
///   M : Alfred 16.611531 + Henry 17.870296 + James 17.771504
///       → 17.417776838271262
/// (mêmes valeurs que `work.expected_wide` dans le programme ; le COMPARE du
/// programme et les assertions Rust utilisent un critère absolu de 1e-9.)
pub const EXPECTED_MEAN_F: f64 = 17.956_833_843_761_384;
pub const EXPECTED_MEAN_M: f64 = 17.417_776_838_271_262;
/// `n(bmi)` par groupe : 5 femmes, 3 hommes.
pub const EXPECTED_N_F: f64 = 5.0;
pub const EXPECTED_N_M: f64 = 3.0;

/// Programme de migration complet. Écrit dans le dossier du script ; les
/// chemins relatifs (`patients.csv`, `*.xlsx`) y sont résolus.
pub const PROGRAM: &str = r#"
proc import datafile='patients.csv' out=work.raw dbms=csv replace;
run;

data work.clin;
    set work.raw;
    bmi = 703 * weight / (height * height);
run;

proc sort data=work.clin out=work.sorted;
    by sex name;
run;

/* La table de correspondance sexe → région transite par un classeur XLSX :
 * export puis import, comme dans une migration réelle. */
data work.region;
    length sex $1 region $10;
    sex = 'F'; region = 'North'; output;
    sex = 'M'; region = 'South'; output;
run;

proc export data=work.region outfile='lookup.xlsx' dbms=xlsx replace;
run;

proc import datafile='lookup.xlsx' out=work.region_in dbms=xlsx;
run;

proc sort data=work.region_in out=work.region_sorted;
    by sex;
run;

data work.enriched;
    merge work.sorted(in=a) work.region_sorted;
    by sex;
    if a;
run;

proc means data=work.enriched nway noprint;
    class sex;
    var bmi;
    output out=work.bmi_stats mean(bmi)=bmi_mean n(bmi)=bmi_n;
run;

proc transpose data=work.bmi_stats out=work.bmi_wide;
    id sex;
    var bmi_mean bmi_n;
run;

/* Attendus : moyennes BMI exactes (formule publiée 703×lb/in²) et effectifs
 * par groupe — le COMPARE juge l'égalité au critère absolu 1e-9. */
data work.expected_wide;
    length _name_ $8;
    _name_ = 'BMI_MEAN'; F = 17.956833843761; M = 17.417776838271;
    output;
    _name_ = 'BMI_N'; F = 5; M = 3;
    output;
run;

proc compare base=work.expected_wide compare=work.bmi_wide
             out=work.wide_diffs outnoequal
             criterion=0.000000001 method=absolute;
run;

/* Capture ODS OUTPUT de l'objet « Summary » d'un MEANS d'affichage. */
ods output Summary=work.summary_capture;
proc means data=work.enriched mean std maxdec=4;
    class sex;
    var bmi;
run;
ods output close;

/* Le résultat de la migration est livré en XLSX et relu : aller-retour sans perte
 * (COMPARE contre la table transposée elle-même, critère 1e-6). */
proc export data=work.bmi_wide outfile='bmi_wide.xlsx' dbms=xlsx replace;
run;

proc import datafile='bmi_wide.xlsx' out=work.bmi_wide_back dbms=xlsx;
run;

proc compare base=work.bmi_wide compare=work.bmi_wide_back
             out=work.back_diffs outnoequal
             criterion=0.000001 method=absolute;
run;
"#;

/// Même programme, mais l'attendu `M` de `BMI_MEAN` est volontairement faux :
/// le COMPARE DOIT produire des lignes dans `OUT=… OUTNOEQUAL` (propriété
/// « un test qui prouve une propriété échoue quand la propriété est brisée »,
/// cf. CONTRIBUTING.md).
pub const PROGRAM_BROKEN_EXPECTATION: &str = r#"
proc import datafile='patients.csv' out=work.raw dbms=csv replace;
run;

data work.clin;
    set work.raw;
    bmi = 703 * weight / (height * height);
run;

proc means data=work.clin nway noprint;
    class sex;
    var bmi;
    output out=work.bmi_stats mean(bmi)=bmi_mean n(bmi)=bmi_n;
run;

proc transpose data=work.bmi_stats out=work.bmi_wide;
    id sex;
    var bmi_mean bmi_n;
run;

data work.expected_wide;
    length _name_ $8;
    _name_ = 'BMI_MEAN'; F = 17.956833843761; M = 42.0;
    output;
    _name_ = 'BMI_N'; F = 5; M = 3;
    output;
run;

proc compare base=work.expected_wide compare=work.bmi_wide
             out=work.wide_diffs outnoequal
             criterion=0.000000001 method=absolute;
run;
"#;

/// Écrit `patients.csv` dans `dir` (dossier du script : les chemins relatifs
/// du programme y sont résolus, cf. le contrat CLI).
pub fn write_patients_csv(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("patients.csv"), PATIENTS_CSV).unwrap();
}

/// Écrit le programme de migration `migration.sas` dans `dir`.
pub fn write_program(dir: &Path, name: &str, program: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join(name), program).unwrap();
}
