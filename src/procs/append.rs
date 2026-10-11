//! PROC APPEND (jalon M7 + options M33.9).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc append base=a data=b [force] [nowarn] [appendver=Vn] ; run ;`
//!
//! - base inexistante → créée comme copie de data (NOTE SAS).
//! - Sans FORCE : toute variable de DATA absente de BASE, ou type
//!   différent, ou longueur char supérieure → ERROR "No appending done
//!   because of anomalies...". Avec FORCE : variables en trop ignorées
//!   (WARNING), longueurs tronquées à celle de BASE, variables de BASE
//!   absentes de DATA → missing.
//! - Alignement par NOM (pas par position) ; décoder en Vec<Value> et
//!   reconstruire le DataFrame.
//! - NOTEs : "Appending WORK.B to WORK.A." + obs lues / obs ajoutées.
//!
//! ## Options M33.9
//! - NOWARN : avec FORCE, supprime le WARNING sur les différences
//!   structurelles forcées. Sans FORCE, la présence de NOWARN est sans
//!   effet (aucun WARNING n'est émis dans ce chemin). L'append est
//!   toujours réalisé normalement — NOWARN ne change pas le résultat,
//!   seulement la verbosité du log.
//! - APPENDVER=Vn (ex. APPENDVER=V6) : hint de version SAS des métadonnées
//!   d'en-tête. Aucun effet sémantique sur la sortie. Accepté et ignoré.
//!
//! ## Contrat J02-P14 — fenêtre OBS=/FIRSTOBS=
//!
//! DATA= est la table LUE observation par observation : sa lecture passe par
//! `procs::common::open_resolved` et honore la fenêtre OBS=/FIRSTOBS= de la
//! session (doc SAS 9.4, System Options : valables pour toute lecture d'un
//! PROC). BASE= est ouverte en MISE À JOUR (append), pas lue en entrée :
//! `procs::common::open_resolved_full` lit la table entière (descripteur +
//! observations conservées) — l'y fenêtrer TRONQUERAIT BASE, ce que SAS ne
//! fait jamais.

use crate::ast::DatasetRef;
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::{self, decode_column};
use crate::session::Session;
use crate::value::{Value, VarType};
use polars::prelude::*;

pub struct AppendAst {
    pub base: DatasetRef,
    pub data: DatasetRef,
    pub force: bool,
    /// NOWARN : supprime le WARNING FORCE sur les différences structurelles.
    pub nowarn: bool,
    /// APPENDVER : hint de version (no-op), ex. "V6". None = non spécifié.
    pub appendver: Option<String>,
}

/// Parse `proc append base=<ref> data=<ref> [force] [nowarn] [appendver=Vn] ; run ;`.
/// Called AFTER `proc append` has been consumed. Consumes through
/// `run;` / `quit;`.
pub fn parse(ts: &mut StatementStream) -> Result<AppendAst> {
    use crate::token::TokenKind;

    let mut base: Option<DatasetRef> = None;
    let mut data: Option<DatasetRef> = None;
    let mut force = false;
    let mut nowarn = false;
    let mut appendver: Option<String> = None;

    // --- PROC APPEND statement options, until `;` (combinateur M31) ---
    common::parse_proc_options(ts, "APPEND", |ts, kw| {
        Ok(match kw {
            "base" => {
                base = Some(common::parse_dataset_opt(ts, "BASE")?);
                true
            }
            "data" => {
                data = Some(common::parse_dataset_opt(ts, "DATA")?);
                true
            }
            "force" => {
                ts.next();
                force = true;
                true
            }
            "nowarn" => {
                // NOWARN : supprime le WARNING de FORCE sur différences structurelles.
                ts.next();
                nowarn = true;
                true
            }
            "appendver" => {
                // APPENDVER=Vn : hint de version, aucun effet sémantique.
                ts.next(); // consume "appendver"
                if ts.peek().kind != TokenKind::Eq {
                    return Err(SasError::parse(
                        "expected '=' after APPENDVER".to_string(),
                        ts.peek().span,
                    ));
                }
                ts.next(); // consume '='
                let ver_tok = ts.peek().clone();
                let ver_name = ver_tok
                    .ident()
                    .ok_or_else(|| {
                        SasError::parse(
                            "expected a version string after APPENDVER=".to_string(),
                            ver_tok.span,
                        )
                    })?
                    .to_uppercase();
                ts.next();
                appendver = Some(ver_name);
                true
            }
            _ => false,
        })
    })?;

    let base = base.ok_or_else(|| {
        SasError::runtime("The BASE= option is required on the PROC APPEND statement.")
    })?;
    let data = data.ok_or_else(|| {
        SasError::runtime("The DATA= option is required on the PROC APPEND statement.")
    })?;

    // Consume through run;/quit; (sub-statements loop) (combinateur M31).
    common::parse_proc_body(ts, "APPEND", |_ts, _kw| Ok(false))?;

    Ok(AppendAst {
        base,
        data,
        force,
        nowarn,
        appendver,
    })
}

