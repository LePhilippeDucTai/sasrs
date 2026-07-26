use super::*;
use crate::dataset::SasDataset;
use crate::source::SourceFile;
use crate::testkit::*;
use polars::df;

fn parse_fastclus(src: &str) -> Result<FastclusAst> {
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next();
    ts.next();
    parse(&mut ts)
}

#[test]
fn parse_minimal() {
    let ast =
        parse_fastclus("proc fastclus data=a maxclusters=2 out=b maxiter=20; var x; run;").unwrap();
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
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "PTS".into(),
        }),
        out: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "CL".into(),
        }),
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
