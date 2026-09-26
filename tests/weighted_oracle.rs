//! J03-P2 — oracle pondéré de bout en bout.
//!
//! Chaque cas de `tests/oracles/weighted_stats.json` (oracle indépendant
//! écrit par J03-P1, hors périmètre de ce changement : il n'est JAMAIS
//! modifié par l'implémenteur) est exécuté via `sasrs::run` : la table est
//! construite par DATALINES, la PROC (MEANS ou UNIVARIATE, VARDEF=/EXCLNPWGT
//! selon le cas) écrit ses statistiques via `OUTPUT OUT=`, et le dataset
//! résultat est RELU depuis le parquet WORK pour comparaison à `expected`
//! (tolérance relative 1e-10).
//!
//! Un cas marqué `uncertain` en échec est SIGNALÉ (préfixe UNCERTAIN dans le
//! message) ; l'oracle n'est jamais réécrit pour faire passer un test.

use polars::prelude::*;
use serde_json::Value as Json;
use std::collections::BTreeMap;
use std::fs::File;

const ORACLE_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/oracles/weighted_stats.json"
);
const REL_TOL: f64 = 1e-10;

fn cases() -> Vec<Json> {
    let text = std::fs::read_to_string(ORACLE_PATH).expect("oracle JSON lisible");
    let v: Json = serde_json::from_str(&text).expect("oracle JSON valide");
    v["cases"].as_array().expect("cases").clone()
}

/// `data w; input x w; datalines; ... ;` depuis `data.x` / `data.w`
/// (null → `.`).
fn datalines_step(data: &Json) -> String {
    let xs = data["x"].as_array().unwrap();
    let ws = data["w"].as_array().unwrap();
    let mut src = String::from("data w;\n  input x w;\n  datalines;\n");
    for (x, w) in xs.iter().zip(ws.iter()) {
        let fmt = |v: &Json| match v.as_f64() {
            Some(f) => format!("{f}"),
            None => ".".to_string(),
        };
        src.push_str(&format!("{} {}\n", fmt(x), fmt(w)));
    }
    src.push_str(";\nrun;\n");
    src
}

/// Programme MEANS : chaque statistique de l'oracle passe par
/// `OUTPUT OUT=` (`stat(var)=name`). La variance est vérifiée via
/// std² (MEANS n'a pas de mot-clé VAR).
fn means_program(vardef: &str) -> String {
    format!(
        "proc means data=w noprint vardef={vdef};\n  weight w;\n  var x;\n  \
         output out=res n(x)=o_n nmiss(x)=o_nmiss sumwgt(x)=o_sumwgt \
         mean(x)=o_mean std(x)=o_std median(x)=o_median q1(x)=o_q1 \
         q3(x)=o_q3 qrange(x)=o_qrange p1(x)=o_p1 p99(x)=o_p99;\nrun;\n",
        vdef = vardef
    )
}

/// Programme UNIVARIATE (`exclnpwgt` ajouté quand demandé). SUMWGT est
/// vérifié par dérivation sum/mean (UNIVARIATE n'a pas de mot-clé SUMWGT
/// dans OUTPUT) ; la variance utilise le défaut VARDEF=DF (diviseur W−1).
fn univariate_program(exclnpwgt: bool) -> String {
    format!(
        "proc univariate data=w noprint{excl};\n  var x;\n  weight w;\n  \
         output out=res n=o_n nmiss=o_nmiss sum=o_sum mean=o_mean std=o_std \
         var=o_var median=o_median q1=o_q1 q3=o_q3 qrange=o_qrange \
         p1=o_p1 p99=o_p99;\nrun;\n",
        excl = if exclnpwgt { " exclnpwgt" } else { "" },
    )
}

/// Lit le parquet WORK `res` et retourne nom → valeur (None = manquant).
fn read_out(work_dir: &std::path::Path) -> BTreeMap<String, Option<f64>> {
    let path = work_dir.join("res.parquet");
    let mut file = File::open(&path).unwrap_or_else(|e| panic!("open {path:?}: {e}"));
    let df = ParquetReader::new(&mut file)
        .finish()
        .unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    let mut out = BTreeMap::new();
    for col in df.get_columns() {
        let s = col.as_materialized_series();
        let vals: Vec<Option<f64>> = s
            .f64()
            .unwrap_or_else(|e| panic!("colonne {} non numérique: {}", s.name(), e))
            .into_iter()
            .collect();
        out.insert(s.name().to_string(), vals.into_iter().next().flatten());
    }
    out
}

