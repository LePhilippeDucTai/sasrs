//! PROC CLUSTER — agglomerative hierarchical clustering (M27).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc cluster data=<ref> [method=ward|average|single|complete]
//!  [outtree=<ref>] [print=<n>] [noeigen]; var <list>; [id <var>;] run;`
//!
//! ## Périmètre
//! - `data=`, `method=` (défaut WARD), `outtree=` (parse-accepté, NOTE),
//!   `print=` (parse-accepté), `noeigen` (parse-accepté, ignoré).
//! - `var` : variables numériques (coordonnées). `id <var>` : étiquette (parse).
//! - Sortie : "Cluster History" (NCl, Clusters Joined, Freq, SPRSQ, RSQ).
//! - Différé : section Eigenvalues, outtree data set.
//!
//! ## Algorithme
//! - n clusters singletons ; matrice de dissimilarité euclidienne initiale.
//! - À chaque étape : paire (i,j) minimisant le critère de la méthode
//!   (Ward = (ni*nj)/(ni+nj) * d², single/complete/average sur les distances).
//! - Mise à jour Lance-Williams. Tie-break : indices (i<j) croissants, on ne
//!   remplace le meilleur que sur STRICTEMENT inférieur (plus petits indices).
//!
//! ## SPRSQ / RSQ (TOUJOURS basés sur la somme des carrés intra, indépendant
//! de la méthode de liaison) :
//! - SS_total = Σ sur toutes les obs et variables des carrés des écarts à la
//!   moyenne globale.
//! - À chaque fusion, ΔSS = (ni*nj)/(ni+nj) * d²(centroïde_i, centroïde_j)
//!   (formule de Ward = augmentation exacte de la SS intra).
//! - SPRSQ = ΔSS / SS_total ; RSQ = 1 - (SS_intra_cumulée / SS_total).
//! - Nommage : un cluster formé à la ligne où il reste NCl clusters → "CL<NCl>".

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::{self, decode_column};
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::VarType;

// ───────────────────────── AST ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkMethod {
    Ward,
    Average,
    Single,
    Complete,
}

impl LinkMethod {
    fn title(self) -> &'static str {
        match self {
            LinkMethod::Ward => "Ward's Minimum Variance Cluster Analysis",
            LinkMethod::Average => "Average Linkage Cluster Analysis",
            LinkMethod::Single => "Single Linkage Cluster Analysis",
            LinkMethod::Complete => "Complete Linkage Cluster Analysis",
        }
    }
}

pub struct ClusterAst {
    pub data: Option<DatasetRef>,
    pub method: LinkMethod,
    pub outtree: Option<DatasetRef>,
    pub print: Option<usize>,
    pub noeigen: bool,
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

fn parse_method(ts: &mut StatementStream) -> Result<LinkMethod> {
    let span = ts.peek().span;
    let name = ts
        .peek()
        .ident()
        .map(str::to_string)
        .ok_or_else(|| SasError::parse("expected a method name after METHOD=", span))?;
    ts.next();
    match name.to_ascii_lowercase().as_str() {
        "ward" => Ok(LinkMethod::Ward),
        "average" | "ave" => Ok(LinkMethod::Average),
        "single" => Ok(LinkMethod::Single),
        "complete" | "com" => Ok(LinkMethod::Complete),
        other => Err(SasError::parse(
            format!("Unknown METHOD= value '{}' on PROC CLUSTER.", other.to_uppercase()),
            span,
        )),
    }
}

/// Parse the PROC CLUSTER block. Called AFTER "proc cluster" has been consumed.
pub fn parse(ts: &mut StatementStream) -> Result<ClusterAst> {
    let mut data: Option<DatasetRef> = None;
    let mut method = LinkMethod::Ward;
    let mut outtree: Option<DatasetRef> = None;
    let mut print: Option<usize> = None;
    let mut noeigen = false;

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
        } else if ts.peek().is_kw("method") {
            ts.next();
            expect_eq(ts, "METHOD")?;
            method = parse_method(ts)?;
        } else if ts.peek().is_kw("outtree") {
            outtree = Some(common::parse_dataset_opt(ts, "OUTTREE")?);
        } else if ts.peek().is_kw("print") {
            ts.next();
            expect_eq(ts, "PRINT")?;
            let span = ts.peek().span;
            let k = match ts.peek().kind {
                TokenKind::Num(v) => v,
                _ => return Err(SasError::parse("expected a number after PRINT=", span)),
            };
            ts.next();
            print = Some(k as usize);
        } else if ts.peek().is_kw("noeigen") {
            ts.next();
            noeigen = true;
        } else if let Some(name) = ts.peek().ident().map(str::to_string) {
            let span = ts.peek().span;
            return Err(SasError::parse(
                format!(
                    "Unexpected option '{}' on PROC CLUSTER statement.",
                    name.to_uppercase()
                ),
                span,
            ));
        } else {
            let span = ts.peek().span;
            return Err(SasError::parse(
                "Unexpected token on PROC CLUSTER statement.",
                span,
            ));
        }
    }

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

    Ok(ClusterAst {
        data,
        method,
        outtree,
        print,
        noeigen,
        var,
        id,
    })
}

