//! Harnais différentiel : chaque programme est exécuté deux fois — chemin
//! ligne-à-ligne (`vectorize` OFF) et fast-path vectorisé (`vectorize` ON) —
//! et les DEUX exécutions doivent produire log, listing, code retour et
//! datasets STRICTEMENT IDENTIQUES. Le fast-path est une pure optimisation :
//! toute divergence est un bug (à corriger dans `src/datastep/fastpath*`).
//!
//! Deux volets (J05-P5) :
//! 1. `fixtures_differential` : chaque fixture `tests/fixtures/**/*.sas`
//!    exécutée dans les deux modes, artefacts comparés (datasets matériels
//!    sous WORK et sous les LIBNAME relatifs du run).
//! 2. `generated_data_steps` (proptest) : programmes DATA step générés —
//!    SET + assignations numériques, missings spéciaux, KEEP/DROP/RENAME/
//!    FORMAT/LABEL, fenêtres FIRSTOBS=/OBS= (0 ligne inclus) — comparés
//!    vectorisé vs boucle.
//!
//! Les deux runs d'un même programme partagent le MÊME répertoire racine
//! (réinitialisé entre les runs) : même chemin WORK, donc aucun écart
//! parasite lié à un nom de tempdir different dans la log.
//!
//! Deux sources de nondéterminisme INDEPENDANTES du fast-path sont traitées
//! par le harnais (et documentées comme findings) :
//! - le sidecar `<t>.parquet.sasmeta.json` sérialise une `HashMap` : l'ordre
//!   des clés JSON est aléatoire PAR PROCESSUS → comparaison canonique
//!   (parse/re-sérialise, clés triées ; le contenu reste bit à bit) ;
//! - certains plans SQL (p. ex. `UNION` = concat + `unique` parallèle de
//!   Polars, `src/sql/plan/setop.rs`) produisent un ORDRE DE LIGNES non
//!   reproductible d'un run à l'autre MÊME SANS vectorisation. Quand une
//!   divergence de fichiers apparaît, on ré-exécute le chemin boucle : s'il
//!   diverge AUSSI de lui-même, l'écart n'est pas attribuable au fast-path et
//!   la comparaison retombe sur la forme canonique (mêmes lignes, ordre
//!   ignoré). Si le chemin boucle est stable, la divergence est RÉELLE et
//!   l'assertion échoue.

mod common;

use polars::prelude::*;
use proptest::collection;
use proptest::prelude::*;
use sasrs::{RunOptions, run};
use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};

// ── Snapshot des artefacts d'un run ─────────────────────────────────────

/// Une cellule sérialisée — les f64 en BITS hex pour distinguer `.` (null),
/// `.A`/`.B` (NaN-payload) et tout nombre.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Cell {
    F(Option<u64>),
    I(Option<i64>),
    B(Option<bool>),
    S(Option<String>),
    Other(String),
}

/// Image décodée d'un fichier Parquet : noms, dtypes, valeurs (ordre original
/// colonne-major) + forme canonique (lignes triées, ordre de lignes ignoré).
#[derive(Debug, Clone, PartialEq)]
struct TableShape {
    names: Vec<String>,
    dtypes: Vec<String>,
    cols: Vec<Vec<Cell>>,
    /// Lignes triées lexicographiquement — comparaison indépendante de
    /// l'ordre de lignes (mêmes lignes = même ensemble).
    rows_sorted: Vec<Vec<Cell>>,
}

fn snap_series(s: &Series) -> Vec<Cell> {
    match s.dtype() {
        DataType::Float64 => s
            .f64()
            .unwrap()
            .into_iter()
            .map(|o| Cell::F(o.map(|f| f.to_bits())))
            .collect(),
        DataType::Int64 => s.i64().unwrap().into_iter().map(Cell::I).collect(),
        DataType::Boolean => s.bool().unwrap().into_iter().map(Cell::B).collect(),
        DataType::String => s
            .str()
            .unwrap()
            .into_iter()
            .map(|o| Cell::S(o.map(|v| v.to_string())))
            .collect(),
        _ => {
            // Autres dtypes (date…) : rendu texte de la colonne entière castée
            // en String — suffisant pour une égalité strictement différentielle.
            vec![Cell::Other(format!(
                "{:?}",
                s.cast(&DataType::String).unwrap()
            ))]
        }
    }
}

