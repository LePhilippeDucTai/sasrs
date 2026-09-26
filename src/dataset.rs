//! Table SAS en mémoire : DataFrame Polars + métadonnées de variables.
//!
//! [`SasDataset`] associe le `DataFrame` et un `Vec<VarMeta>` (nom, type SAS,
//! longueur, format, label) dans l'ORDRE des colonnes — le modèle SAS n'a que
//! deux types, numérique (f64) et caractère, et les colonnes Parquet natives
//! (entiers, dates, booléens) sont coercées à la lecture.
//!
//! Les métadonnées qui n'ont pas d'équivalent Parquet (format, label) voyagent
//! dans un fichier annexe JSON à côté du `.parquet`.
//!
//! # Écriture atomique (ADR 0001)
//!
//! La publication d'une table est un protocole en quatre temps : parquet vers
//! un temporaire du MÊME dossier + fsync, rename atomique (les données sont
//! publiées), sidecar vers un temporaire + fsync, rename. Le sidecar porte
//! l'empreinte du parquet publié (taille, lignes, colonnes) ; un sidecar dont
//! l'empreinte ne correspond pas est périmé : ignoré à la lecture avec un
//! WARNING. Une interruption ne publie donc JAMAIS de nouvelles données avec
//! des métadonnées fausses.

use crate::error::{Result, SasError};
use crate::value::VarType;
use polars::prelude::*;
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Days between 1960-01-01 (SAS epoch) and 1970-01-01 (Unix epoch).
pub const SAS_EPOCH_OFFSET_DAYS: f64 = 3653.0;
/// Seconds between the two epochs.
pub const SAS_EPOCH_OFFSET_SECONDS: f64 = 3653.0 * 86400.0;
/// Largest integer exactly representable in an f64.
const MAX_EXACT_INT: i64 = 1 << 53;

#[derive(Debug, Clone)]
pub struct VarMeta {
    pub name: String,
    pub ty: VarType,
    /// Longueur de stockage d'une variable caractère, en CARACTÈRES (contrat
    /// D-001, docs/encoding.md — convention session SAS LATIN1/WLATIN1) ;
    /// 8 pour du numérique.
    pub length: usize,
    pub format: Option<String>,
    pub label: Option<String>,
}

/// A SAS dataset: a Polars DataFrame restricted to the SAS type model
/// (Float64 + String columns) plus per-variable metadata.
pub struct SasDataset {
    pub df: DataFrame,
    pub vars: Vec<VarMeta>,
}

impl SasDataset {
    pub fn n_obs(&self) -> usize {
        self.df.height()
    }

    pub fn n_vars(&self) -> usize {
        self.vars.len()
    }

    /// Read a parquet file, coercing native types into the strict SAS model
    /// (numeric = f64, character = string). Dates/datetimes/times become
    /// numbers on the SAS epoch carrying a default display format, exactly
    /// like SAS where the format is what makes a number a date.
    /// Returns the dataset plus NOTE/WARNING lines for the log.
    pub fn read_parquet(path: &Path) -> Result<(SasDataset, Vec<String>)> {
        let file = File::open(path)?;
        // Empreinte du fichier TEL QU'IL EST sur disque — comparée à celle du
        // sidecar : une divergence signifie un sidecar périmé (ADR 0001).
        let size = file.metadata()?.len();
        let df = ParquetReader::new(file).finish()?;
        let fingerprint = SidecarFingerprint {
            size,
            rows: df.height(),
            columns: df.width(),
        };
        let mut notes = Vec::new();
        let mut columns: Vec<Column> = Vec::with_capacity(df.width());
        let mut vars = Vec::with_capacity(df.width());

        for col in df.get_columns() {
            let name = col.name().to_string();
            let s = col.as_materialized_series();
            let (series, meta) = coerce_series(&name, s, &mut notes)?;
            columns.push(series.into());
            vars.push(meta);
        }

        // Métadonnées SAS (format/label/longueur) persistées dans un sidecar JSON :
        // le Parquet ne porte que types et données ; format, libellé et longueur
        // déclarée (qui, en SAS, ne sont QUE de l'affichage) survivent au round-trip
        // via ce fichier annexe. Un sidecar dont l'empreinte ne correspond pas au
        // parquet présent est PÉRIMÉ (écriture interrompue, ancien format, copie
        // manuelle) : ignoré avec un diagnostic — jamais appliqué aux données.
        // Absent → on garde les VarMeta dérivés du Parquet (rétro-compatible :
        // datasets écrits sans format/label/longueur explicite).
        match read_sidecar(path, &fingerprint) {
            SidecarState::Valid(meta_map) => {
                for v in &mut vars {
                    if let Some(saved) = meta_map.get(&v.name.to_uppercase()) {
                        // Le format/libellé/longueur sauvegardés l'emportent (y compris pour
                        // remplacer le DATE9. inféré d'une colonne Date physique).
                        if saved.format.is_some() {
                            v.format = saved.format.clone();
                        }
                        if saved.label.is_some() {
                            v.label = saved.label.clone();
                        }
                        if let Some(saved_len) = saved.length {
                            v.length = saved_len;
                        }
                    }
                }
            }
            SidecarState::Stale => notes.push(format!(
                "WARNING: Stale metadata sidecar ignored for {}: its fingerprint \
                 no longer matches the parquet file.",
                path.display()
            )),
            SidecarState::Absent => {}
        }

        let df = DataFrame::new(columns)?;
        Ok((SasDataset { df, vars }, notes))
    }

