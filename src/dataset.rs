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

        let df = DataFrame::new(columns)?;
        Ok((SasDataset { df, vars }, notes))
    }

    pub fn write_parquet(&self, path: &Path) -> Result<()> {
        let mut file = File::create(path)?;
        let mut df = self.df.clone();
        ParquetWriter::new(&mut file).finish(&mut df)?;
        Ok(())
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
