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
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "PTS".into(),
        }),
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

#[test]
fn outtree_dataset_shape_and_monotone() {
    use crate::missing::value_to_num;
    use crate::procs::common::decode_column;

    let mut session = make_session();
    let df = df!["x" => [1.0_f64, 2.0, 3.0, 7.0, 8.0, 9.0]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "PTS", ds);
    let ast = ClusterAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "PTS".into(),
        }),
        method: LinkMethod::Ward,
        outtree: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "TREE".into(),
        }),
        print: None,
        noeigen: false,
        var: vec!["x".into()],
        id: None,
    };
    execute(&ast, &mut session).unwrap();

    let (tree, _) = session.libs.get("WORK").unwrap().read("TREE").unwrap();
    // Row count = #leaves (6) + #merges (5) = 11.
    assert_eq!(tree.n_obs(), 11, "row count");
    // Columns: _NAME_ _PARENT_ _NCL_ _FREQ_ _HEIGHT_ x.
    let names: Vec<String> = tree.vars.iter().map(|v| v.name.to_uppercase()).collect();
    assert!(names.contains(&"_NAME_".to_string()));
    assert!(names.contains(&"_PARENT_".to_string()));
    assert!(names.contains(&"_HEIGHT_".to_string()));
    assert!(names.contains(&"X".to_string()));

    // _HEIGHT_ over the cluster rows (last 5) must be monotone non-decreasing.
    let hcol = tree.vars.iter().position(|v| v.name == "_HEIGHT_").unwrap();
    let heights: Vec<f64> = decode_column(&tree, hcol)
        .unwrap()
        .iter()
        .map(|v| value_to_num(v).unwrap_or(f64::NAN))
        .collect();
    let merge_heights = &heights[6..]; // skip the 6 leaves
    for w in merge_heights.windows(2) {
        assert!(
            w[0] <= w[1] + 1e-12,
            "heights not monotone: {merge_heights:?}"
        );
    }
}