/// Truncate a string to at most `max_chars` Unicode characters.
fn truncate_to_length(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    // Contrat D-001 (docs/encoding.md) : les longueurs caractère comptent en
    // CARACTÈRES (convention session SAS LATIN1/WLATIN1), pas en octets — la
    // troncature par caractères est le comportement attendu, aligné sur la
    // troncature PDV et sur `VarMeta::length`.
    s.chars().take(max_chars).collect()
}

/// Execute PROC APPEND. Called by `procs::execute_proc`.
pub fn execute(ast: &AppendAst, session: &mut Session) -> Result<()> {
    let base_libref = ast.base.libref_or_work();
    let base_table = ast.base.name.to_uppercase();

    let base_disp = ast.base.display();
    let data_disp = ast.data.display();

    // Read the DATA dataset.
    // J02-P14 — DATA= est la table LUE : fenêtre OBS=/FIRSTOBS= via
    // `open_resolved` (doc SAS 9.4, System Options, OBS=/FIRSTOBS=).
    let data_ds = common::open_resolved(&ast.data, session)?;
    let n_data = data_ds.n_obs();

    // Check whether BASE exists.
    let base_provider = session.libs.get(&base_libref)?;
    let base_exists = base_provider.exists(&base_table);

    session
        .log
        .note(&format!("Appending {data_disp} to {base_disp}."));

    if !base_exists {
        // BASE does not exist: copy DATA to BASE verbatim.
        session
            .log
            .note("BASE data set does not exist. DATA file is being copied to BASE file.");
        base_provider.write(&base_table, &data_ds)?;
        session.last_dataset = Some(format!("{base_libref}.{base_table}"));
        let n = data_ds.n_obs();
        session.log.note(&format!(
            "The data set {base_disp} has {n} observations and {} variables.",
            data_ds.n_vars(),
        ));
        return Ok(());
    }

    // BASE exists: read it.
    // J02-P14 — BASE= est ouverte en MISE À JOUR (append), pas lue en
    // entrée : lecture ENTIÈRE via `open_resolved_full` (lecture de
    // métadonnées + conservation des observations ; l'y fenêtrer tronquerait
    // BASE — SAS n'applique pas OBS= à la table BASE d'un APPEND).
    let base_ds = common::open_resolved_full(&ast.base, session)?;
    let n_base = base_ds.n_obs();

    // Build lookup: uppercase name → (index, VarMeta) for BASE and DATA.
    let base_idx: std::collections::HashMap<String, usize> = base_ds
        .vars
        .iter()
        .enumerate()
        .map(|(i, v)| (v.name.to_uppercase(), i))
        .collect();
    let data_idx: std::collections::HashMap<String, usize> = data_ds
        .vars
        .iter()
        .enumerate()
        .map(|(i, v)| (v.name.to_uppercase(), i))
        .collect();

    // --- Anomaly checks (without FORCE) ---
    if !ast.force {
        let mut anomalies = false;
        for dv in &data_ds.vars {
            let key = dv.name.to_uppercase();
            match base_idx.get(&key) {
                None => {
                    // Variable in DATA not in BASE.
                    session
                        .log
                        .error(&format!("Variable {key} was not found on the BASE file."));
                    anomalies = true;
                }
                Some(&bi) => {
                    let bv = &base_ds.vars[bi];
                    if bv.ty != dv.ty {
                        session.log.error(&format!(
                            "Variable {key} has a different type in the DATA and BASE data sets."
                        ));
                        anomalies = true;
                    } else if bv.ty == VarType::Char && dv.length > bv.length {
                        session.log.error(&format!(
                            "Variable {key} has a length of {} in the DATA file but {} in the BASE file.",
                            dv.length, bv.length
                        ));
                        anomalies = true;
                    }
                }
            }
        }
        if anomalies {
            return Err(SasError::runtime(
                "No appending done because of anomalies in the DATA file. \
                 Use the FORCE option to force the append.",
            ));
        }
    }

    // --- With FORCE: warn about DATA variables not in BASE (they'll be dropped). ---
    // NOWARN suppresses these warnings when FORCE is active.
    if ast.force && !ast.nowarn {
        for dv in &data_ds.vars {
            let key = dv.name.to_uppercase();
            if !base_idx.contains_key(&key) {
                session.log.warning(&format!(
                    "Variable {key} was not found on BASE file. \
                     The variable will not be appended."
                ));
            }
        }
    }

    // Decode all DATA columns once.
    let data_cols: Vec<Vec<Value>> = (0..data_ds.vars.len())
        .map(|i| decode_column(&data_ds, i))
        .collect::<Result<_>>()?;

    // Decode all BASE columns once.
    let base_cols: Vec<Vec<Value>> = (0..base_ds.vars.len())
        .map(|i| decode_column(&base_ds, i))
        .collect::<Result<_>>()?;

    // Build result: one column per BASE variable, BASE rows then DATA rows.
    let mut new_columns: Vec<Column> = Vec::with_capacity(base_ds.vars.len());
    let new_vars: Vec<VarMeta> = base_ds.vars.clone();

    for (bi, bv) in base_ds.vars.iter().enumerate() {
        let key = bv.name.to_uppercase();
        // Get the DATA column if present (by name, case-insensitive).
        let data_col_opt: Option<&Vec<Value>> = data_idx.get(&key).map(|&di| &data_cols[di]);

        match bv.ty {
            VarType::Num => {
                // BASE rows first, then DATA rows.
                let mut vals: Vec<Option<f64>> = Vec::with_capacity(n_base + n_data);
                for v in &base_cols[bi] {
                    vals.push(value_to_num(v));
                }
                match data_col_opt {
                    Some(dc) => {
                        for v in dc {
                            vals.push(value_to_num(v));
                        }
                    }
                    None => {
                        // BASE-only variable: fill DATA rows with missing.
                        for _ in 0..n_data {
                            vals.push(None);
                        }
                    }
                }
                let ca =
                    Float64Chunked::from_iter_options(bv.name.as_str().into(), vals.into_iter());
                new_columns.push(ca.into_series().into());
            }
            VarType::Char => {
                let base_len = bv.length;
                let mut vals: Vec<Option<String>> = Vec::with_capacity(n_base + n_data);
                for v in &base_cols[bi] {
                    match v {
                        Value::Char(s) => vals.push(Some(s.clone())),
                        _ => vals.push(Some(String::new())),
                    }
                }
                match data_col_opt {
                    Some(dc) => {
                        for v in dc {
                            match v {
                                Value::Char(s) => {
                                    // Truncate to BASE length.
                                    let truncated = truncate_to_length(s, base_len);
                                    vals.push(Some(truncated));
                                }
                                _ => vals.push(Some(String::new())),
                            }
                        }
                    }
                    None => {
                        // BASE-only variable: fill DATA rows with blank (missing char).
                        for _ in 0..n_data {
                            vals.push(Some(String::new()));
                        }
                    }
                }
                let ca: StringChunked = vals
                    .into_iter()
                    .map(|o| o.as_deref().map(str::to_string))
                    .collect();
                let ca = ca.with_name(bv.name.as_str().into());
                new_columns.push(ca.into_series().into());
            }
        }
    }

    let new_df = DataFrame::new(new_columns)?;
    let result = SasDataset {
        df: new_df,
        vars: new_vars,
    };

    base_provider.write(&base_table, &result)?;
    session.last_dataset = Some(format!("{base_libref}.{base_table}"));

    let n_total = result.n_obs();
    let n_vars = result.n_vars();

    // NOTEs SAS au pluriel invariable (checklist #7) : toujours
    // "observations"/"variables", même pour 1.
    session.log.note(&format!(
        "There were {n_data} observations read from the data set {data_disp}."
    ));
    session.log.note(&format!("{n_data} observations added."));
    session.log.note(&format!(
        "The data set {base_disp} has {n_total} observations and {n_vars} variables."
    ));

    Ok(())
}