// ───────────────────────── clustering core ─────────────────────────

/// One recorded merge step.
#[derive(Debug, Clone)]
pub struct MergeStep {
    pub ncl: usize,
    pub joined_a: String,
    pub joined_b: String,
    pub freq: usize,
    pub sprsq: f64,
    pub rsq: f64,
}

/// An active cluster during agglomeration.
struct ClusterNode {
    members: Vec<usize>,
    centroid: Vec<f64>,
    /// Display label: "OB<i>" for a singleton, "CL<ncl>" for a composite.
    label: String,
}

/// Run agglomerative clustering on `coords` (one vector per observation),
/// returning the merge history (NCl from n-1 down to 1).
///
/// `labels` provides the singleton display labels (e.g. ID values or "OB1").
pub fn agglomerate(
    coords: &[Vec<f64>],
    method: LinkMethod,
    labels: &[String],
) -> Vec<MergeStep> {
    let n = coords.len();
    let p = if n > 0 { coords[0].len() } else { 0 };

    // Total sum of squared deviations from the global mean (all vars).
    let mut gmean = vec![0.0_f64; p];
    for row in coords {
        for j in 0..p {
            gmean[j] += row[j];
        }
    }
    for m in &mut gmean {
        *m /= n as f64;
    }
    let mut ss_total = 0.0_f64;
    for row in coords {
        for j in 0..p {
            let d = row[j] - gmean[j];
            ss_total += d * d;
        }
    }

    // Initialize clusters (singletons).
    let mut clusters: Vec<Option<ClusterNode>> = coords
        .iter()
        .enumerate()
        .map(|(i, c)| {
            Some(ClusterNode {
                members: vec![i],
                centroid: c.clone(),
                label: labels[i].clone(),
            })
        })
        .collect();

    // Pairwise dissimilarities between active clusters. dmat[i][j] is the
    // linkage-criterion value (for Ward = the merge cost = ΔSS).
    let mut dmat = vec![vec![0.0_f64; n]; n];
    for i in 0..n {
        for j in (i + 1)..n {
            let v = pair_criterion(
                method,
                clusters[i].as_ref().unwrap(),
                clusters[j].as_ref().unwrap(),
                coords,
            );
            dmat[i][j] = v;
            dmat[j][i] = v;
        }
    }

    let mut active: Vec<usize> = (0..n).collect();
    let mut history: Vec<MergeStep> = Vec::new();
    let mut ss_within = 0.0_f64;
    let denom = if ss_total != 0.0 { ss_total } else { 1.0 };

    // n-1 merges. After each merge, NCl = number of remaining clusters.
    for step in 0..n.saturating_sub(1) {
        // Find the closest pair, scanning ascending (i<j); strict-less replace.
        let mut best: Option<(usize, usize, f64)> = None;
        for ai in 0..active.len() {
            for bj in (ai + 1)..active.len() {
                let i = active[ai];
                let j = active[bj];
                let (lo, hi) = if i < j { (i, j) } else { (j, i) };
                let v = dmat[lo][hi];
                match best {
                    None => best = Some((lo, hi, v)),
                    Some((_, _, bv)) if v < bv => best = Some((lo, hi, v)),
                    _ => {}
                }
            }
        }
        let (i, j, _crit) = best.expect("at least one pair while active>1");

        // ΔSS from this merge (Ward formula = exact within-SS increase).
        let ci = clusters[i].as_ref().unwrap();
        let cj = clusters[j].as_ref().unwrap();
        let ni = ci.members.len() as f64;
        let nj = cj.members.len() as f64;
        let d2 = squared_centroid_distance(&ci.centroid, &cj.centroid);
        let delta_ss = (ni * nj) / (ni + nj) * d2;
        ss_within += delta_ss;

        let ncl_after = active.len() - 1;

        // New merged cluster.
        let mut members = ci.members.clone();
        members.extend_from_slice(&cj.members);
        let new_n = members.len() as f64;
        let mut centroid = vec![0.0_f64; p];
        for k in 0..p {
            centroid[k] = (ni * ci.centroid[k] + nj * cj.centroid[k]) / new_n;
        }
        let label = if ncl_after == 0 {
            "CL1".to_string()
        } else {
            format!("CL{}", ncl_after)
        };

        let joined_a = ci.label.clone();
        let joined_b = cj.label.clone();

        history.push(MergeStep {
            ncl: ncl_after,
            joined_a,
            joined_b,
            freq: members.len(),
            sprsq: delta_ss / denom,
            rsq: 1.0 - ss_within / denom,
        });

        // Merge j into i; remove j.
        clusters[i] = Some(ClusterNode {
            members,
            centroid,
            label,
        });
        clusters[j] = None;
        active.retain(|&x| x != j);

        // Recompute distances from the new cluster i to all other active.
        for &k in &active {
            if k == i {
                continue;
            }
            let v = pair_criterion(
                method,
                clusters[i].as_ref().unwrap(),
                clusters[k].as_ref().unwrap(),
                coords,
            );
            let (lo, hi) = if i < k { (i, k) } else { (k, i) };
            dmat[lo][hi] = v;
            dmat[hi][lo] = v;
        }
        let _ = step;
    }

    history
}

