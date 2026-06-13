use crate::error::{Result, SasError};
use crate::value::VarType;
use polars::prelude::*;
use std::fs::File;
use std::path::Path;

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
    /// Storage length: bytes for char (display width), 8 for numeric.
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
        let df = ParquetReader::new(file).finish()?;
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

        // Métadonnées SAS (format/label) persistées dans un sidecar JSON :
        // le Parquet ne porte que types et données ; format et libellé (qui,
        // en SAS, ne sont QUE de l'affichage) survivent au round-trip via ce
        // fichier annexe. Absent → on garde les VarMeta dérivés du Parquet
        // (rétro-compatible : datasets écrits sans format/label).
        if let Some(meta_map) = read_sidecar(path) {
            for v in &mut vars {
                if let Some(saved) = meta_map.get(&v.name.to_uppercase()) {
                    // Le format/libellé sauvegardé l'emporte (y compris pour
                    // remplacer le DATE9. inféré d'une colonne Date physique).
                    if saved.format.is_some() {
                        v.format = saved.format.clone();
                    }
                    if saved.label.is_some() {
                        v.label = saved.label.clone();
                    }
                }
            }
        }

        let df = DataFrame::new(columns)?;
        Ok((SasDataset { df, vars }, notes))
    }

    pub fn write_parquet(&self, path: &Path) -> Result<()> {
        let mut file = File::create(path)?;
        let mut df = self.df.clone();
        ParquetWriter::new(&mut file).finish(&mut df)?;
        write_sidecar(path, &self.vars)?;
        Ok(())
    }
}

/// Métadonnée SAS persistée par variable (format/libellé). Le type et la
/// longueur se redéduisent du Parquet ; seuls format et libellé doivent être
/// conservés à part.
#[derive(serde::Serialize, serde::Deserialize)]
struct SavedMeta {
    format: Option<String>,
    label: Option<String>,
}

/// Chemin du sidecar JSON associé à un fichier parquet (`t.parquet` →
/// `t.parquet.sasmeta.json`).
fn sidecar_path(path: &Path) -> std::path::PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".sasmeta.json");
    std::path::PathBuf::from(s)
}

/// Écrit le sidecar de métadonnées si AU MOINS une variable porte un format
/// ou un libellé ; sinon, supprime un sidecar éventuellement obsolète (et
/// n'en crée aucun — round-trip identique pour les datasets sans
/// format/label, stabilité des snapshots existants).
fn write_sidecar(path: &Path, vars: &[VarMeta]) -> Result<()> {
    let has_meta = vars.iter().any(|v| v.format.is_some() || v.label.is_some());
    let sc = sidecar_path(path);
    if !has_meta {
        let _ = std::fs::remove_file(&sc);
        return Ok(());
    }
    let map: std::collections::HashMap<String, SavedMeta> = vars
        .iter()
        .map(|v| {
            (
                v.name.to_uppercase(),
                SavedMeta {
                    format: v.format.clone(),
                    label: v.label.clone(),
                },
            )
        })
        .collect();
    let json = serde_json::to_string(&map)
        .map_err(|e| SasError::runtime(format!("failed to serialize SAS metadata: {e}")))?;
    std::fs::write(&sc, json)?;
    Ok(())
}

/// Lit le sidecar de métadonnées s'il existe (nom UPPERCASE → métadonnée).
/// Toute erreur de lecture/parsing est silencieusement ignorée (on retombe
/// sur les VarMeta dérivés du Parquet).
fn read_sidecar(path: &Path) -> Option<std::collections::HashMap<String, SavedMeta>> {
    let sc = sidecar_path(path);
    let data = std::fs::read_to_string(&sc).ok()?;
    serde_json::from_str(&data).ok()
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
                let overflow = ca
                    .iter()
                    .flatten()
                    .any(|v| v.abs() > MAX_EXACT_INT);
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
            let secs: Float64Chunked = raw
                .f64()?
                .iter()
                .map(|o| o.map(|v| v / 1e9))
                .collect();
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
mod tests {
    use super::*;
    use crate::missing::{decode_nan, encode_special};
    use crate::value::MissingKind;
    use polars::df;

    fn num_meta(name: &str) -> VarMeta {
        VarMeta {
            name: name.to_string(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        }
    }

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
}