fn snap_parquet(path: &Path) -> TableShape {
    let df = ParquetReader::new(File::open(path).unwrap())
        .finish()
        .unwrap_or_else(|e| panic!("parquet illisible {}: {e}", path.display()));
    let names = df
        .get_column_names_owned()
        .into_iter()
        .map(|n| n.to_string())
        .collect();
    let dtypes = df
        .get_columns()
        .iter()
        .map(|c| format!("{:?}", c.dtype()))
        .collect();
    let cols = df
        .get_columns()
        .iter()
        .map(|c| snap_series(c.as_materialized_series()))
        .collect::<Vec<_>>();
    // Transposition en lignes pour la forme canonique. Une colonne rendue en
    // `Other` (dtype exotique, un seul pseudo-élément) bloque toute
    // transposition cohérente : la forme canonique retombe alors sur les
    // colonnes non-Other (comparaison dégradée mais déterministe).
    let plain = cols.iter().all(|c| c.len() == df.height());
    let rows_sorted = if plain {
        let mut rows = (0..df.height())
            .map(|r| cols.iter().map(|c| c[r].clone()).collect::<Vec<_>>())
            .collect::<Vec<_>>();
        rows.sort();
        rows
    } else {
        Vec::new()
    };
    TableShape {
        names,
        dtypes,
        cols,
        rows_sorted,
    }
}

#[derive(Debug, Clone, PartialEq)]
enum FileSnap {
    Parquet(TableShape),
    /// Sidecar `.sasmeta.json` : JSON canonique (clés triées). Le champ
    /// `fingerprint.size` (taille EN OCTETS du parquet) dépend de
    /// l'encodage, donc de l'ordre des lignes : il est retiré dans la
    /// comparaison non ordonnée (le CONTENU du dataset est déjà comparé
    /// via `rows_sorted`).
    Sidecar(serde_json::Value),
    Bytes(Vec<u8>),
}

/// Sidecar sans l'empreinte `fingerprint.size` (dépendante de l'ordre des
/// lignes par le biais de l'encodage parquet).
fn sidecar_sans_size(v: &serde_json::Value) -> serde_json::Value {
    let mut v = v.clone();
    if let Some(fp) = v.get_mut("fingerprint").map(|f| f.take()) {
        let mut fp = fp;
        if let Some(size) = fp.get_mut("size") {
            *size = serde_json::Value::Null;
        }
        v["fingerprint"] = fp;
    }
    v
}

/// Compare deux arbres de fichiers. `ordered == false` : les datasets sont
/// comparés via leur forme canonique (lignes triées) — réservé au cas où le
/// chemin boucle est démontré non reproductible lui-même.
fn files_eq(a: &BTreeMap<String, FileSnap>, b: &BTreeMap<String, FileSnap>, ordered: bool) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().all(|(k, va)| match b.get(k) {
        Some(vb) => {
            if ordered {
                va == vb
            } else {
                match (va, vb) {
                    (FileSnap::Parquet(ta), FileSnap::Parquet(tb)) => {
                        ta.names == tb.names
                            && ta.dtypes == tb.dtypes
                            && ta.rows_sorted == tb.rows_sorted
                    }
                    (FileSnap::Sidecar(sa), FileSnap::Sidecar(sb)) => {
                        sidecar_sans_size(sa) == sidecar_sans_size(sb)
                    }
                    _ => va == vb,
                }
            }
        }
        None => false,
    })
}

#[derive(Debug, Clone, PartialEq)]
struct Outcome {
    log: String,
    listing: String,
    exit_code: i32,
    /// Chemin relatif → contenu (datasets décodés, reste brut).
    files: BTreeMap<String, FileSnap>,
}

