//! Tests du provider CSV et de la résolution des librefs.

use super::*;

fn make_ds(vals: Vec<i32>, names: Vec<&str>) -> SasDataset {
    // Build a small DataFrame with one numeric column and one char column.
    let numeric = Series::new(
        "x".into(),
        vals.iter().map(|&v| v as f64).collect::<Vec<_>>(),
    );
    let chars = Series::new("name".into(), names);
    let df = DataFrame::new(vec![numeric.into(), chars.into()]).unwrap();
    SasDataset::from_dataframe(df).unwrap().0
}

#[test]
fn csv_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let lib = CsvLibrary::new(tmp.path().to_path_buf());
    let ds = make_ds(vec![1, 2, 3], vec!["a", "b", "c"]);
    lib.write("mytable", &ds).unwrap();

    let path = tmp.path().join("mytable.csv");
    assert!(path.is_file(), "CSV file should exist after write");

    let (ds2, _) = lib.read("mytable").unwrap();
    assert_eq!(ds2.df.height(), 3, "row count");
    assert!(ds2.df.column("x").is_ok(), "numeric column present");
    assert!(ds2.df.column("name").is_ok(), "char column present");
}

#[test]
fn csv_round_trip_values() {
    let tmp = tempfile::tempdir().unwrap();
    let lib = CsvLibrary::new(tmp.path().to_path_buf());
    let ds = make_ds(vec![10, 20], vec!["foo", "bar"]);
    lib.write("t", &ds).unwrap();
    let (ds2, _) = lib.read("t").unwrap();
    let col = ds2.df.column("x").unwrap();
    // CSV is read back as floats or ints – check values via to_string.
    let s: Vec<f64> = col
        .cast(&DataType::Float64)
        .unwrap()
        .f64()
        .unwrap()
        .into_no_null_iter()
        .collect();
    assert_eq!(s, vec![10.0, 20.0]);
}

#[test]
fn csv_exists() {
    let tmp = tempfile::tempdir().unwrap();
    let lib = CsvLibrary::new(tmp.path().to_path_buf());
    assert!(!lib.exists("none"));
    let ds = make_ds(vec![1], vec!["x"]);
    lib.write("none", &ds).unwrap();
    assert!(lib.exists("none"));
    // Case-insensitive: table name is lowercased for the file.
    assert!(lib.exists("NONE"));
}

#[test]
fn csv_list() {
    let tmp = tempfile::tempdir().unwrap();
    let lib = CsvLibrary::new(tmp.path().to_path_buf());
    assert_eq!(lib.list().unwrap(), Vec::<String>::new());
    let ds = make_ds(vec![1], vec!["v"]);
    lib.write("alpha", &ds).unwrap();
    lib.write("beta", &ds).unwrap();
    let names = lib.list().unwrap();
    assert_eq!(names, vec!["ALPHA".to_string(), "BETA".to_string()]);
}

#[test]
fn csv_delete() {
    let tmp = tempfile::tempdir().unwrap();
    let lib = CsvLibrary::new(tmp.path().to_path_buf());
    let ds = make_ds(vec![1], vec!["v"]);
    lib.write("todelete", &ds).unwrap();
    assert!(lib.exists("todelete"));
    lib.delete("todelete").unwrap();
    assert!(!lib.exists("todelete"));
}

#[test]
fn csv_rename() {
    let tmp = tempfile::tempdir().unwrap();
    let lib = CsvLibrary::new(tmp.path().to_path_buf());
    let ds = make_ds(vec![1], vec!["v"]);
    lib.write("old", &ds).unwrap();
    lib.rename("old", "new").unwrap();
    assert!(!lib.exists("old"));
    assert!(lib.exists("new"));
}

#[test]
fn csv_rename_nonexistent_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let lib = CsvLibrary::new(tmp.path().to_path_buf());
    let err = lib.rename("ghost", "new").unwrap_err();
    assert!(err.to_string().contains("does not exist"), "{err}");
}

#[test]
fn csv_read_nonexistent_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let lib = CsvLibrary::new(tmp.path().to_path_buf());
    let err_msg = lib
        .read("nobody")
        .err()
        .expect("expected error reading non-existent table")
        .to_string();
    assert!(err_msg.contains("does not exist"), "{err_msg}");
}

#[test]
fn csv_scan_lazy_works() {
    let tmp = tempfile::tempdir().unwrap();
    let lib = CsvLibrary::new(tmp.path().to_path_buf());
    let ds = make_ds(vec![42], vec!["q"]);
    lib.write("lazy", &ds).unwrap();
    let lf = lib.scan("lazy").unwrap();
    let df = lf.collect().unwrap();
    assert_eq!(df.height(), 1);
}