/// The merge criterion between two clusters for the given linkage method.
///
/// Ward uses the centroid-based ΔSS. Single/Complete/Average are computed
/// exactly from the raw inter-observation Euclidean distances (this is the
/// definition; equivalent to the Lance-Williams recurrences).
fn pair_criterion(method: LinkMethod, a: &ClusterNode, b: &ClusterNode, coords: &[Vec<f64>]) -> f64 {
    match method {
        LinkMethod::Ward => {
            let na = a.members.len() as f64;
            let nb = b.members.len() as f64;
            (na * nb) / (na + nb) * squared_centroid_distance(&a.centroid, &b.centroid)
        }
        LinkMethod::Single | LinkMethod::Complete | LinkMethod::Average => {
            let mut acc = match method {
                LinkMethod::Single => f64::INFINITY,
                LinkMethod::Complete => f64::NEG_INFINITY,
                _ => 0.0,
            };
            for &ia in &a.members {
                for &ib in &b.members {
                    let d = euclid(&coords[ia], &coords[ib]);
                    match method {
                        LinkMethod::Single => acc = acc.min(d),
                        LinkMethod::Complete => acc = acc.max(d),
                        _ => acc += d,
                    }
                }
            }
            if method == LinkMethod::Average {
                acc /= (a.members.len() * b.members.len()) as f64;
            }
            acc
        }
    }
}

fn squared_centroid_distance(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum()
}

fn euclid(a: &[f64], b: &[f64]) -> f64 {
    squared_centroid_distance(a, b).sqrt()
}