    /// Coerce an arbitrary DataFrame (e.g. a PROC SQL result, which may carry
    /// u32/i64/bool/Float64/String columns from aggregates and joins) into the
    /// strict SAS type model: numeric → f64, character → string. Reuses the
    /// same per-column coercion as `read_parquet` so VarMeta inference is
    /// identical. Returns the dataset plus any NOTE/WARNING lines.
    pub fn from_dataframe(df: DataFrame) -> Result<(SasDataset, Vec<String>)> {
        let mut notes = Vec::new();
        let mut columns: Vec<Column> = Vec::with_capacity(df.width());
        let mut vars = Vec::with_capacity(df.width());

        for col in df.get_columns() {
            let name = col.name().to_string();
            let s = col.as_materialized_series();
            let (series, meta) = coerce_series(&name, s, &mut notes)?;
            columns.push(series.into());
            vars.push(meta);
        }

        let df = DataFrame::new(columns)?;
        Ok((SasDataset { df, vars }, notes))
    }

    /// Écriture atomique parquet + sidecar (ADR 0001). Une interruption ne
    /// publie jamais de nouvelles données avec des métadonnées fausses : le
    /// parquet est publié AVANT le sidecar, et tout sidecar dont l'empreinte
    /// ne correspond pas au parquet est ignoré à la lecture.
    pub fn write_parquet(&self, path: &Path) -> Result<()> {
        // Orphelins d'une écriture précédente interrompue sur cette cible.
        clean_orphan_temps(path)?;

        // 1. Parquet vers un temporaire du MÊME dossier (le rename y est
        //    atomique), fsync du fichier avant toute publication.
        let tmp = tmp_path_for(path);
        let mut file = File::create(&tmp)?;
        let mut df = self.df.clone();
        ParquetWriter::new(&mut file).finish(&mut df)?;
        file.sync_all()?;
        let size = file.metadata()?.len();
        drop(file);
        fault_point("after_parquet_tmp");

        // 2. Publication des données : rename atomique + fsync du dossier.
        std::fs::rename(&tmp, path)?;
        fsync_dir(path.parent());
        fault_point("after_parquet_rename");

        // 3. Métadonnées. Si une interruption frappe ici, l'ANCIEN sidecar
        //    subsiste mais son empreinte ne peut plus correspondre au nouveau
        //    parquet : il sera ignoré à la lecture (diagnostic), jamais
        //    appliqué aux nouvelles données.
        let fingerprint = SidecarFingerprint {
            size,
            rows: self.df.height(),
            columns: self.vars.len(),
        };
        write_sidecar(path, &self.vars, fingerprint)
    }
}

/// Empreinte du parquet publiée dans le sidecar : taille en octets, nombre de
/// lignes, nombre de colonnes. Une divergence quelconque ⇒ sidecar périmé.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct SidecarFingerprint {
    size: u64,
    rows: usize,
    columns: usize,
}

/// Contenu du sidecar `<table>.parquet.sasmeta.json` (ADR 0001) : l'empreinte
/// du parquet pour lequel il a été écrit, puis les métadonnées par variable.
#[derive(serde::Serialize, serde::Deserialize)]
struct SidecarFile {
    fingerprint: SidecarFingerprint,
    vars: HashMap<String, SavedMeta>,
}

/// Métadonnée SAS persistée par variable (format/libellé/longueur déclarée).
/// Le type se redéduit du Parquet ; format, libellé et longueur doivent être
/// conservés à part.
#[derive(serde::Serialize, serde::Deserialize)]
struct SavedMeta {
    format: Option<String>,
    label: Option<String>,
    #[serde(default)]
    length: Option<usize>,
}

/// Chemin du sidecar JSON associé à un fichier parquet (`t.parquet` →
/// `t.parquet.sasmeta.json`).
pub(crate) fn sidecar_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".sasmeta.json");
    PathBuf::from(s)
}