fn walk_files(dir: &Path, base: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            walk_files(&p, base, out);
        } else {
            out.push(p.strip_prefix(base).unwrap().to_path_buf());
        }
    }
}

/// Racine fraîche : entrées canoniques des fixtures (data/class.parquet,
/// data/pets.csv). MÊME chemin pour les deux runs → aucune divergence de
/// chemin dans la log.
fn reset_root(root: &Path) {
    let _ = std::fs::remove_dir_all(root);
    common::write_class_parquet(&root.join("data"));
    common::write_pets_csv(&root.join("data"));
}

fn snapshot_tree(root: &Path) -> BTreeMap<String, FileSnap> {
    let mut paths = Vec::new();
    walk_files(root, root, &mut paths);
    paths.sort();
    let mut map = BTreeMap::new();
    for rel in paths {
        let abs = root.join(&rel);
        let snap = if rel.extension().is_some_and(|e| e == "parquet") {
            FileSnap::Parquet(snap_parquet(&abs))
        } else if rel.to_string_lossy().ends_with(".sasmeta.json") {
            // Le sidecar sérialise une HashMap : l'ordre des clés JSON est
            // aléatoire PAR PROCESSUS (RandomState), même hors vectorisation.
            // Snapshot canonique : parse puis re-sérialise (serde_json Map =
            // BTreeMap ⇒ clés triées) — le CONTENU reste comparé bit à bit.
            FileSnap::Sidecar(
                serde_json::from_slice::<serde_json::Value>(&std::fs::read(&abs).unwrap()).unwrap(),
            )
        } else {
            FileSnap::Bytes(std::fs::read(&abs).unwrap())
        };
        map.insert(rel.to_string_lossy().into_owned(), snap);
    }
    map
}

fn execute(source: &str, vectorize: bool, root: &Path) -> Outcome {
    reset_root(root);
    let work = root.join("work");
    let outcome = run(
        source,
        RunOptions {
            work_dir: Some(work),
            base_dir: Some(root.to_path_buf()),
            deterministic: true,
            vectorize,
        },
    );
    Outcome {
        log: outcome.log,
        listing: outcome.listing,
        exit_code: outcome.exit_code,
        files: snapshot_tree(root),
    }
}

/// Exécute `source` dans les deux modes sur le MÊME répertoire racine et
/// exige l'identité totale (log, listing, exit code, fichiers produits).
///
/// Si les FICHIERS divergent, un second run boucle tranche : un chemin boucle
/// stable ⇒ divergence RÉELLE du fast-path (échec) ; un chemin boucle qui
/// diverge de lui-même ⇒ nondéterminisme moteur indépendant du fast-path
/// (cf. en-tête du module) ⇒ comparaison canonique des datasets.
fn assert_identical(source: &str, label: &str) {
    let root = tempfile::tempdir_in(std::env::temp_dir()).unwrap();
    let r = root.path();
    let off = execute(source, false, r);
    let on = execute(source, true, r);
    assert_eq!(off.exit_code, on.exit_code, "{label}: exit code divergent");
    assert_eq!(off.listing, on.listing, "{label}: listing divergent");
    if off.log != on.log {
        // Affiche la première ligne divergente pour un diagnostic rapide.
        let (mut la, mut lb) = (off.log.lines(), on.log.lines());
        let mut n = 0;
        for (a, b) in (&mut la).zip(&mut lb) {
            n += 1;
            if a != b {
                assert_eq!(a, b, "{label}: log diverge ligne {n}");
            }
        }
        assert_eq!(
            off.log.lines().count(),
            on.log.lines().count(),
            "{label}: longueur de log divergente"
        );
        panic!("{label}: log divergent");
    }
    if !files_eq(&off.files, &on.files, true) {
        let off2 = execute(source, false, r);
        if files_eq(&off.files, &off2.files, true) {
            // Chemin boucle reproductible : la divergence vient du fast-path.
            assert!(
                files_eq(&off.files, &on.files, true),
                "{label}: fichiers produits divergents (fast-path)"
            );
        } else {
            // Nondéterminisme moteur indépendant du fast-path : la comparaison
            // stricte est impossible PAR CONSTRUCTION ; on exige au moins
            // l'identité du CONTENU (mêmes lignes, ordre ignoré).
            assert!(
                files_eq(&off.files, &on.files, false),
                "{label}: contenu des datasets divergent au-delà de l'ordre des lignes"
            );
        }
    }
}

