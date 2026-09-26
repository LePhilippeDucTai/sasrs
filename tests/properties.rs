//! Tests de propriétés (J05-P4) — proptest, stratégies dédiées par propriété.
//!
//! Six propriétés, chacune avec sa stratégie, un nombre de cas borné et une
//! graine FIXÉE (`RngSeed::Fixed`) : toute défaillance est reproductible.
//!
//! AUCUNE modification de `src/` : tout passe par l'API publique du crate
//! (`sasrs::run`, `SasDataset::write_parquet/read_parquet`, `Value::sas_cmp`,
//! `missing::{encode_special, value_to_num, num_to_value}`,
//! `formats::{FormatCatalog, FormatSpec}`) — le chemin public le plus direct,
//! observé dans les tests existants (cf. `tests/storage_integrity.rs`).
//!
//! Les propriétés qui font tourner une session complète (PDV, PROC SORT)
//! gardent un nombre de cas modeste pour que la suite reste < 30 s.

use polars::prelude::*;
use proptest::prelude::*;
use proptest::test_runner::RngSeed;
use sasrs::dataset::{SasDataset, VarMeta};
use sasrs::formats::{FormatCatalog, FormatSpec};
use sasrs::missing::{encode_special, num_to_value, value_to_num};
use sasrs::value::{MissingKind, Value, VarType};
use sasrs::{RunOptions, run};
use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};

// ─────────────────────────────────────────────────────────────────────────────
// Configuration commune
// ─────────────────────────────────────────────────────────────────────────────

/// Graine fixe : exécutions reproductibles bit à bit.
const SEED: u64 = 0x5A5B_4A05_C0DE;

fn config(cases: u32) -> ProptestConfig {
    let mut c = ProptestConfig::with_cases(cases);
    c.rng_seed = RngSeed::Fixed(SEED);
    c
}

/// Métadonnée numérique minimale (longueur SAS 8).
fn num_meta(name: &str) -> VarMeta {
    VarMeta {
        name: name.to_string(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
    }
}

/// Répertoire de travail unique par cas (sous `$TMPDIR`, donc hors du dépôt).
fn scratch(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, AtomicOrdering::SeqCst);
    let dir =
        std::env::temp_dir().join(format!("sasrs-j05p4-{}-{}-{}", tag, std::process::id(), n));
    std::fs::create_dir_all(&dir).expect("création du répertoire de travail");
    dir
}