/// ── J02-P14 : fenêtre OBS=/FIRSTOBS= sur la lecture DATA= ────────────────
///
/// Oracle : doc SAS 9.4, System Options, OBS= / FIRSTOBS= — les options
/// système valent pour TOUTE lecture d'une table par un PROC :
/// <https://documentation.sas.com/doc/en/pgmsascdc/9.4_3.5/lesysoptsref/p0c2av6mvf2zsrn1b48go7vk6bro.htm>
///
/// Base (ancien silence) : PROC APPEND lisait DATA= par `provider.read`
/// direct — la table ENTIÈRE était lue, la fenêtre en vigueur ignorée
/// (8 observations ajoutées sous `obs=2`, 5 sous `firstobs=4`). La fenêtre
/// ne touche pas BASE= (ouverte en mise à jour) : sous `obs=2`, BASE garde
/// ses 3 observations d'origine puis reçoit les 2 PREMIÈRES de DATA.
#[cfg(test)]
mod contract_j02_p14 {
    use crate::RunOptions;
    use crate::dataset::SasDataset;
    use std::collections::BTreeMap;
    use std::path::Path;

    /// Exécute `program` dans un WORK isolé, puis relit toutes les tables
    /// WORK écrites (parquet) : (issue, tables par nom minuscule).
    fn run_and_read(program: &str) -> (crate::RunOutcome, BTreeMap<String, SasDataset>) {
        let tmp = tempfile::tempdir().unwrap();
        let work = tmp.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let out = crate::run(
            program,
            RunOptions {
                work_dir: Some(work.clone()),
                base_dir: Some(tmp.path().to_path_buf()),
                deterministic: true,
                vectorize: false,
            },
        );
        let tables = read_tables(&work);
        (out, tables)
    }

