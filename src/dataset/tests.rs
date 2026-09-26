use super::*;
use crate::library::LibraryProvider as _;
use crate::missing::{decode_nan, encode_special};
use crate::testkit::*;
use crate::value::{MissingKind, VarType};
use polars::df;
use std::collections::HashSet;

/// Garantie centrale des missings spéciaux : le NaN-payload survit
/// BIT À BIT à write_parquet → read_parquet (parquet stocke les
/// doubles tels quels ; Polars ne canonicalise pas le NaN). Si ce
/// test casse un jour (canonicalisation), c'est un blocage à
/// remonter — pas à contourner par un encodage parallèle.
#[test]
fn parquet_roundtrip_preserves_special_missing_nan_payloads() {
    let kinds = [
        MissingKind::Letter(0),  // .A
        MissingKind::Underscore, // ._
        MissingKind::Letter(25), // .Z
    ];
    let vals: Vec<Option<f64>> = kinds
        .iter()
        .map(|k| Some(encode_special(*k)))
        .chain([None, Some(1.5)]) // `.` ordinaire = null, et un nombre.
        .collect();
    let df = df!("x" => &vals).unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x")],
    };

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.parquet");
    ds.write_parquet(&path).unwrap();
    let (back, notes) = SasDataset::read_parquet(&path).unwrap();
    assert!(notes.is_empty(), "unexpected coercion notes: {notes:?}");

    let col = back.df.column("x").unwrap().f64().unwrap();
    // `.` ordinaire : null Polars — et UN SEUL null dans la colonne
    // (les spéciaux ne sont PAS des nulls).
    assert_eq!(col.null_count(), 1);
    assert_eq!(col.get(3), None);
    // Spéciaux : des NaN (pas des nulls) dont le payload est intact.
    for (i, kind) in kinds.iter().enumerate() {
        let v = col.get(i).expect("special missing must not be null");
        assert!(v.is_nan());
        assert_eq!(
            v.to_bits(),
            encode_special(*kind).to_bits(),
            "parquet canonicalized the NaN payload for {kind:?}"
        );
        assert_eq!(decode_nan(v), *kind);
    }
    // Et un nombre ordinaire passe inchangé.
    assert_eq!(col.get(4), Some(1.5));
}

// ── J04-P1 : protocole d'écriture atomique (ADR 0001) ─────────────────────
//
// Les valeurs attendues proviennent de la sémantique documentée par l'ADR
// (ordre de publication, empreinte, diagnostic de péremption) — pas d'un
// oracle généré par l'implémentation.

/// Dataset `x` (num) + `c` (char) de `rows` lignes, SANS métadonnées
/// persistables (aucun sidecar ne doit être écrit).
fn atomic_ds_plain(rows: usize) -> SasDataset {
    let x: Vec<f64> = (0..rows).map(|i| i as f64).collect();
    // Valeurs mono-caractère : longueur inférée 1 → AUCUNE métadonnée
    // persistable (la règle « longueur déclarée > 1 » ne se déclenche pas).
    let c: Vec<String> = (0..rows)
        .map(|i| ((b'a' + i as u8) as char).to_string())
        .collect();
    let df = df!("x" => x, "c" => c).unwrap();
    SasDataset::from_dataframe(df).unwrap().0
}

/// Même tableau, mais `c` porte format + libellé + longueur déclarée
/// (un sidecar DOIT être écrit).
fn atomic_ds_meta(rows: usize, fmt: &str, label: &str) -> SasDataset {
    let mut ds = atomic_ds_plain(rows);
    for v in &mut ds.vars {
        if v.ty == VarType::Char {
            v.format = Some(fmt.to_string());
            v.label = Some(label.to_string());
            v.length = 8;
        }
    }
    ds
}

fn dir_entries(dir: &Path) -> HashSet<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}

/// Publication complète : le dossier final ne contient QUE le parquet et son
/// sidecar (aucun temporaire résiduel), et le round-trip restitue les
/// métadonnées — SANS aucun diagnostic de péremption.
#[test]
fn atomic_write_publishes_parquet_and_matching_sidecar() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.parquet");
    atomic_ds_meta(3, "F1.", "lab")
        .write_parquet(&path)
        .unwrap();

    let mut expected = HashSet::new();
    expected.insert("t.parquet".to_string());
    expected.insert("t.parquet.sasmeta.json".to_string());
    assert_eq!(dir_entries(dir.path()), expected, "residual temporaries");

    let (back, notes) = SasDataset::read_parquet(&path).unwrap();
    assert!(notes.is_empty(), "unexpected notes: {notes:?}");
    let c = back.vars.iter().find(|v| v.name == "c").unwrap();
    assert_eq!(c.format.as_deref(), Some("F1."));
    assert_eq!(c.label.as_deref(), Some("lab"));
    assert_eq!(c.length, 8);
}