fn cleanup(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

// ─────────────────────────────────────────────────────────────────────────────
// Stratégies dédiées
// ─────────────────────────────────────────────────────────────────────────────

/// Les 28 missings SAS dans l'ordre du tri : `._`, `.`, `.A`..`.Z`.
fn all_missing_kinds() -> Vec<MissingKind> {
    let mut v = vec![MissingKind::Underscore, MissingKind::Dot];
    for i in 0..26u8 {
        v.push(MissingKind::Letter(i));
    }
    v
}

fn missing_strategy() -> BoxedStrategy<MissingKind> {
    prop::sample::select(all_missing_kinds()).boxed()
}

/// Numérique fini raisonnable (pas de NaN brut : les NaN à payload sont le
/// domaine exclusif des missings spéciaux).
fn num_strategy() -> BoxedStrategy<f64> {
    prop_oneof![
        -1000.0f64..1000.0,
        -1e9f64..1e9,
        Just(0.0f64),
        Just(-1.0f64),
        Just(f64::MIN_POSITIVE),
        Just(-12345.678),
    ]
    .boxed()
}

fn unicode_char_strategy(pool: &str) -> BoxedStrategy<char> {
    prop::sample::select(pool.chars().collect::<Vec<_>>()).boxed()
}

/// Valeur quelconque : missing, numérique ou caractère (multibyte inclus).
fn value_strategy() -> BoxedStrategy<Value> {
    prop_oneof![
        missing_strategy().prop_map(Value::Missing),
        num_strategy().prop_map(Value::Num),
        prop::collection::vec(unicode_char_strategy("abéö日çβ€ xYZ"), 0..8)
            .prop_map(|cs| Value::Char(cs.into_iter().collect::<String>())),
    ]
    .boxed()
}

// ─────────────────────────────────────────────────────────────────────────────
// Propriété 1 — sas_cmp est un ordre total cohérent avec l'ordre SAS des
// missings (`._` < `.` < `.A` … `.Z` < nombres)
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(config(64))]

    /// Antisymétrie, totalité et transitivité sur un échantillon quelconque.
    #[test]
    fn prop_sas_cmp_is_total_order(values in prop::collection::vec(value_strategy(), 1..10)) {
        for a in &values {
            for b in &values {
                let ab = a.sas_cmp(b);
                let ba = b.sas_cmp(a);
                match ab {
                    Ordering::Less => prop_assert_eq!(ba, Ordering::Greater),
                    Ordering::Equal => prop_assert_eq!(ba, Ordering::Equal),
                    Ordering::Greater => prop_assert_eq!(ba, Ordering::Less),
                }
            }
        }
        for a in &values {
            for b in &values {
                for c in &values {
                    if a.sas_cmp(b) != Ordering::Greater
                        && b.sas_cmp(c) != Ordering::Greater
                    {
                        prop_assert!(a.sas_cmp(c) != Ordering::Greater);
                    }
                }
            }
        }
    }

    /// Chaîne exacte des 28 missings, tous strictement inférieurs à tout
    /// nombre (y compris négatif) — l'ordre SAS documenté.
    #[test]
    fn prop_sas_cmp_missing_chain(neg in -1e12f64..0.0, pos in 1e-6f64..1e12) {
        let kinds = all_missing_kinds();
        for i in 0..kinds.len() {
            for j in 0..kinds.len() {
                let a = Value::Missing(kinds[i]);
                let b = Value::Missing(kinds[j]);
                prop_assert_eq!(a.sas_cmp(&b), kinds[i].cmp(&kinds[j]));
            }
            let m = Value::Missing(kinds[i]);
            prop_assert_eq!(m.sas_cmp(&Value::Num(neg)), Ordering::Less);
            prop_assert_eq!(m.sas_cmp(&Value::Num(pos)), Ordering::Less);
            prop_assert_eq!(Value::Num(neg).sas_cmp(&m), Ordering::Greater);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Propriété 2 — les missings spéciaux survivent à un aller-retour parquet
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(config(32))]

    #[test]
    fn prop_special_missings_parquet_roundtrip(
        misses in prop::collection::vec(missing_strategy(), 1..12),
        nums in prop::collection::vec(num_strategy(), 0..8),
    ) {
        // Colonne : missings (NaN-à-payload, ou null pour `.`) entrelacés
        // avec des nombres normaux.
        let mut values: Vec<Value> = misses.into_iter().map(Value::Missing).collect();
        values.extend(nums.into_iter().map(Value::Num));
        let cells: Vec<Option<f64>> = values.iter().map(value_to_num).collect();
        let df = df!["X" => cells].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("X")] };
        let dir = scratch("miss");
        let path = dir.join("t.parquet");
        ds.write_parquet(&path).unwrap();
        let (back, _notes) = SasDataset::read_parquet(&path).unwrap();
        let out = back.df.column("X").unwrap().f64().unwrap();
        prop_assert_eq!(out.len(), values.len());
        for (i, v) in values.iter().enumerate() {
            let decoded = num_to_value(out.get(i));
            prop_assert_eq!(&decoded, v, "case {}", i);
            // Les BITS du NaN porteur passent tels quels au travers du
            // parquet (payload exact, pas seulement le genre de missing).
            if let Value::Missing(k) = v
                && let Some(f) = out.get(i)
            {
                prop_assert_eq!(f.to_bits(), encode_special(*k).to_bits());
            }
        }
        cleanup(&dir);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Propriété 3 — round-trip VarMeta (format, label, longueur)
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(config(24))]

    #[test]
    fn prop_var_meta_roundtrip(
        char_len in 2usize..24,
        char_body in prop::collection::vec(unicode_char_strategy("abéö日ç "), 0..6),
        num_format_w in 4u16..16,
        num_format_d in 0u16..4,
        label in "[A-Za-z0-9 .-]{0,12}",
    ) {
        let s: String = char_body.into_iter().collect();
        // La longueur déclarée doit être compatible (>= la plus longue
        // valeur) pour être persistée — cf. apply_sidecar_meta.
        let char_len = char_len.max(s.chars().count()).max(1);
        let label_opt = if label.is_empty() { None } else { Some(label) };
        let char_format = format!("${}.", char_len + 3);
        let num_format = format!("{}.{}", num_format_w, num_format_d);
        let vars = vec![
            VarMeta {
                name: "C".to_string(),
                ty: VarType::Char,
                length: char_len,
                format: Some(char_format.clone()),
                label: label_opt.clone(),
            },
            VarMeta {
                name: "N".to_string(),
                ty: VarType::Num,
                length: 8,
                format: Some(num_format.clone()),
                label: None,
            },
        ];
        let df = df![
            "C" => [s.clone()],
            "N" => [3.25f64],
        ].unwrap();
        let ds = SasDataset { df, vars };
        let dir = scratch("meta");
        let path = dir.join("t.parquet");
        ds.write_parquet(&path).unwrap();
        let (back, _notes) = SasDataset::read_parquet(&path).unwrap();
        prop_assert_eq!(back.vars.len(), 2);

        let c = &back.vars[0];
        prop_assert_eq!(&c.name, "C");
        prop_assert_eq!(c.ty, VarType::Char);
        prop_assert_eq!(c.length, char_len);
        prop_assert_eq!(&c.format, &Some(char_format));
        prop_assert_eq!(&c.label, &label_opt);

        let n = &back.vars[1];
        prop_assert_eq!(&n.name, "N");
        prop_assert_eq!(n.ty, VarType::Num);
        prop_assert_eq!(n.length, 8);
        prop_assert_eq!(&n.format, &Some(num_format));
        prop_assert_eq!(&n.label, &None);

        // Les données elles-mêmes ne bougent pas.
        prop_assert_eq!(
            back.df.column("C").unwrap().str().unwrap().get(0).map(str::to_string),
            Some(s)
        );
        cleanup(&dir);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Propriété 4 — troncature PDV : jamais d'UTF-8 invalide, longueur en
// CARACTÈRES (pas en octets)
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(config(24))]

    #[test]
    fn prop_pdv_truncation_counts_chars(
        len in 1usize..10,
        body in prop::collection::vec(unicode_char_strategy("éö日çβ€aB "), 0..24),
    ) {
        let s: String = body.into_iter().collect();
        let s = s.trim_end().to_string(); // le PDV stocke trimé
        // Échapper un éventuel apostrophe pour le littéral SAS.
        let lit = s.replace('\'', "''");
        let dir = scratch("pdv");
        let program = format!(
            "libname d '{}';\ndata d.t;\nlength s $ {len};\ns = '{lit}';\noutput;\nrun;\n",
            dir.display()
        );
        let out = run(&program, RunOptions {
            base_dir: Some(dir.clone()),
            deterministic: true,
            ..Default::default()
        });
        prop_assert_eq!(out.exit_code, 0, "log SAS:\n{}", out.log);

        let (back, _notes) = SasDataset::read_parquet(&dir.join("t.parquet")).unwrap();
        let raw = back.df.column("s").unwrap().str().unwrap().get(0).expect("une observation");
        // Toujours de l'UTF-8 valide.
        prop_assert!(std::str::from_utf8(raw.as_bytes()).is_ok());
        // Longueur respectée en CARACTÈRES, pas en octets.
        prop_assert!(
            raw.chars().count() <= len,
            "'{}' fait {} chars > {}", raw, raw.chars().count(), len
        );
        // La valeur est le PRÉFIXE (en caractères) de l'entrée, trimée.
        let expected: String = s.chars().take(len).collect::<String>().trim_end().to_string();
        prop_assert_eq!(raw, expected);
        cleanup(&dir);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Propriété 5 — `$w.` et `w.d` : jamais de panique, largeur en caractères
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(config(64))]

    #[test]
    fn prop_dollar_w_counts_chars(
        w in 1u16..32,
        body in prop::collection::vec(unicode_char_strategy("éö日çβ€aB9 "), 0..20),
        miss in missing_strategy(),
    ) {
        let s: String = body.into_iter().collect();
        let cat = FormatCatalog::default();

        // $w. : tronque/padde à EXACTEMENT w CARACTÈRES (multibyte inclus).
        let spec = FormatSpec::parse(&format!("${}.", w)).expect("format $w. parse");
        prop_assert_eq!(spec.w, Some(w));
        let out = cat.format(&Value::Char(s.clone()), &spec);
        prop_assert_eq!(out.chars().count(), w as usize, "in='{}' out='{}'", s, out);
        let prefix: String = s.chars().take(w as usize).collect();
        let padded = format!("{:<width$}", prefix, width = w as usize);
        prop_assert_eq!(out, padded);
        // Missing via $w. : le caractère du missing, justifié droite sur w.
        let out = cat.format(&Value::Missing(miss), &spec);
        prop_assert_eq!(out.chars().count(), w as usize);

        // w.d numérique : juste droite sur w, ou `*` si débordement —
        // EXACTEMENT w caractères dans tous les cas, jamais de panique.
        let spec = FormatSpec::parse(&format!("{}.{}", w, 3)).expect("format w.d parse");
        prop_assert_eq!(&spec.name, "");
        let out = cat.format(&Value::Num(1234.5678), &spec);
        prop_assert_eq!(out.chars().count(), w as usize, "w={} out='{}'", w, out);
        let out = cat.format(&Value::Num(-0.005), &spec);
        prop_assert_eq!(out.chars().count(), w as usize, "w={} out='{}'", w, out);
        let out = cat.format(&Value::Missing(miss), &spec);
        prop_assert_eq!(out.chars().count(), w as usize);

        // w. sans décimales : même contrat.
        let spec = FormatSpec::parse(&format!("{}.", w)).expect("format w. parse");
        let out = cat.format(&Value::Num(98.7), &spec);
        prop_assert_eq!(out.chars().count(), w as usize, "w={} out='{}'", w, out);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Propriété 6 — SORT stable et conforme à sas_cmp
// ─────────────────────────────────────────────────────────────────────────────

/// Clé numérique : missing (28 sortes) ou entier — petit domaine pour créer
/// des ÉGALITÉS et vraiment éprouver la stabilité.
fn sort_key_strategy() -> BoxedStrategy<Value> {
    prop_oneof![
        missing_strategy().prop_map(Value::Missing),
        (-6i64..6).prop_map(|i| Value::Num(i as f64)),
    ]
    .boxed()
}

/// Clé caractère sur un alphabet minuscule (égalités fréquentes).
fn sort_char_key_strategy() -> BoxedStrategy<String> {
    prop::collection::vec(prop::sample::select(vec!['a', 'b', 'c']), 0..2)
        .prop_map(|cs| cs.into_iter().collect())
        .boxed()
}

/// Exécute PROC SORT sur `in` → `sorted` dans `dir`, rend la colonne K
/// décodée en Values + la colonne ORD d'origine.
fn sort_and_read(dir: &Path, input: SasDataset) -> (Vec<Value>, Vec<i64>) {
    input.write_parquet(&dir.join("src.parquet")).unwrap();
    let program = format!(
        "libname d '{}';\nproc sort data=d.src out=d.sorted;\nby k;\nrun;\n",
        dir.display()
    );
    let out = run(
        &program,
        RunOptions {
            base_dir: Some(dir.to_path_buf()),
            deterministic: true,
            ..Default::default()
        },
    );
    assert_eq!(out.exit_code, 0, "log SAS:\n{}", out.log);
    let (back, _notes) = SasDataset::read_parquet(&dir.join("sorted.parquet")).unwrap();
    let kcol = back.df.column("K").unwrap();
    let ocol = back.df.column("ORD").unwrap().f64().unwrap();
    let keys: Vec<Value> = if kcol.dtype() == &DataType::String {
        let kc = kcol.str().unwrap();
        (0..kc.len())
            .map(|i| Value::Char(kc.get(i).unwrap_or("").to_string()))
            .collect()
    } else {
        let kc = kcol.f64().unwrap();
        (0..kc.len()).map(|i| num_to_value(kc.get(i))).collect()
    };
    let ords: Vec<i64> = (0..ocol.len())
        .map(|i| ocol.get(i).unwrap() as i64)
        .collect();
    (keys, ords)
}

proptest! {
    #![proptest_config(config(16))]

    /// Clé numérique (missings inclus) : sortie = tri STABLE des entrées
    /// selon sas_cmp — les égalités gardent l'ordre d'origine.
    #[test]
    fn prop_sort_numeric_stable_conformant(
        keys in prop::collection::vec(sort_key_strategy(), 0..20)
    ) {
        let dir = scratch("sortn");
        let n = keys.len();
        let cells: Vec<Option<f64>> = keys.iter().map(value_to_num).collect();
        let ords: Vec<i64> = (0..n as i64).collect();
        let df = df![
            "K" => cells,
            "ORD" => ords,
        ].unwrap();
        let ds = SasDataset {
            df,
            vars: vec![num_meta("K"), num_meta("ORD")],
        };
        let (got, got_ord) = sort_and_read(&dir, ds);
        prop_assert_eq!(got.len(), n);

        // Ordonné au sens de sas_cmp.
        for w in got.windows(2) {
            prop_assert!(w[0].sas_cmp(&w[1]) != Ordering::Greater,
                "sort non ordonné: {:?}", got);
        }
        // Stable : identique au tri stable de référence (sort_by + sas_cmp).
        let mut expect: Vec<(&Value, i64)> = keys.iter().zip(0..n as i64).collect();
        expect.sort_by(|a, b| a.0.sas_cmp(b.0));
        for i in 0..n {
            prop_assert_eq!(&got[i], expect[i].0, "case {}", i);
            prop_assert_eq!(got_ord[i], expect[i].1, "instable en {}", i);
        }
        cleanup(&dir);
    }

    /// Clé caractère : même propriété (collation trim_end de sas_cmp).
    #[test]
    fn prop_sort_char_stable_conformant(
        keys in prop::collection::vec(sort_char_key_strategy(), 0..20)
    ) {
        let dir = scratch("sortc");
        let n = keys.len();
        let ords: Vec<i64> = (0..n as i64).collect();
        let df = df![
            "K" => keys.clone(),
            "ORD" => ords,
        ].unwrap();
        let ds = SasDataset {
            df,
            vars: vec![
                VarMeta {
                    name: "K".to_string(),
                    ty: VarType::Char,
                    length: 2,
                    format: None,
                    label: None,
                },
                num_meta("ORD"),
            ],
        };
        let (got, got_ord) = sort_and_read(&dir, ds);
        prop_assert_eq!(got.len(), n);

        for w in got.windows(2) {
            prop_assert!(w[0].sas_cmp(&w[1]) != Ordering::Greater,
                "sort non ordonné: {:?}", got);
        }
        let key_values: Vec<Value> = keys.into_iter().map(Value::Char).collect();
        let mut expect: Vec<(&Value, i64)> = key_values.iter().zip(0..n as i64).collect();
        expect.sort_by(|a, b| a.0.sas_cmp(b.0));
        for i in 0..n {
            prop_assert_eq!(&got[i], expect[i].0, "case {}", i);
            prop_assert_eq!(got_ord[i], expect[i].1, "instable en {}", i);
        }
        cleanup(&dir);
    }
}
