use super::*;

/// `lib.table` reference; libref defaults to WORK when absent.
#[derive(Debug, Clone, PartialEq)]
pub struct DatasetRef {
    pub libref: Option<String>,
    pub name: String,
}

impl DatasetRef {
    /// Display form "WORK.A" used in log NOTEs.
    pub fn display(&self) -> String {
        format!(
            "{}.{}",
            self.libref.as_deref().unwrap_or("WORK").to_uppercase(),
            self.name.to_uppercase()
        )
    }

    pub fn libref_or_work(&self) -> String {
        self.libref.as_deref().unwrap_or("WORK").to_uppercase()
    }
}

/// Options de dataset `(keep=... drop=... rename=(...) where=(...))` (M2).
/// `keep`/`drop` : `None` = option absente (≠ liste vide). `rename` :
/// paires (ancien, nouveau). `where_` : expression filtrante (valide en
/// entrée SET seulement ; en sortie DATA → erreur de compilation).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DatasetOptions {
    pub keep: Option<Vec<String>>,
    pub drop: Option<Vec<String>>,
    pub rename: Vec<(String, String)>,
    pub where_: Option<Expr>,
    /// `in=nom` (M3) : variable automatique temporaire 0/1 indiquant si le
    /// dataset a participé au groupe de clé BY courant d'un MERGE. Valide
    /// uniquement en INPUT de MERGE ; en sortie DATA → erreur de
    /// compilation. Jamais écrite en sortie (comme FIRST./LAST.).
    pub in_: Option<String>,
}

/// Référence de dataset accompagnée de ses options : `lib.a(keep=x y)`.
#[derive(Debug, Clone, PartialEq)]
pub struct DatasetSpec {
    pub dref: DatasetRef,
    pub options: DatasetOptions,
}

/// Options de NIVEAU STATEMENT du `SET` (M16.4), placées APRÈS la liste des
/// datasets : `set a b end=eof nobs=n point=p;`. À distinguer des options de
/// DATASET (`DatasetOptions`, entre parenthèses après chaque référence).
///
/// - `end` : nom d'une variable temporaire automatique (jamais écrite en
///   sortie, comme FIRST./LAST.) mise à 0 pendant l'itération et à 1 lorsque
///   la DERNIÈRE observation du DERNIER dataset a été lue.
/// - `nobs` : nom d'une variable numérique affectée AVANT la boucle au nombre
///   total d'observations (somme sur tous les datasets du SET).
/// - `point` : nom d'une variable numérique d'INDEX (1-based). Sa présence
///   DÉSACTIVE la boucle implicite et l'output implicite : à chaque exécution
///   du SET, l'observation à l'index courant est lue. Index missing/invalide/
///   hors bornes → erreur runtime.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SetOptions {
    pub end: Option<String>,
    pub nobs: Option<String>,
    pub point: Option<String>,
}

impl DatasetSpec {
    /// Spec sans options (helper pour les constructions simples / tests).
    pub fn plain(dref: DatasetRef) -> Self {
        DatasetSpec {
            dref,
            options: DatasetOptions::default(),
        }
    }

    /// Display form "WORK.A" (délégué à `DatasetRef`).
    pub fn display(&self) -> String {
        self.dref.display()
    }

    pub fn libref_or_work(&self) -> String {
        self.dref.libref_or_work()
    }
}