    fn read_tables(dir: &Path) -> BTreeMap<String, SasDataset> {
        let mut tables = BTreeMap::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let p = entry.unwrap().path();
            if p.extension().is_some_and(|e| e == "parquet") {
                let stem = p.file_stem().unwrap().to_str().unwrap().to_string();
                if let Ok((ds, _)) = SasDataset::read_parquet(&p) {
                    tables.insert(stem.to_lowercase(), ds);
                }
            }
        }
        tables
    }

    /// Valeurs de la première colonne numérique de `ds`.
    fn first_num_col(ds: &SasDataset) -> Vec<Option<f64>> {
        ds.df
            .get_columns()
            .first()
            .unwrap()
            .as_materialized_series()
            .f64()
            .unwrap()
            .into_iter()
            .collect()
    }

    /// `options obs=2;` avant l'APPEND : seules les 2 premières observations
    /// de DATA= sont lues et ajoutées ; BASE= garde ses observations
    /// d'origine (fenêtrer BASE tronquerait la table).
    #[test]
    fn ra_j02_p14_append_obs_window_data_input() {
        let (out, tables) = run_and_read(
            "data work.a; do i = 1 to 3; output; end; run;\n\
             data work.b; do i = 1 to 5; output; end; run;\n\
             options obs=2;\n\
             proc append base=work.a data=work.b; run;\n",
        );
        assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
        assert!(!out.log.contains("ERROR:"), "log:\n{}", out.log);
        // NOTEs SAS : 2 observations LUES (fenêtre), 2 ajoutées, 5 au total.
        assert!(
            out.log
                .contains("There were 2 observations read from the data set WORK.B."),
            "log:\n{}",
            out.log
        );
        assert!(
            out.log.contains("2 observations added."),
            "log:\n{}",
            out.log
        );
        assert!(
            out.log
                .contains("The data set WORK.A has 5 observations and 1 variables."),
            "log:\n{}",
            out.log
        );
        // Ancien silence : DATA= lue entière → 3 + 5 = 8 observations.
        let a = tables.get("a").expect("WORK.A must exist");
        assert_eq!(a.n_obs(), 5, "BASE must keep 3 obs + 2 windowed appends");
        // BASE jamais tronquée (1,2,3), puis les 2 PREMIÈRES de DATA (1,2).
        assert_eq!(
            first_num_col(a),
            vec![Some(1.0), Some(2.0), Some(3.0), Some(1.0), Some(2.0)],
            "BASE rows must be preserved, DATA windowed"
        );
    }

    /// `options firstobs=4;` : DATA= n'est lue qu'à partir de la 4e
    /// observation (fenêtre [4, ∞[). Ancien silence : 5 ajoutées.
    #[test]
    fn ra_j02_p14_append_firstobs_window_data_input() {
        let (out, tables) = run_and_read(
            "data work.a; do i = 1 to 3; output; end; run;\n\
             data work.b; do i = 1 to 5; output; end; run;\n\
             options firstobs=4;\n\
             proc append base=work.a data=work.b; run;\n",
        );
        assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
        let a = tables.get("a").expect("WORK.A must exist");
        // 3 obs de BASE + les observations 4 et 5 de DATA.
        assert_eq!(
            first_num_col(a),
            vec![Some(1.0), Some(2.0), Some(3.0), Some(4.0), Some(5.0)],
            "firstobs=4 window on DATA="
        );
    }

    /// BASE= inexistante : DATA= est copiée vers BASE — copie FENÊTRÉE
    /// (la copie est la lecture de DATA=). Ancien silence : 5 copiées.
    #[test]
    fn ra_j02_p14_append_obs_window_new_base_copy() {
        let (out, tables) = run_and_read(
            "data work.b; do i = 1 to 5; output; end; run;\n\
             options obs=2;\n\
             proc append base=work.new data=work.b; run;\n",
        );
        assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
        let new = tables.get("new").expect("WORK.NEW must be created");
        assert_eq!(
            first_num_col(new),
            vec![Some(1.0), Some(2.0)],
            "BASE created from windowed DATA read"
        );
        assert!(
            out.log
                .contains("The data set WORK.NEW has 2 observations and 1 variables."),
            "log:\n{}",
            out.log
        );
    }
}

#[cfg(test)]
mod tests;