/// Marqueur des temporaires d'écriture atomique (ADR 0001) : les fichiers
/// `<cible><TMP_MARKER>.<pid>` ne sont jamais des tables et sont purgés à la
/// prochaine écriture de la même cible.
pub(crate) const TMP_MARKER: &str = ".sasrs-tmp";

/// Chemin du temporaire d'écriture pour `target` (même dossier).
fn tmp_path_for(target: &Path) -> PathBuf {
    let mut s = target.as_os_str().to_os_string();
    s.push(format!("{TMP_MARKER}.{}", std::process::id()));
    PathBuf::from(s)
}

/// fsync best-effort du dossier parent après un rename (durabilité de
/// l'entrée de répertoire ; échoue silencieusement où la plateforme
/// n'offre pas l'ouverture d'un dossier).
fn fsync_dir(parent: Option<&Path>) {
    if let Some(dir) = parent
        && let Ok(d) = File::open(dir)
    {
        let _ = d.sync_all();
    }
}

/// Supprime les temporaires orphelins (`<cible><TMP_MARKER>…`) d'une écriture
/// interrompue sur cette même cible. Best effort : une erreur de purge
/// n'empêche pas l'écriture (l'orphelin est de toute façon ignoré par `list`).
fn clean_orphan_temps(target: &Path) -> Result<()> {
    let Some(dir) = target.parent() else {
        return Ok(());
    };
    let Some(name) = target.file_name() else {
        return Ok(());
    };
    let prefix = format!("{}{TMP_MARKER}", name.to_string_lossy());
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().starts_with(&prefix) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    Ok(())
}

/// Point d'arrêt nommé pour l'injection de pannes (ADR 0001, feature
/// `fault-injection`). Si `SASRS_FAULT_INJECT` (liste de noms séparés par des
/// virgules) contient `name`, le processus s'interrompt sur-le-champ —
/// interruption brute, sans unwind ni destructeur, fidèle à un kill.
#[cfg(feature = "fault-injection")]
pub(crate) fn fault_point(name: &str) {
    let Ok(list) = std::env::var("SASRS_FAULT_INJECT") else {
        return;
    };
    if list.split(',').map(str::trim).any(|n| n == name) {
        // Le message doit sortir AVANT l'exit : stderr non bufferisé suffit.
        eprintln!("SASRS fault point hit: {name}");
        std::process::exit(86);
    }
}

/// Hors feature `fault-injection` : aucun coût, aucun effet.
#[cfg(not(feature = "fault-injection"))]
#[inline(always)]
pub(crate) fn fault_point(_name: &str) {}

/// Écrit le sidecar de métadonnées si AU MOINS une variable porte un format,
/// un libellé ou une longueur déclarée ; sinon, supprime un sidecar éventuellement
/// obsolète (et n'en crée aucun — round-trip identique pour les datasets sans
/// format/label/longueur explicite, stabilité des snapshots existants).
/// L'écriture est atomique (temporaire + fsync + rename, ADR 0001) et le
/// contenu porte l'empreinte du parquet DÉJÀ publié : un sidecar périmé est
/// détectable et ignoré à la lecture.
fn write_sidecar(path: &Path, vars: &[VarMeta], fingerprint: SidecarFingerprint) -> Result<()> {
    let sc = sidecar_path(path);
    // Orphelins d'une tentative précédente — purgés même sans métadonnées.
    clean_orphan_temps(&sc)?;
    let has_meta = vars.iter().any(|v| {
        v.format.is_some() || v.label.is_some() || (v.ty == VarType::Char && v.length > 1) // Save declared lengths > 1
    });
    if !has_meta {
        // Pas de métadonnées : on retire l'éventuel sidecar APRÈS la
        // publication du parquet — entre-temps son empreinte est périmée, il
        // ne peut pas s'appliquer aux nouvelles données.
        let _ = std::fs::remove_file(&sc);
        return Ok(());
    }
    let map: HashMap<String, SavedMeta> = vars
        .iter()
        .map(|v| {
            (
                v.name.to_uppercase(),
                SavedMeta {
                    format: v.format.clone(),
                    label: v.label.clone(),
                    length: if v.ty == VarType::Char {
                        Some(v.length)
                    } else {
                        None
                    },
                },
            )
        })
        .collect();
    let json = serde_json::to_string(&SidecarFile {
        fingerprint,
        vars: map,
    })
    .map_err(|e| SasError::runtime(format!("failed to serialize SAS metadata: {e}")))?;
    let tmp = tmp_path_for(&sc);
    let mut file = File::create(&tmp)?;
    file.write_all(json.as_bytes())?;
    file.sync_all()?;
    fault_point("after_sidecar_tmp");
    std::fs::rename(&tmp, &sc)?;
    fsync_dir(sc.parent());
    fault_point("after_sidecar_rename");
    Ok(())
}