/// Cœur de la garantie : un sidecar dont l'empreinte ne correspond pas au
/// parquet présent est périmé → ignoré, AVEC diagnostic, jamais appliqué.
#[test]
fn atomic_write_stale_sidecar_ignored_with_diagnostic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.parquet");
    atomic_ds_meta(2, "F1.", "lab")
        .write_parquet(&path)
        .unwrap();

    // Remplace le parquet SANS passer par le protocole (simulation d'un
    // accident externe) : le sidecar resté en place ne peut plus correspondre.
    let other = dir.path().join("other.parquet");
    atomic_ds_plain(7).write_parquet(&other).unwrap();
    std::fs::copy(&other, &path).unwrap();

    let (back, notes) = SasDataset::read_parquet(&path).unwrap();
    assert_eq!(back.n_obs(), 7, "new data must be readable");
    assert!(
        notes.iter().any(|n| n.contains("Stale metadata sidecar")),
        "missing staleness diagnostic: {notes:?}"
    );
    let c = back.vars.iter().find(|v| v.name == "c").unwrap();
    assert_eq!(c.format, None, "stale format must NOT apply");
    assert_eq!(c.label, None, "stale label must NOT apply");
}

/// Réécrire la même table rafraîchit le sidecar (même forme, métadonnées
/// différentes) : l'ancien sidecar ne survit pas à la nouvelle publication.
#[test]
fn atomic_write_rewrite_refreshes_sidecar() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.parquet");
    atomic_ds_meta(2, "F1.", "old")
        .write_parquet(&path)
        .unwrap();
    atomic_ds_meta(2, "F2.", "new")
        .write_parquet(&path)
        .unwrap();

    let (back, notes) = SasDataset::read_parquet(&path).unwrap();
    assert!(notes.is_empty(), "unexpected notes: {notes:?}");
    let c = back.vars.iter().find(|v| v.name == "c").unwrap();
    assert_eq!(c.format.as_deref(), Some("F2."));
    assert_eq!(c.label.as_deref(), Some("new"));
}

/// Sans métadonnées : aucun sidecar créé, et un sidecar périmé d'une
/// écriture précédente est supprimé (le dossier ne garde que le parquet).
#[test]
fn atomic_write_without_meta_writes_no_sidecar_and_removes_old() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.parquet");
    atomic_ds_meta(2, "F1.", "lab")
        .write_parquet(&path)
        .unwrap();
    assert!(sidecar_path(&path).is_file());
    atomic_ds_plain(2).write_parquet(&path).unwrap();

    let mut expected = HashSet::new();
    expected.insert("t.parquet".to_string());
    assert_eq!(dir_entries(dir.path()), expected, "sidecar must be removed");

    let (back, notes) = SasDataset::read_parquet(&path).unwrap();
    assert!(notes.is_empty(), "unexpected notes: {notes:?}");
    assert!(
        back.vars
            .iter()
            .all(|v| v.format.is_none() && v.label.is_none())
    );
}

/// Temporaires orphelins d'une écriture interrompue : jamais des tables
/// (list) et purgés par la prochaine écriture de la même cible.
#[test]
fn atomic_write_orphan_temps_ignored_by_list_then_cleaned() {
    let dir = tempfile::tempdir().unwrap();
    let lib = crate::library::DirLibrary::new(dir.path().to_path_buf());
    lib.write("t", &atomic_ds_plain(1)).unwrap();

    // Orphelins simulés d'une écriture tuée en plein milieu.
    std::fs::write(dir.path().join("t.parquet.sasrs-tmp.999"), b"x").unwrap();
    std::fs::write(
        dir.path().join("t.parquet.sasmeta.json.sasrs-tmp.999"),
        b"x",
    )
    .unwrap();
    assert_eq!(
        lib.list().unwrap(),
        vec!["T".to_string()],
        "orphans listed!"
    );

    lib.write("t", &atomic_ds_plain(2)).unwrap();
    assert!(!dir.path().join("t.parquet.sasrs-tmp.999").exists());
    assert!(
        !dir.path()
            .join("t.parquet.sasmeta.json.sasrs-tmp.999")
            .exists()
    );
    assert_eq!(lib.list().unwrap(), vec!["T".to_string()]);
}

/// Suppression d'une table : le sidecar part avec le parquet (pas de sidecar
/// orphelin pouvant correspondre à une future réécriture).
#[test]
fn atomic_write_delete_removes_sidecar_too() {
    let dir = tempfile::tempdir().unwrap();
    let lib = crate::library::DirLibrary::new(dir.path().to_path_buf());
    lib.write("t", &atomic_ds_meta(2, "F1.", "lab")).unwrap();
    assert!(sidecar_path(&dir.path().join("t.parquet")).is_file());
    lib.delete("t").unwrap();
    assert!(dir_entries(dir.path()).is_empty());
}

