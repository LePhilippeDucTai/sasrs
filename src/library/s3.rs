use super::*;

/// Backend de stockage S3 (ou compatible S3) derrière la feature `s3`.
///
/// STATUT (M13) : ce backend est désormais RÉELLEMENT branché. `LIBNAME ref
/// 's3://bucket/prefix';` enregistre une `S3Library` (au lieu d'une
/// `DirLibrary`) et `read`/`scan` font un scan parquet cloud authentique via
/// Polars (`scan_parquet` + `CloudOptions`), porté par object_store/aws-*.
/// Une table `t` d'un libref lié au bucket `b`/préfixe `p` est mappée sur l'URI
/// `s3://b/p/t.parquet`, lue par le scanner parquet de Polars (chemin de
/// coercition identique à `DirLibrary`, seul l'URI et le transport changent).
///
/// Tout ce code n'est compilé qu'avec la feature `s3` (qui tire `polars/cloud`
/// + `polars/aws`). Sous le build par défaut, ce backend n'existe pas et un
/// chemin `s3://` est traité comme aujourd'hui (chemin local).
///
/// Credentials / région : `read`/`scan` dérivent les `CloudOptions` de
/// l'environnement via `CloudOptions::from_untyped_config(uri, [])`, qui laisse
/// object_store détecter les variables AWS standard (`AWS_REGION` /
/// `AWS_DEFAULT_REGION`, `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`,
/// `AWS_SESSION_TOKEN`, `AWS_ENDPOINT_URL`, profils/IMDS…). Aucune credential
/// n'est codée en dur.
///
/// Les opérations mutantes (write/delete/rename/list) restent non gérées par ce
/// backend orienté lecture et renvoient une erreur runtime claire.
#[cfg(feature = "s3")]
pub struct S3Library {
    pub(super) bucket: String,
    pub(super) prefix: String,
}

#[cfg(feature = "s3")]
impl S3Library {
    pub fn new(bucket: impl Into<String>, prefix: impl Into<String>) -> Self {
        S3Library {
            bucket: bucket.into(),
            prefix: prefix.into(),
        }
    }

    /// Parse `s3://bucket/prefix...` en `(bucket, prefix)`. Le schéma `s3://`
    /// (insensible à la casse) est retiré ; le premier segment est le bucket,
    /// le reste (barres de tête/fin retirées) est le préfixe (éventuellement
    /// vide). Renvoie une erreur si le bucket est vide.
    pub fn from_uri(uri: &str) -> Result<Self> {
        let rest = uri
            .strip_prefix("s3://")
            .or_else(|| uri.strip_prefix("S3://"))
            .ok_or_else(|| SasError::runtime(format!("{uri} is not an s3:// URI.")))?;
        let (bucket, prefix) = match rest.split_once('/') {
            Some((b, p)) => (b, p),
            None => (rest, ""),
        };
        if bucket.is_empty() {
            return Err(SasError::runtime(format!(
                "{uri} is missing an S3 bucket name."
            )));
        }
        Ok(S3Library {
            bucket: bucket.to_string(),
            prefix: prefix.trim_matches('/').to_string(),
        })
    }

    /// Construit l'URI `s3://<bucket>/<prefix>/<table>.parquet` (nom de table
    /// en minuscules, comme `DirLibrary`). Le préfixe vide ou bordé de `/` est
    /// normalisé pour éviter les doubles barres.
    pub(super) fn uri(&self, table: &str) -> String {
        let prefix = self.prefix.trim_matches('/');
        let table = table.to_lowercase();
        if prefix.is_empty() {
            format!("s3://{}/{table}.parquet", self.bucket)
        } else {
            format!("s3://{}/{prefix}/{table}.parquet", self.bucket)
        }
    }

    /// `ScanArgsParquet` portant les `CloudOptions` dérivées de l'environnement
    /// pour cet URI. `from_untyped_config(uri, [])` choisit le backend (AWS ici)
    /// d'après le schéma et laisse object_store récupérer région/credentials
    /// depuis les variables d'environnement AWS standard. En cas d'échec de
    /// résolution (rare), on retombe sur des `CloudOptions` par défaut.
    pub(super) fn scan_args(&self, uri: &str) -> ScanArgsParquet {
        let cloud_options =
            polars::prelude::cloud::CloudOptions::from_untyped_config(uri, std::iter::empty::<(String, String)>())
                .ok();
        ScanArgsParquet {
            cloud_options,
            ..Default::default()
        }
    }
}

#[cfg(feature = "s3")]
impl LibraryProvider for S3Library {
    fn list(&self) -> Result<Vec<String>> {
        Err(SasError::runtime(
            "Listing tables in an S3 library is not supported by the cloud scan stub.",
        ))
    }

    fn exists(&self, _table: &str) -> bool {
        // Pas de HEAD object dans le stub : on ne peut pas l'affirmer.
        false
    }

    fn read(&self, table: &str) -> Result<(SasDataset, Vec<String>)> {
        // Même contrat que DirLibrary::read : lecture eager puis coercition au
        // modèle SAS (et notes de conversion) via from_dataframe.
        let df = self.scan(table)?.collect()?;
        SasDataset::from_dataframe(df)
    }

    fn scan(&self, table: &str) -> Result<LazyFrame> {
        // Scan parquet cloud authentique : on passe l'URI `s3://...` tel quel
        // (PlPath/&str), avec les CloudOptions dérivées de l'environnement.
        let uri = self.uri(table);
        let args = self.scan_args(&uri);
        let lf = LazyFrame::scan_parquet(&uri, args)?;
        Ok(lf)
    }

    fn is_cloud(&self) -> bool {
        true
    }

    fn write(&self, _table: &str, _ds: &SasDataset) -> Result<()> {
        Err(SasError::runtime(
            "Writing to an S3 library is not supported yet (read-only cloud scan stub).",
        ))
    }

    fn delete(&self, _table: &str) -> Result<()> {
        Err(SasError::runtime(
            "Deleting from an S3 library is not supported yet (read-only cloud scan stub).",
        ))
    }

    fn rename(&self, _old: &str, _new: &str) -> Result<()> {
        Err(SasError::runtime(
            "Renaming in an S3 library is not supported yet (read-only cloud scan stub).",
        ))
    }
}
