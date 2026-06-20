//! PROC FASTCLUS — k-means clustering (M27).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc fastclus data=<ref> maxclusters=k [out=<ref>] [maxiter=<n>]
//!  [converge=<f>] [seed=<n>]; var <list>; [id <var>;] run;`
//!
//! ## Périmètre
//! - `maxclusters=` (obligatoire), `out=`, `maxiter=` (défaut 10),
//!   `converge=` (défaut 0.02), `seed=` (parse-accepté ; non utilisé : la
//!   sélection des graines est déterministe "farthest-first").
//! - `var` : variables numériques (coordonnées). `id` : étiquette (parse).
//!
//! ## Algorithme
//! 1. Graines : sélection "farthest-first" — graine 1 = première obs, graine
//!    suivante = obs la plus éloignée des graines déjà choisies.
//! 2. Affecter chaque obs au centroïde le plus proche (euclidien).
//! 3. Recalculer les centroïdes.
//! 4. Répéter jusqu'à maxiter ou convergence (déplacement max < converge×RMS).

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::{self, decode_column};
use crate::procs::distance::{distance, DistMethod};
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::VarType;
use polars::prelude::*;

// ───────────────────────── AST ─────────────────────────

pub struct FastclusAst {
    pub data: Option<DatasetRef>,
    pub out: Option<DatasetRef>,
    pub maxclusters: usize,
    pub maxiter: usize,
    pub converge: f64,
    pub seed: Option<i64>,
    pub var: Vec<String>,
    pub id: Option<String>,
}

// ───────────────────────── Parser ─────────────────────────

fn expect_eq(ts: &mut StatementStream, opt: &str) -> Result<()> {
    if ts.peek().kind != TokenKind::Eq {
        return Err(SasError::parse(
            format!("expected '=' after {opt}"),
            ts.peek().span,
        ));
    }
    ts.next();
    Ok(())
}

fn parse_num(ts: &mut StatementStream, opt: &str) -> Result<f64> {
    let span = ts.peek().span;
    match ts.peek().kind {
        TokenKind::Num(v) => {
            ts.next();
            Ok(v)
        }
        _ => Err(SasError::parse(format!("expected a number after {opt}="), span)),
    }
}

/// Parse the PROC FASTCLUS block. Called AFTER "proc fastclus" consumed.
pub fn parse(ts: &mut StatementStream) -> Result<FastclusAst> {
    let mut data: Option<DatasetRef> = None;
    let mut out: Option<DatasetRef> = None;
    let mut maxclusters: Option<usize> = None;
    let mut maxiter: usize = 10;
    let mut converge: f64 = 0.02;
    let mut seed: Option<i64> = None;

    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("data") {
            data = Some(common::parse_dataset_opt(ts, "DATA")?);
        } else if ts.peek().is_kw("out") {
            out = Some(common::parse_out_opt(ts)?);
        } else if ts.peek().is_kw("maxclusters") || ts.peek().is_kw("maxc") {
            ts.next();
            expect_eq(ts, "MAXCLUSTERS")?;
            maxclusters = Some(parse_num(ts, "MAXCLUSTERS")? as usize);
        } else if ts.peek().is_kw("maxiter") {
            ts.next();
            expect_eq(ts, "MAXITER")?;
            maxiter = parse_num(ts, "MAXITER")? as usize;
        } else if ts.peek().is_kw("converge") {
            ts.next();
            expect_eq(ts, "CONVERGE")?;
            converge = parse_num(ts, "CONVERGE")?;
        } else if ts.peek().is_kw("seed") {
            ts.next();
            expect_eq(ts, "SEED")?;
            seed = Some(parse_num(ts, "SEED")? as i64);
        } else if let Some(name) = ts.peek().ident().map(str::to_string) {
            let span = ts.peek().span;
            return Err(SasError::parse(
                format!(
                    "Unexpected option '{}' on PROC FASTCLUS statement.",
                    name.to_uppercase()
                ),
                span,
            ));
        } else {
            let span = ts.peek().span;
            return Err(SasError::parse(
                "Unexpected token on PROC FASTCLUS statement.",
                span,
            ));
        }
    }

    let maxclusters = maxclusters.ok_or_else(|| {
        SasError::runtime("The MAXCLUSTERS= option is required for PROC FASTCLUS.")
    })?;

    let mut var: Vec<String> = Vec::new();
    let mut id: Option<String> = None;
    // Sous-statements jusqu'à `run;`/`quit;` (combinateur partagé M31).
    common::parse_proc_body(ts, |ts, kw| {
        Ok(match kw {
            "var" => {
                ts.next();
                var = ts.parse_name_list()?;
                ts.expect_semi()?;
                true
            }
            "id" => {
                ts.next();
                let names = ts.parse_name_list()?;
                id = names.into_iter().next();
                ts.expect_semi()?;
                true
            }
            _ => false,
        })
    })?;

    Ok(FastclusAst {
        data,
        out,
        maxclusters,
        maxiter,
        converge,
        seed,
        var,
        id,
    })
}