fn close(actual: Option<f64>, expected: Option<f64>) -> Result<(), String> {
    match (actual, expected) {
        (None, None) => Ok(()),
        (Some(a), Some(e)) => {
            let tol = REL_TOL * e.abs().max(1e-12);
            if (a - e).abs() <= tol {
                Ok(())
            } else {
                Err(format!("calcule={a:?} attendu={e:?}"))
            }
        }
        (a, e) => Err(format!("calcule={a:?} attendu={e:?} (manquant divergent)")),
    }
}

fn expected_of(case: &Json, key: &str) -> Option<f64> {
    case["expected"][key].as_f64()
}

#[test]
fn corpus_charge_dix_cas() {
    // Garde-fou : l'oracle doit rester présent et non vide.
    assert!(cases().len() >= 10, "corpus oracle amaigri ?");
}

#[test]
fn weighted_oracle_end_to_end() {
    let mut failures: Vec<String> = Vec::new();
    for case in cases() {
        let id = case["id"].as_str().unwrap_or("?");
        let mode = case["mode"].as_str().unwrap_or("means");
        let vardef = case["vardef"].as_str().unwrap_or("df");
        let uncertain = case.get("uncertain").is_some();

        let program = match mode {
            "means" => format!("{}{}", datalines_step(&case["data"]), means_program(vardef)),
            other => format!(
                "{}{}",
                datalines_step(&case["data"]),
                univariate_program(other == "univariate-exclnpwgt")
            ),
        };

        let tmp = tempfile::tempdir().unwrap();
        let outcome = sasrs::run(
            &program,
            sasrs::RunOptions {
                work_dir: Some(tmp.path().to_path_buf()),
                deterministic: true,
                ..Default::default()
            },
        );
        assert_eq!(outcome.exit_code, 0, "{}: log:\n{}", id, outcome.log);
        let got = read_out(tmp.path());

        let mut bad: Vec<String> = Vec::new();
        // Statistiques directes communes.
        for (key, col) in [
            ("n", "o_n"),
            ("nmiss", "o_nmiss"),
            ("mean", "o_mean"),
            ("std", "o_std"),
            ("median", "o_median"),
            ("q1", "o_q1"),
            ("q3", "o_q3"),
            ("qrange", "o_qrange"),
            ("p1", "o_p1"),
            ("p99", "o_p99"),
        ] {
            if let Err(e) = close(got.get(col).and_then(|v| *v), expected_of(&case, key)) {
                bad.push(format!("{key}: {e}"));
            }
        }
        // SUMWGT : mot-clé direct côté MEANS, dérivé sum/mean côté UNIVARIATE.
        let sumwgt_actual = match mode {
            "means" => got.get("o_sumwgt").and_then(|v| *v),
            _ => match (
                got.get("o_sum").and_then(|v| *v),
                got.get("o_mean").and_then(|v| *v),
            ) {
                (Some(s), Some(m)) if m != 0.0 => Some(s / m),
                (None, None) => None,
                (a, _) => a,
            },
        };
        if let Err(e) = close(sumwgt_actual, expected_of(&case, "sumwgt")) {
            bad.push(format!("sumwgt: {e}"));
        }
        // VAR : direct côté UNIVARIATE, std² côté MEANS.
        let var_actual = match mode {
            "univariate" | "univariate-exclnpwgt" => got.get("o_var").and_then(|v| *v),
            _ => got.get("o_std").and_then(|v| *v).map(|s| s * s),
        };
        if let Err(e) = close(var_actual, expected_of(&case, "var")) {
            bad.push(format!("var: {e}"));
        }

        if !bad.is_empty() {
            let flag = if uncertain {
                // Cas incertain : échec SIGNALÉ, l'oracle n'est jamais réécrit.
                " [UNCERTAIN — signalé, oracle non modifié]"
            } else {
                ""
            };
            failures.push(format!(
                "{id} (mode={mode}, vardef={vardef}){flag}: {bad:?}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "cas en échec contre l'oracle pondéré:\n{}",
        failures.join("\n")
    );
}