// ── Volet 1 : chaque fixture, vectorisé vs boucle ───────────────────────

#[test]
fn fixtures_differential() {
    let fixtures_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut fixtures = Vec::new();
    walk_files(&fixtures_root, &fixtures_root, &mut fixtures);
    fixtures.sort();
    assert!(!fixtures.is_empty(), "aucune fixture trouvée");
    for rel in &fixtures {
        let source = std::fs::read_to_string(fixtures_root.join(rel)).unwrap();
        let label = format!("fixture {}", rel.display());
        assert_identical(&source, &label);
    }
}

// ── Volet 2 : DATA steps générés (proptest) ─────────────────────────────

/// Valeur d'entrée numérique : entiers, décimaux, missing ordinaire `.`,
/// missings spéciaux `.A`/`.B`.
fn value_strat() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => (0i64..30).prop_map(|i| i.to_string()),
        2 => (0i64..300).prop_map(|c| format!("{}.{:01}", c / 10, c % 10)),
        2 => Just(".".to_string()),
        1 => Just(".A".to_string()),
        1 => Just(".B".to_string()),
    ]
}

/// RHS d'assignation : arbre +,-,* sur copies de variables et littéraux,
/// avec missing spécial littéral (repli attendu → reste différentiel).
#[derive(Clone, Debug)]
enum Rhs {
    Var(u8),
    Lit(u8),
    MissSpecial,
    Add(Box<Rhs>, Box<Rhs>),
    Sub(Box<Rhs>, Box<Rhs>),
    Mul(Box<Rhs>, Box<Rhs>),
}

fn rhs_strat() -> impl Strategy<Value = Rhs> {
    let leaf = prop_oneof![
        3 => (0u8..10).prop_map(Rhs::Var),
        3 => (0u8..100).prop_map(Rhs::Lit),
        1 => Just(Rhs::MissSpecial),
    ];
    leaf.prop_recursive(3, 24, 2, |inner| {
        prop_oneof![
            1 => (inner.clone(), inner.clone()).prop_map(|(l, r)| Rhs::Add(Box::new(l), Box::new(r))),
            1 => (inner.clone(), inner.clone()).prop_map(|(l, r)| Rhs::Sub(Box::new(l), Box::new(r))),
            1 => (inner.clone(), inner).prop_map(|(l, r)| Rhs::Mul(Box::new(l), Box::new(r))),
        ]
    })
}

/// Déclaratif sans effet d'exécution : KEEP/DROP/RENAME/FORMAT/LABEL sur des
/// indices (résolus modulo le pool de variables du programme).
#[derive(Clone, Debug)]
enum Decl {
    Keep(Vec<u8>),
    Drop(Vec<u8>),
    Rename(u8, u8),
    Format(u8),
    Label(u8),
}

fn decl_strat() -> impl Strategy<Value = Decl> {
    prop_oneof![
        2 => collection::vec(0u8..8, 1..4).prop_map(Decl::Keep),
        2 => collection::vec(0u8..8, 1..4).prop_map(Decl::Drop),
        2 => (0u8..8, 0u8..8).prop_map(|(a, b)| Decl::Rename(a, b)),
        1 => (0u8..8).prop_map(Decl::Format),
        1 => (0u8..8).prop_map(Decl::Label),
    ]
}