/// Tests d'interruption : le binaire de test est relancé en PROCESSUS ENFANT
/// avec `SASRS_FAULT_INJECT` posé ; le point de panne tue l'enfant
/// (exit 86), exactement comme un kill pendant l'écriture. L'état sur disque
/// est alors vérifié par le parent. Requiert la feature `fault-injection`.
#[cfg(feature = "fault-injection")]
mod atomic_write_faults {
    use super::*;
    use std::process::{Command, Output};

    // `--exact` filtre sur le chemin COMPLET du test.
    const DRIVER: &str = "dataset::tests::atomic_write_faults::atomic_write_fault_child_driver";

    fn run_child(dir: &Path, point: &str) -> Output {
        Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg(DRIVER)
            .env("SASRS_TEST_ATOMIC_DIR", dir)
            .env("SASRS_FAULT_INJECT", point)
            .output()
            .unwrap()
    }

    fn assert_killed_at_fault(out: &Output) {
        assert_eq!(
            out.status.code(),
            Some(86),
            "child did not die at the fault point — stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn char_var(ds: &SasDataset) -> &VarMeta {
        ds.vars.iter().find(|v| v.name == "c").unwrap()
    }

    /// Driver exécuté DANS l'enfant : écrit la version B (2 lignes, méta
    /// `F2.`) — tué par le point de panne en cours de route.
    #[test]
    fn atomic_write_fault_child_driver() {
        let Ok(dir) = std::env::var("SASRS_TEST_ATOMIC_DIR") else {
            return; // appel direct (cargo test) : rien à faire
        };
        let path = std::path::Path::new(&dir).join("t.parquet");
        atomic_ds_meta(2, "F2.", "child")
            .write_parquet(&path)
            .unwrap();
    }

    /// Interruption AVANT publication du parquet : la version précédente de
    /// la table est intacte, métadonnées comprises ; le temporaire orphelin
    /// n'est pas une table.
    #[test]
    fn atomic_write_interrupted_after_parquet_tmp_keeps_previous_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.parquet");
        atomic_ds_meta(1, "F1.", "parent")
            .write_parquet(&path)
            .unwrap();

        let out = run_child(dir.path(), "after_parquet_tmp");
        assert_killed_at_fault(&out);

        let (back, notes) = SasDataset::read_parquet(&path).unwrap();
        assert_eq!(back.n_obs(), 1, "old row count must survive");
        assert!(notes.is_empty(), "old metadata must stay valid: {notes:?}");
        assert_eq!(char_var(&back).format.as_deref(), Some("F1."));
        let lib = crate::library::DirLibrary::new(dir.path().to_path_buf());
        assert_eq!(lib.list().unwrap(), vec!["T".to_string()]);
    }

    /// L'INVARIANT du protocole : interruption APRÈS publication du parquet
    /// (avant le sidecar). Les nouvelles données sont lisibles, mais
    /// l'ANCIEN sidecar — périmé — est ignoré avec diagnostic : jamais de
    /// nouvelles données avec de fausses métadonnées.
    #[test]
    fn atomic_write_interrupted_after_parquet_rename_never_applies_old_meta() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.parquet");
        atomic_ds_meta(1, "F1.", "parent")
            .write_parquet(&path)
            .unwrap();

        let out = run_child(dir.path(), "after_parquet_rename");
        assert_killed_at_fault(&out);

        let (back, notes) = SasDataset::read_parquet(&path).unwrap();
        assert_eq!(back.n_obs(), 2, "new data IS published");
        assert_eq!(char_var(&back).format, None, "old format must NOT apply");
        assert_eq!(char_var(&back).label, None, "old label must NOT apply");
        assert!(
            notes.iter().any(|n| n.contains("Stale metadata sidecar")),
            "missing staleness diagnostic: {notes:?}"
        );
    }

    /// Interruption après le temporaire du sidecar : même invariant — données
    /// nouvelles, ancien sidecar périmé ignoré, orphelins ignorés par `list`.
    #[test]
    fn atomic_write_interrupted_after_sidecar_tmp_same_invariant() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.parquet");
        atomic_ds_meta(1, "F1.", "parent")
            .write_parquet(&path)
            .unwrap();

        let out = run_child(dir.path(), "after_sidecar_tmp");
        assert_killed_at_fault(&out);

        let (back, notes) = SasDataset::read_parquet(&path).unwrap();
        assert_eq!(back.n_obs(), 2);
        assert_eq!(char_var(&back).format, None);
        assert!(
            notes.iter().any(|n| n.contains("Stale metadata sidecar")),
            "missing staleness diagnostic: {notes:?}"
        );
        // Le temporaire sidecar orphelin existe et n'est pas une table.
        let orphans: Vec<String> = dir_entries(dir.path())
            .into_iter()
            .filter(|n| n.contains(".sasrs-tmp."))
            .collect();
        assert!(!orphans.is_empty(), "expected an orphan sidecar temp");
        let lib = crate::library::DirLibrary::new(dir.path().to_path_buf());
        assert_eq!(lib.list().unwrap(), vec!["T".to_string()]);
    }
}