// ───────────────────────── k-means core ─────────────────────────

/// Result of running k-means.
pub struct KMeansResult {
    /// Cluster assignment (1..=k) per observation.
    pub assign: Vec<usize>,
    /// Final centroids.
    pub centroids: Vec<Vec<f64>>,
}

/// Pick k initial seeds by farthest-first: seed 1 = first observation, each
/// subsequent seed = observation maximizing its minimum distance to the
/// already-chosen seeds. Returns observation indices.
fn farthest_first_seeds(coords: &[Vec<f64>], k: usize) -> Vec<usize> {
    let n = coords.len();
    let mut seeds = vec![0usize];
    while seeds.len() < k && seeds.len() < n {
        let mut best_idx = 0usize;
        let mut best_d = -1.0_f64;
        for i in 0..n {
            if seeds.contains(&i) {
                continue;
            }
            let mind = seeds
                .iter()
                .map(|&s| distance(DistMethod::Euclid, &coords[i], &coords[s]))
                .fold(f64::INFINITY, f64::min);
            if mind > best_d {
                best_d = mind;
                best_idx = i;
            }
        }
        seeds.push(best_idx);
    }
    seeds
}

/// Run k-means with farthest-first seeding.
pub fn kmeans(coords: &[Vec<f64>], k: usize, maxiter: usize, converge: f64) -> KMeansResult {
    let n = coords.len();
    let p = if n > 0 { coords[0].len() } else { 0 };
    let k = k.min(n).max(1);

    let seeds = farthest_first_seeds(coords, k);
    let mut centroids: Vec<Vec<f64>> = seeds.iter().map(|&s| coords[s].clone()).collect();
    let mut assign = vec![0usize; n];

    // Overall RMS std scale for the convergence threshold.
    let rms_scale = overall_rms_std(coords).max(1e-12);

    let iters = maxiter.max(1);
    for _ in 0..iters {
        // Assign.
        for i in 0..n {
            let mut best = 0usize;
            let mut bestd = f64::INFINITY;
            for (c, cent) in centroids.iter().enumerate() {
                let d = distance(DistMethod::Euclid, &coords[i], cent);
                if d < bestd {
                    bestd = d;
                    best = c;
                }
            }
            assign[i] = best;
        }

        // Recompute centroids.
        let mut sums = vec![vec![0.0_f64; p]; k];
        let mut counts = vec![0usize; k];
        for i in 0..n {
            let c = assign[i];
            counts[c] += 1;
            for j in 0..p {
                sums[c][j] += coords[i][j];
            }
        }
        let mut max_move = 0.0_f64;
        for c in 0..k {
            if counts[c] == 0 {
                continue;
            }
            let mut newc = vec![0.0_f64; p];
            for j in 0..p {
                newc[j] = sums[c][j] / counts[c] as f64;
            }
            let move_d = distance(DistMethod::Euclid, &newc, &centroids[c]);
            max_move = max_move.max(move_d);
            centroids[c] = newc;
        }

        if max_move < converge * rms_scale {
            // Re-assign once more against the updated centroids to settle.
            for i in 0..n {
                let mut best = 0usize;
                let mut bestd = f64::INFINITY;
                for (c, cent) in centroids.iter().enumerate() {
                    let d = distance(DistMethod::Euclid, &coords[i], cent);
                    if d < bestd {
                        bestd = d;
                        best = c;
                    }
                }
                assign[i] = best;
            }
            break;
        }
    }

    KMeansResult {
        assign: assign.iter().map(|&c| c + 1).collect(),
        centroids,
    }
}