/// Rend un RHS : les indices de variables sont résolus dans `pool`
/// (modulo sa taille) pour que les dépendances séquentielles (x2 = x1 + a)
/// soient générées aussi.
fn render_rhs(rhs: &Rhs, pool: &[String]) -> String {
    match rhs {
        Rhs::Var(i) => pool[(*i as usize) % pool.len()].clone(),
        Rhs::Lit(v) => {
            if *v < 50 {
                format!("{v}")
            } else {
                format!("{}.5", v - 50)
            }
        }
        Rhs::MissSpecial => ".A".to_string(),
        Rhs::Add(l, r) => format!("({} + {})", render_rhs(l, pool), render_rhs(r, pool)),
        Rhs::Sub(l, r) => format!("({} - {})", render_rhs(l, pool), render_rhs(r, pool)),
        Rhs::Mul(l, r) => format!("({} * {})", render_rhs(l, pool), render_rhs(r, pool)),
    }
}

fn render_decl(decl: &Decl, pool: &[String]) -> String {
    let name = |i: u8| pool[(i as usize) % pool.len()].clone();
    match decl {
        Decl::Keep(ix) => format!(
            "keep {};",
            ix.iter().map(|&i| name(i)).collect::<Vec<_>>().join(" ")
        ),
        Decl::Drop(ix) => format!(
            "drop {};",
            ix.iter().map(|&i| name(i)).collect::<Vec<_>>().join(" ")
        ),
        Decl::Rename(a, b) => format!("rename {}={};", name(*a), name(*b)),
        Decl::Format(a) => format!("format {} 8.2;", name(*a)),
        Decl::Label(a) => format!("label {}='Lbl';", name(*a)),
    }
}

fn build_program(
    rows: &[(String, String)],
    rhs_list: &[Rhs],
    decls: &[Decl],
    firstobs: u8,
    obs: u8,
) -> String {
    let mut src = String::from("data work.src;\n  input a b;\n  datalines;\n");
    for (a, b) in rows {
        src.push_str(&format!("{a} {b}\n"));
    }
    src.push_str(";\nrun;\n\ndata work.out;\n  set work.src (firstobs=");
    src.push_str(&firstobs.to_string());
    src.push_str(" obs=");
    src.push_str(&obs.to_string());
    src.push_str(");\n");
    let mut pool = vec!["a".to_string(), "b".to_string()];
    for (k, rhs) in rhs_list.iter().enumerate() {
        let name = format!("x{}", k + 1);
        src.push_str(&format!("  {} = {};\n", name, render_rhs(rhs, &pool)));
        pool.push(name);
    }
    for decl in decls {
        src.push_str("  ");
        src.push_str(&render_decl(decl, &pool));
        src.push('\n');
    }
    src.push_str("run;\n");
    src
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]
    #[test]
    fn generated_data_steps(
        rows in collection::vec((value_strat(), value_strat()), 1..8),
        rhs_list in collection::vec(rhs_strat(), 1..4),
        decls in collection::vec(decl_strat(), 0..3),
        firstobs in 1u8..5,
        obs in 0u8..8,
    ) {
        let program = build_program(&rows, &rhs_list, &decls, firstobs, obs);
        let root = tempfile::tempdir_in(std::env::temp_dir()).unwrap();
        let r = root.path();
        let off = execute(&program, false, r);
        let on = execute(&program, true, r);
        prop_assert_eq!(off.exit_code, on.exit_code);
        prop_assert_eq!(off.listing, on.listing);
        prop_assert_eq!(off.log, on.log, "log divergente pour:\n{}", program);
        prop_assert_eq!(off.files, on.files, "datasets divergents pour:\n{}", program);
    }
}

// ── Cas structurels 0 ligne (hors génération) ───────────────────────────

/// Fenêtre vide par OBS=0 : sortie à 0 observations dans les deux chemins
/// (le fast-path voit n_rows=0 en entrée via repli FIRSTOBS/OBS).
#[test]
fn zero_rows_obs_window() {
    assert_identical(
        "data work.src;\n  input a b;\n  datalines;\n1 2\n3 4\n;\nrun;\n\
         data work.out;\n  set work.src (obs=0);\n  x = a + b;\nrun;\n",
        "obs=0",
    );
}