#[test]
fn csv_is_not_cloud() {
    let tmp = tempfile::tempdir().unwrap();
    let lib = CsvLibrary::new(tmp.path().to_path_buf());
    assert!(!lib.is_cloud());
}

#[test]
fn assign_csv_registers_libref() {
    let tmp = tempfile::tempdir().unwrap();
    let mut mgr = LibraryManager::new(None).unwrap();
    mgr.assign_csv("csvlib", tmp.path().to_path_buf()).unwrap();
    let prov = mgr.get("csvlib").unwrap();
    assert!(!prov.is_cloud());
}

#[test]
fn assign_csv_rejects_missing_dir() {
    let mut mgr = LibraryManager::new(None).unwrap();
    let err = mgr
        .assign_csv("x", PathBuf::from("/nonexistent/path/xyz"))
        .unwrap_err();
    assert!(err.to_string().contains("does not exist"), "{err}");
}

#[cfg(feature = "s3")]
mod s3_tests {
    use super::*;

    #[test]
    fn builds_s3_uri_lowercasing_table() {
        let lib = S3Library::new("my-bucket", "data/sas");
        assert_eq!(lib.uri("Class"), "s3://my-bucket/data/sas/class.parquet");
    }

    #[test]
    fn empty_prefix_has_no_double_slash() {
        let lib = S3Library::new("my-bucket", "");
        assert_eq!(lib.uri("CLASS"), "s3://my-bucket/class.parquet");
    }

    #[test]
    fn surrounding_slashes_in_prefix_are_trimmed() {
        let lib = S3Library::new("my-bucket", "/trimmed/");
        assert_eq!(lib.uri("t"), "s3://my-bucket/trimmed/t.parquet");
    }

    #[test]
    fn mutating_ops_return_runtime_errors() {
        let lib = S3Library::new("b", "p");
        let ds = SasDataset {
            df: DataFrame::empty(),
            vars: Vec::new(),
        };
        assert!(lib.write("t", &ds).is_err());
        assert!(lib.delete("t").is_err());
        assert!(lib.rename("a", "b").is_err());
        assert!(lib.list().is_err());
        assert!(!lib.exists("t"));
    }

    // ── from_uri: parsing s3://bucket/prefix ────────────────────────────────

    #[test]
    fn from_uri_splits_bucket_and_prefix() {
        let lib = S3Library::from_uri("s3://my-bucket/data/sas").unwrap();
        assert_eq!(lib.bucket, "my-bucket");
        assert_eq!(lib.prefix, "data/sas");
        // Round-trips through the URI builder.
        assert_eq!(lib.uri("Class"), "s3://my-bucket/data/sas/class.parquet");
    }

    #[test]
    fn from_uri_bucket_only_has_empty_prefix() {
        let lib = S3Library::from_uri("s3://my-bucket").unwrap();
        assert_eq!(lib.bucket, "my-bucket");
        assert_eq!(lib.prefix, "");
        assert_eq!(lib.uri("CLASS"), "s3://my-bucket/class.parquet");
    }

    #[test]
    fn from_uri_trims_trailing_slash() {
        let lib = S3Library::from_uri("s3://my-bucket/data/sas/").unwrap();
        assert_eq!(lib.prefix, "data/sas");
        // Bucket with a bare trailing slash → empty prefix.
        let lib2 = S3Library::from_uri("s3://my-bucket/").unwrap();
        assert_eq!(lib2.bucket, "my-bucket");
        assert_eq!(lib2.prefix, "");
    }

    #[test]
    fn from_uri_rejects_non_s3_or_empty_bucket() {
        assert!(S3Library::from_uri("/local/path").is_err());
        assert!(S3Library::from_uri("s3:///just/prefix").is_err());
    }

    #[test]
    fn s3_library_reports_cloud_marker() {
        let lib = S3Library::new("b", "p");
        assert!(lib.is_cloud());
    }

    // ── Provider selection via LibraryManager::assign ───────────────────────

    #[test]
    fn assign_s3_uri_selects_cloud_provider() {
        let mgr = LibraryManager::new(None).unwrap();
        // A normal local path → DirLibrary (not cloud).
        let tmp = tempfile::TempDir::new().unwrap();
        let mut mgr = mgr;
        mgr.assign("loc", tmp.path().to_path_buf()).unwrap();
        assert!(!mgr.get("loc").unwrap().is_cloud());

        // An s3:// path → S3Library (cloud), no directory check, no network I/O.
        mgr.assign_uri("cloudlib", "s3://my-bucket/data").unwrap();
        let prov = mgr.get("cloudlib").unwrap();
        assert!(prov.is_cloud());
    }
}