// ───────────────────────── execute ─────────────────────────

pub fn execute(ast: &ClusterAst, session: &mut Session) -> Result<()> {
    if ast.var.is_empty() {
        return Err(SasError::runtime("PROC CLUSTER requires a VAR statement."));
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

    // Resolve VAR columns (numeric only).
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
    if n < 2 {
        return Err(SasError::runtime(
            "PROC CLUSTER requires at least 2 observations.",
        ));
    }

    // Singleton labels: ID value if an ID variable is present, else OB<i>.
    let labels: Vec<String> = match &ast.id {
        Some(idname) => {
            let idx = ds
                .vars
                .iter()
                .position(|m| m.name.eq_ignore_ascii_case(idname))
                .ok_or_else(|| {
                    SasError::runtime(format!(
                        "Variable '{}' not found in dataset '{}'.",
                        idname, display
                    ))
                })?;
            let vals = decode_column(&ds, idx)?;
            vals.iter().map(label_of_value).collect()
        }
        None => (0..n).map(|i| format!("OB{}", i + 1)).collect(),
    };

    let history = agglomerate(&coords, ast.method, &labels);

    // ───────────────────────── listing ─────────────────────────
    session.listing.page_header();
    centered(session, "The CLUSTER Procedure");
    centered(session, ast.method.title());
    session.listing.blank();
    centered(session, "Cluster History");
    session.listing.blank();
    {
        let headers: Vec<String> = vec![
            "NCl".into(),
            "Clusters Joined".into(),
            String::new(),
            "Freq".into(),
            "SPRSQ".into(),
            "RSQ".into(),
        ];
        let aligns = vec![
            Align::Right,
            Align::Right,
            Align::Left,
            Align::Right,
            Align::Right,
            Align::Right,
        ];
        let rows: Vec<Vec<String>> = history
            .iter()
            .map(|s| {
                vec![
                    s.ncl.to_string(),
                    s.joined_a.clone(),
                    s.joined_b.clone(),
                    s.freq.to_string(),
                    format!("{:.4}", s.sprsq),
                    format!("{:.4}", s.rsq.max(0.0)),
                ]
            })
            .collect();
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }

    if ast.noeigen {
        // Eigenvalues section is deferred; NOEIGEN simply confirms we skip it.
        session
            .log
            .note("PROC CLUSTER NOEIGEN: eigenvalue section is not produced.");
    }
    if let Some(k) = ast.print {
        session.log.note(&format!(
            "PROC CLUSTER PRINT={} is accepted; full history is shown.",
            k
        ));
    }
    if ast.outtree.is_some() {
        session.log.note(
            "PROC CLUSTER OUTTREE= is not yet implemented; the output data set was not created.",
        );
    }

    Ok(())
}

fn label_of_value(v: &crate::value::Value) -> String {
    use crate::value::{format_best, Value};
    match v {
        Value::Char(s) => s.trim().to_string(),
        Value::Num(f) => format_best(*f, 12).trim().to_string(),
        Value::Missing(k) => k.display(),
    }
}

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

    fn parse_cluster(src: &str) -> Result<ClusterAst> {
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next();
        ts.next();
        parse(&mut ts)
    }

    #[test]
    fn parse_minimal() {
        let ast = parse_cluster("proc cluster data=a method=ward; var x; id name; run;").unwrap();
        assert_eq!(ast.method, LinkMethod::Ward);
        assert_eq!(ast.var, vec!["x"]);
        assert_eq!(ast.id.as_deref(), Some("name"));
    }

    #[test]
    fn parse_options() {
        let ast = parse_cluster(
            "proc cluster data=a method=average outtree=t print=15 noeigen; var x y; run;",
        )
        .unwrap();
        assert_eq!(ast.method, LinkMethod::Average);
        assert_eq!(ast.outtree.as_ref().unwrap().name, "t");
        assert_eq!(ast.print, Some(15));
        assert!(ast.noeigen);
    }

    /// Ward oracle on x=(1,2,3,7,8,9). SS_total=58.
    /// Within-SS series: 0.5, 1.0, 2.5, 4.0, 58 →
    /// RSQ: 0.9914, 0.9828, 0.9569, 0.9310, 0.0000
    /// SPRSQ: 0.0086, 0.0086, 0.0259, 0.0259, 0.9310
    #[test]
    fn ward_oracle_six_points() {
        let coords: Vec<Vec<f64>> = [1.0, 2.0, 3.0, 7.0, 8.0, 9.0]
            .iter()
            .map(|&x| vec![x])
            .collect();
        let labels: Vec<String> = (0..6).map(|i| format!("OB{}", i + 1)).collect();
        let h = agglomerate(&coords, LinkMethod::Ward, &labels);
        assert_eq!(h.len(), 5);

        let ss = [0.5, 1.0, 2.5, 4.0, 58.0];
        let expect_rsq = [0.9914, 0.9828, 0.9569, 0.9310, 0.0000];
        let expect_sprsq = [0.0086, 0.0086, 0.0259, 0.0259, 0.9310];
        for (k, step) in h.iter().enumerate() {
            assert_eq!(step.ncl, 5 - k, "step {k} ncl");
            assert!(
                (step.rsq.max(0.0) - expect_rsq[k]).abs() < 5e-4,
                "step {k} rsq={} expected {}",
                step.rsq,
                expect_rsq[k]
            );
            assert!(
                (step.sprsq - expect_sprsq[k]).abs() < 5e-4,
                "step {k} sprsq={} expected {}",
                step.sprsq,
                expect_sprsq[k]
            );
        }
        // Final within-SS == SS_total.
        let cum: f64 = h.iter().map(|s| s.sprsq * 58.0).sum();
        assert!((cum - ss[4]).abs() < 1e-9, "cum SS = {cum}");
    }

    /// 4 points in 2 well-separated pairs: cluster {0,1} and {2,3} must each
    /// form before the final all-into-one merge (i.e. at NCl=2 there are
    /// exactly the two correct pairs joined into one).
    #[test]
    fn two_pairs_group_correctly() {
        // pair A near 0, pair B near 100.
        let coords = vec![vec![0.0], vec![1.0], vec![100.0], vec![101.0]];
        let labels: Vec<String> = (0..4).map(|i| format!("OB{}", i + 1)).collect();
        let h = agglomerate(&coords, LinkMethod::Ward, &labels);
        // First merge (NCl=3): OB1+OB2 (smallest indices, tie with OB3+OB4).
        assert_eq!(h[0].ncl, 3);
        assert_eq!(h[0].joined_a, "OB1");
        assert_eq!(h[0].joined_b, "OB2");
        // Second merge (NCl=2): OB3+OB4 forms.
        assert_eq!(h[1].ncl, 2);
        assert_eq!(h[1].joined_a, "OB3");
        assert_eq!(h[1].joined_b, "OB4");
        // Final merge joins the two composite clusters CL3 and CL2.
        assert_eq!(h[2].ncl, 1);
        let joined: Vec<&str> = vec![h[2].joined_a.as_str(), h[2].joined_b.as_str()];
        assert!(joined.contains(&"CL3"), "{joined:?}");
        assert!(joined.contains(&"CL2"), "{joined:?}");
    }

    #[test]
    fn execute_listing_smoke() {
        let mut session = make_session();
        let df = df!["x" => [1.0_f64, 2.0, 3.0, 7.0, 8.0, 9.0]].unwrap();
        let ds = SasDataset {
            df,
            vars: vec![num_meta("x")],
        };
        write_dataset(&mut session, "PTS", ds);
        let ast = ClusterAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "PTS".into() }),
            method: LinkMethod::Ward,
            outtree: None,
            print: None,
            noeigen: false,
            var: vec!["x".into()],
            id: None,
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains("The CLUSTER Procedure"), "{listing}");
        assert!(listing.contains("Cluster History"), "{listing}");
        assert!(listing.contains("0.9310"), "{listing}");
    }
}