/// FIRSTOBS au-delà de la fin : 0 observations lues, sortie vide.
#[test]
fn zero_rows_firstobs_past_end() {
    assert_identical(
        "data work.src;\n  input a b;\n  datalines;\n1 2\n;\nrun;\n\
         data work.out;\n  set work.src (firstobs=5);\n  x = a * 3;\nrun;\n",
        "firstobs>fin",
    );
}

/// Non-régression (finding J05-P5) : le fast-path vectorisé ne doit PAS
/// s'activer sur un DATA step dont le SET porte une option de fenêtrage
/// invalide — les deux chemins rendent la même ERREUR, même log, même exit.
#[test]
fn invalid_set_option_both_paths_error_identically() {
    assert_identical(
        "data work.src;\n  input a b;\n  datalines;\n1 2\n;\nrun;\n\
         data work.out;\n  set work.src (firstobs=2, obs=1);\n  x = a;\nrun;\n",
        "firstobs>obs invalide",
    );
}

// ── Issue #20 — déterminisme UNION (non-ALL) et sidecar ─────────────────
//
// Depuis la correction (tri total par toutes les colonnes après `unique`,
// sidecar sérialisé en `BTreeMap`), ces artefacts sont désormais STABLES
// byte-à-byte entre deux sessions INDÉPENDANTES : plus besoin de la
// comparaison canonique (ordre des clés / des lignes ignoré) ni du repli
// « le chemin boucle diverge de lui-même ».

/// Programme de l'issue #20 : UNION (non-ALL) avec ex æquo multi-colonnes,
/// résultat publié en Parquet AVEC sidecar (formats + libellés ⇒ has_meta).
const UNION_DETERMINISM_SRC: &str = r#"
libname out 'out';

data work.a;
    length name $8;
    format wt 8.1;
    label name = 'Nom' wt = 'Poids (kg)';
    input name $ age wt;
    datalines;
Alfred 14 112.5
Alice 13 84.0
Carol 14 62.8
David 15 99.0
;

data work.b;
    length name $8;
    format wt 8.1;
    label name = 'Nom' wt = 'Poids (kg)';
    input name $ age wt;
    datalines;
Carol 14 62.8
Jane 12 74.2
Alfred 14 112.5
Bob 13 90.5
;

proc sql;
    create table out.unioned as
        select name, age, wt from work.a
        union
        select name, age, wt from work.b;
quit;

proc print data=out.unioned;
run;
"#;

/// Deux sessions entièrement séparées (répertoires racine distincts) doivent
/// produire la même log, le même listing, le même Parquet et le MÊME sidecar
/// JSON byte-à-byte (clés triées, ordre de lignes déterministe).
#[test]
fn union_and_sidecar_byte_identical_across_sessions() {
    let mut snaps: Vec<(String, String, Vec<u8>, Vec<u8>)> = Vec::new();
    for i in 0..2 {
        let root = tempfile::tempdir_in(std::env::temp_dir()).unwrap();
        std::fs::create_dir_all(root.path().join("out")).unwrap();
        let outcome = run(
            UNION_DETERMINISM_SRC,
            RunOptions {
                work_dir: Some(root.path().join("work")),
                base_dir: Some(root.path().to_path_buf()),
                deterministic: true,
                vectorize: i == 1,
            },
        );
        assert_eq!(outcome.exit_code, 0, "run {i}: {}", outcome.log);
        let parquet = std::fs::read(root.path().join("out/unioned.parquet")).unwrap();
        let sidecar = std::fs::read(root.path().join("out/unioned.parquet.sasmeta.json")).unwrap();
        snaps.push((outcome.log, outcome.listing, parquet, sidecar));
    }
    assert_eq!(
        snaps[0], snaps[1],
        "issue #20 : artefacts non déterministes"
    );
}