/// Overall pooled RMS std across all variables (n-1 per variable, averaged).
fn overall_rms_std(coords: &[Vec<f64>]) -> f64 {
    let n = coords.len();
    if n < 2 {
        return 0.0;
    }
    let p = coords[0].len();
    let mut ss_sum = 0.0_f64;
    for j in 0..p {
        let mean = coords.iter().map(|r| r[j]).sum::<f64>() / n as f64;
        let ss: f64 = coords.iter().map(|r| (r[j] - mean).powi(2)).sum();
        ss_sum += ss;
    }
    (ss_sum / ((n - 1) as f64 * p as f64)).sqrt()
}

// ───────────────────────── execute ─────────────────────────

pub fn execute(ast: &FastclusAst, session: &mut Session) -> Result<()> {
    if ast.var.is_empty() {
        return Err(SasError::runtime("PROC FASTCLUS requires a VAR statement."));
    }

    let in_ref = common::resolve_last_dataset(&ast.data, session)?;
    let in_libref = in_ref.libref_or_work();
    let in_table = in_ref.name.to_uppercase();
    let display = format!("{in_libref}.{in_table}");

    let provider = session.libs.get(&in_libref)?;
    let (ds, notes) = provider.read(&in_table)?;
    for note in notes {
        session.log.forward(&note);
    }
    let n_read = ds.n_obs();
    session.log.note(&format!(
        "There were {} observations read from the data set {}.",
        n_read, display
    ));

    let mut cols: Vec<usize> = Vec::with_capacity(ast.var.len());
    for nm in &ast.var {
        match ds.vars.iter().position(|m| m.name.eq_ignore_ascii_case(nm)) {
            Some(i) if ds.vars[i].ty == VarType::Num => cols.push(i),
            _ => {
                return Err(SasError::runtime(format!(
                    "Variable '{}' not found in dataset '{}'.",
                    nm, display
                )))
            }
        }
    }
    let p = cols.len();
    let names: Vec<String> = cols.iter().map(|&c| ds.vars[c].name.clone()).collect();

    let decoded: Vec<Vec<f64>> = cols
        .iter()
        .map(|&c| {
            decode_column(&ds, c).map(|vals| {
                vals.iter()
                    .map(|v| value_to_num(v).unwrap_or(f64::NAN))
                    .collect::<Vec<f64>>()
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let coords: Vec<Vec<f64>> = (0..n_read)
        .map(|r| decoded.iter().map(|col| col[r]).collect())
        .collect();
    let n = coords.len();
    let k = ast.maxclusters.min(n).max(1);

    if let Some(s) = ast.seed {
        session.log.note(&format!(
            "PROC FASTCLUS SEED={} is accepted; seeds are selected by the deterministic farthest-first rule.",
            s
        ));
    }

    let res = kmeans(&coords, k, ast.maxiter, ast.converge);

    // Per-cluster stats.
    let mut freq = vec![0usize; k];
    let mut max_seed_dist = vec![0.0_f64; k];
    for i in 0..n {
        let c = res.assign[i] - 1;
        freq[c] += 1;
        let d = distance(DistMethod::Euclid, &coords[i], &res.centroids[c]);
        max_seed_dist[c] = max_seed_dist[c].max(d);
    }

    // Nearest cluster (by centroid distance) + that distance.
    let mut nearest = vec![0usize; k];
    let mut nearest_dist = vec![0.0_f64; k];
    for c in 0..k {
        let mut best = c;
        let mut bestd = f64::INFINITY;
        for o in 0..k {
            if o == c {
                continue;
            }
            let d = distance(DistMethod::Euclid, &res.centroids[c], &res.centroids[o]);
            if d < bestd {
                bestd = d;
                best = o;
            }
        }
        nearest[c] = best;
        nearest_dist[c] = if bestd.is_finite() { bestd } else { 0.0 };
    }

    // Per-cluster RMS std (pooled across variables, denominator n).
    let cluster_rms: Vec<f64> = (0..k)
        .map(|c| cluster_rms_std(&coords, &res.assign, c + 1, &res.centroids[c]))
        .collect();

    // R-Square per variable and overall: 1 - within-SS / total-SS.
    let (var_rsq, overall_rsq, total_std, within_std) =
        variable_rsquare(&coords, &res.assign, &res.centroids, p);

    // ───────────────────────── listing ─────────────────────────
    session.listing.page_header();
    centered(session, "The FASTCLUS Procedure");
    session.listing.blank();
    centered(
        session,
        &format!(
            "Replace=FULL   Radius=0   Maxclusters={}   Maxiter={}",
            k, ast.maxiter
        ),
    );
    session.listing.blank();
    centered(session, "Cluster Summary");
    session.listing.blank();
    {
        let headers: Vec<String> = vec![
            "Cluster".into(),
            "Frequency".into(),
            "RMS Std".into(),
            "Maximum Distance from Seed to Obs".into(),
            "Nearest Cluster".into(),
            "Distance Between Cluster Centroids".into(),
        ];
        let aligns = vec![
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
        ];
        let rows: Vec<Vec<String>> = (0..k)
            .map(|c| {
                vec![
                    (c + 1).to_string(),
                    freq[c].to_string(),
                    format!("{:.4}", cluster_rms[c]),
                    format!("{:.4}", max_seed_dist[c]),
                    (nearest[c] + 1).to_string(),
                    format!("{:.4}", nearest_dist[c]),
                ]
            })
            .collect();
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }

    centered(session, "Statistics for Variables");
    session.listing.blank();
    {
        let headers: Vec<String> = vec![
            "Variable".into(),
            "Total STD".into(),
            "Within STD".into(),
            "R-Square".into(),
            "RSQ/(1-RSQ)".into(),
        ];
        let aligns = vec![
            Align::Left,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
        ];
        let mut rows: Vec<Vec<String>> = Vec::with_capacity(p + 1);
        for j in 0..p {
            let rsq = var_rsq[j];
            rows.push(vec![
                names[j].clone(),
                format!("{:.4}", total_std[j]),
                format!("{:.4}", within_std[j]),
                format!("{:.4}", rsq),
                format!("{:.4}", ratio(rsq)),
            ]);
        }
        // Overall row.
        let tot_all = (total_std.iter().map(|s| s * s).sum::<f64>() / p as f64).sqrt();
        let wit_all = (within_std.iter().map(|s| s * s).sum::<f64>() / p as f64).sqrt();
        rows.push(vec![
            "OVER-ALL".into(),
            format!("{:.4}", tot_all),
            format!("{:.4}", wit_all),
            format!("{:.4}", overall_rsq),
            format!("{:.4}", ratio(overall_rsq)),
        ]);
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }

    // ───────────────────────── out= ─────────────────────────
    if let Some(out_ref) = &ast.out {
        // Copy input columns and append _CLUSTER_.
        let mut out_ds_df = ds.df.clone();
        let cluster_col: Vec<f64> = res.assign.iter().map(|&c| c as f64).collect();
        out_ds_df
            .with_column(Series::new("_CLUSTER_".into(), cluster_col))
            .map_err(|e| SasError::runtime(format!("FASTCLUS OUT= build failed: {e}")))?;
        let mut vars = ds.vars.clone();
        vars.push(crate::dataset::VarMeta {
            name: "_CLUSTER_".into(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        });
        let out_ds = SasDataset {
            df: out_ds_df,
            vars,
        };
        let out_libref = out_ref.libref_or_work();
        let out_table = out_ref.name.to_uppercase();
        let out_display = format!("{out_libref}.{out_table}");
        let n_rows = out_ds.n_obs();
        let n_vars = out_ds.vars.len();
        session.libs.get(&out_libref)?.write(&out_table, &out_ds)?;
        session.last_dataset = Some(out_display.clone());
        session.log.note(&format!(
            "The data set {} has {} observations and {} variables.",
            out_display, n_rows, n_vars
        ));
    }

    let _ = ast.id; // ID statement parse-accepted; not used in the listing v1.

    Ok(())
}

fn ratio(rsq: f64) -> f64 {
    if (1.0 - rsq).abs() < 1e-12 {
        f64::INFINITY
    } else {
        rsq / (1.0 - rsq)
    }
}

/// Pooled RMS std within one cluster (denominator = freq, across all vars).
fn cluster_rms_std(coords: &[Vec<f64>], assign: &[usize], cluster: usize, centroid: &[f64]) -> f64 {
    let p = centroid.len();
    let mut ss = 0.0_f64;
    let mut cnt = 0usize;
    for (i, &c) in assign.iter().enumerate() {
        if c == cluster {
            cnt += 1;
            for j in 0..p {
                ss += (coords[i][j] - centroid[j]).powi(2);
            }
        }
    }
    if cnt == 0 {
        0.0
    } else {
        (ss / (cnt * p) as f64).sqrt()
    }
}

/// Per-variable Total STD (n-1), Within STD (pooled), R-Square, and overall.
fn variable_rsquare(
    coords: &[Vec<f64>],
    assign: &[usize],
    centroids: &[Vec<f64>],
    p: usize,
) -> (Vec<f64>, f64, Vec<f64>, Vec<f64>) {
    let n = coords.len();
    let mut total_ss = vec![0.0_f64; p];
    let mut within_ss = vec![0.0_f64; p];
    for j in 0..p {
        let mean = coords.iter().map(|r| r[j]).sum::<f64>() / n as f64;
        for (i, &c) in assign.iter().enumerate() {
            total_ss[j] += (coords[i][j] - mean).powi(2);
            within_ss[j] += (coords[i][j] - centroids[c - 1][j]).powi(2);
        }
    }
    let total_std: Vec<f64> = total_ss
        .iter()
        .map(|s| if n > 1 { (s / (n - 1) as f64).sqrt() } else { 0.0 })
        .collect();
    let k = centroids.len();
    let within_df = (n.saturating_sub(k)).max(1) as f64;
    let within_std: Vec<f64> = within_ss.iter().map(|s| (s / within_df).sqrt()).collect();
    let var_rsq: Vec<f64> = (0..p)
        .map(|j| {
            if total_ss[j] > 0.0 {
                1.0 - within_ss[j] / total_ss[j]
            } else {
                0.0
            }
        })
        .collect();
    let tot_sum: f64 = total_ss.iter().sum();
    let wit_sum: f64 = within_ss.iter().sum();
    let overall = if tot_sum > 0.0 {
        1.0 - wit_sum / tot_sum
    } else {
        0.0
    };
    (var_rsq, overall, total_std, within_std)
}

use crate::dataset::SasDataset;

fn centered(session: &mut Session, text: &str) {
    let ls = session.listing.ls();
    let pad = ls.saturating_sub(text.len()) / 2;
    session
        .listing
        .write_line(&format!("{}{}", " ".repeat(pad), text));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::{SasDataset, VarMeta};
    use crate::session::Session;
    use crate::source::SourceFile;
    use crate::value::VarType;
    use polars::df;
    use std::path::PathBuf;

    fn make_session() -> Session {
        Session::new(None, PathBuf::from("."), true).unwrap()
    }

    fn num_meta(name: &str) -> VarMeta {
        VarMeta {
            name: name.to_string(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        }
    }

    fn write_dataset(session: &mut Session, table: &str, ds: SasDataset) {
        session.libs.get("WORK").unwrap().write(table, &ds).unwrap();
        session.last_dataset = Some(format!("WORK.{}", table.to_uppercase()));
    }

    fn parse_fastclus(src: &str) -> Result<FastclusAst> {
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next();
        ts.next();
        parse(&mut ts)
    }

    #[test]
    fn parse_minimal() {
        let ast = parse_fastclus("proc fastclus data=a maxclusters=2 out=b maxiter=20; var x; run;")
            .unwrap();
        assert_eq!(ast.maxclusters, 2);
        assert_eq!(ast.maxiter, 20);
        assert_eq!(ast.out.as_ref().unwrap().name, "b");
        assert_eq!(ast.var, vec!["x"]);
    }

    #[test]
    fn parse_requires_maxclusters() {
        let r = parse_fastclus("proc fastclus data=a; var x; run;");
        assert!(r.is_err());
        assert!(r.err().unwrap().to_string().contains("MAXCLUSTERS"));
    }

    /// Seeding test: farthest-first on {1,2,3,7,8,9} picks obs0 (x=1) and the
    /// farthest, obs5 (x=9).
    #[test]
    fn farthest_first_picks_extremes() {
        let coords: Vec<Vec<f64>> = [1.0, 2.0, 3.0, 7.0, 8.0, 9.0]
            .iter()
            .map(|&x| vec![x])
            .collect();
        let seeds = farthest_first_seeds(&coords, 2);
        assert_eq!(seeds, vec![0, 5]);
    }

    /// k=2 on {1,2,3,7,8,9} → centroids exactly {2.0, 8.0}.
    #[test]
    fn kmeans_oracle_centroids() {
        let coords: Vec<Vec<f64>> = [1.0, 2.0, 3.0, 7.0, 8.0, 9.0]
            .iter()
            .map(|&x| vec![x])
            .collect();
        let res = kmeans(&coords, 2, 20, 0.02);
        let mut cents: Vec<f64> = res.centroids.iter().map(|c| c[0]).collect();
        cents.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert!((cents[0] - 2.0).abs() < 1e-12, "c0={}", cents[0]);
        assert!((cents[1] - 8.0).abs() < 1e-12, "c1={}", cents[1]);
        // Group membership: {1,2,3} together, {7,8,9} together.
        assert_eq!(res.assign[0], res.assign[1]);
        assert_eq!(res.assign[1], res.assign[2]);
        assert_eq!(res.assign[3], res.assign[4]);
        assert_eq!(res.assign[4], res.assign[5]);
        assert_ne!(res.assign[0], res.assign[5]);
    }

    #[test]
    fn execute_writes_out_with_cluster() {
        let mut session = make_session();
        let df = df!["x" => [1.0_f64, 2.0, 3.0, 7.0, 8.0, 9.0]].unwrap();
        let ds = SasDataset {
            df,
            vars: vec![num_meta("x")],
        };
        write_dataset(&mut session, "PTS", ds);
        let ast = FastclusAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "PTS".into() }),
            out: Some(DatasetRef { libref: Some("WORK".into()), name: "CL".into() }),
            maxclusters: 2,
            maxiter: 20,
            converge: 0.02,
            seed: None,
            var: vec!["x".into()],
            id: None,
        };
        execute(&ast, &mut session).unwrap();
        let (out, _) = session.libs.get("WORK").unwrap().read("CL").unwrap();
        assert!(out.vars.iter().any(|v| v.name == "_CLUSTER_"));
        let cl = out.df.column("_CLUSTER_").unwrap().f64().unwrap();
        // first three same cluster, last three the other.
        assert_eq!(cl.get(0), cl.get(1));
        assert_eq!(cl.get(1), cl.get(2));
        assert_ne!(cl.get(0), cl.get(5));
    }
}