/// État du sidecar à la lecture (ADR 0001).
enum SidecarState {
    /// Aucun sidecar sur disque (dataset sans métadonnées persistées).
    Absent,
    /// Présent mais périmé : empreinte ≠ parquet, ou format illisible
    /// (sidecar d'avant le protocole). À ignorer, avec diagnostic.
    Stale,
    /// Présent, lisible, et dont l'empreinte correspond au parquet lu.
    Valid(HashMap<String, SavedMeta>),
}

/// Lit le sidecar de métadonnées et le compare à l'empreinte du parquet
/// réellement présent : seule une correspondance EXACTE rend les métadonnées
/// applicables.
fn read_sidecar(path: &Path, fingerprint: &SidecarFingerprint) -> SidecarState {
    let sc = sidecar_path(path);
    let Ok(data) = std::fs::read_to_string(sc) else {
        return SidecarState::Absent;
    };
    match serde_json::from_str::<SidecarFile>(&data) {
        Ok(f) if f.fingerprint == *fingerprint => SidecarState::Valid(f.vars),
        _ => SidecarState::Stale,
    }
}

fn coerce_series(name: &str, s: &Series, notes: &mut Vec<String>) -> Result<(Series, VarMeta)> {
    let num_meta = |format: Option<&str>| VarMeta {
        name: name.to_string(),
        ty: VarType::Num,
        length: 8,
        format: format.map(str::to_string),
        label: None,
    };

    let series = match s.dtype() {
        DataType::Float64 => return Ok((s.clone(), num_meta(None))),
        DataType::Float32
        | DataType::Int8
        | DataType::Int16
        | DataType::Int32
        | DataType::UInt8
        | DataType::UInt16
        | DataType::UInt32
        | DataType::Boolean => s.cast(&DataType::Float64)?,
        DataType::Int64 | DataType::UInt64 => {
            let as_i64 = s.cast(&DataType::Int64).ok();
            if let Some(ref i) = as_i64 {
                let ca = i.i64()?;
                let overflow = ca.iter().flatten().any(|v| v.abs() > MAX_EXACT_INT);
                if overflow {
                    notes.push(format!(
                        "WARNING: Column {name} contains integers larger than 2**53; \
                         precision was lost converting to SAS numeric."
                    ));
                }
            }
            s.cast(&DataType::Float64)?
        }
        DataType::Date => {
            // Physical representation: i32 days since 1970-01-01.
            let days = s.cast(&DataType::Float64)?;
            let shifted: Float64Chunked = days
                .f64()?
                .iter()
                .map(|o| o.map(|v| v + SAS_EPOCH_OFFSET_DAYS))
                .collect();
            let series = shifted.into_series().with_name(name.into());
            return Ok((series, num_meta(Some("DATE9."))));
        }
        DataType::Datetime(unit, _) => {
            let divisor = match unit {
                TimeUnit::Nanoseconds => 1e9,
                TimeUnit::Microseconds => 1e6,
                TimeUnit::Milliseconds => 1e3,
            };
            let raw = s.cast(&DataType::Float64)?;
            let shifted: Float64Chunked = raw
                .f64()?
                .iter()
                .map(|o| o.map(|v| v / divisor + SAS_EPOCH_OFFSET_SECONDS))
                .collect();
            let series = shifted.into_series().with_name(name.into());
            return Ok((series, num_meta(Some("DATETIME20."))));
        }
        DataType::Time => {
            // i64 nanoseconds since midnight -> seconds.
            let raw = s.cast(&DataType::Float64)?;
            let secs: Float64Chunked = raw.f64()?.iter().map(|o| o.map(|v| v / 1e9)).collect();
            let series = secs.into_series().with_name(name.into());
            return Ok((series, num_meta(Some("TIME8."))));
        }
        DataType::String => {
            let max_len = s
                .str()?
                .iter()
                .flatten()
                .map(|v| v.chars().count())
                .max()
                .unwrap_or(0)
                .max(1);
            return Ok((
                s.clone(),
                VarMeta {
                    name: name.to_string(),
                    ty: VarType::Char,
                    length: max_len,
                    format: None,
                    label: None,
                },
            ));
        }
        other => {
            return Err(SasError::runtime(format!(
                "column {name} has unsupported parquet type {other} \
                 (SAS supports only numeric and character data)"
            )));
        }
    };

    Ok((series.with_name(name.into()), num_meta(None)))
}

#[cfg(test)]
mod tests;
