use super::*;

/// J02-P9 — fenêtre OBS=/FIRSTOBS= d'une lecture de PROC.
///
/// Doc SAS 9.4, System Options (OBS=, FIRSTOBS=) : ces options système
/// s'appliquent à TOUTE lecture d'une table par un PROC (procédure), pas
/// seulement aux SET/MERGE de l'étape DATA. Elles étaient appliquées au seul
/// DATA step (`datastep::compile_io`) et ignorées en silence par les PROC.
///
/// `firstobs`/`obs` viennent de `session.options` (défauts 1 / MAX = aucune
/// fenêtre) ; `stored_n` est le compte d'observations de la table LUE. Rend
/// `(start, end)` : indices 0-based bornés par `stored_n`, `end >= start`.
fn obs_window(session: &Session, stored_n: usize) -> (usize, usize) {
    let n = stored_n;
    let start = session.options.firstobs.saturating_sub(1).min(n);
    let end = session.options.obs.map_or(n, |o| o.min(n)).max(start);
    (start, end)
}

/// J02-P9 — découpe `ds` sur la fenêtre OBS=/FIRSTOBS= en vigueur.
///
/// Sans fenêtre (firstobs=1, obs=None), `ds` est rendu TEL QUEL (pas de copie
/// polars superflue). Une fenêtre tronque le `DataFrame` par UN `take` sur
/// tous les indices conservés, `vars` (métadonnées) inchangés.
fn apply_obs_window(mut ds: SasDataset, session: &Session) -> SasDataset {
    let n = ds.n_obs();
    let (start, end) = obs_window(session, n);
    if start != 0 || end != n {
        let idx: Vec<u32> = (start as u32..end as u32).collect();
        let idx_ca = polars::prelude::IdxCa::from_vec("".into(), idx);
        if let Ok(sliced) = ds.df.take(&idx_ca) {
            ds.df = sliced;
        }
    }
    ds
}

/// Résout `data=` ou `_LAST_` en un `DatasetRef` concret. Bloc identique
/// utilisé par `print.rs`, `sort.rs`, `means.rs` (`resolve_input`) : la chaîne
/// `_LAST_` a la forme « LIBREF.NAME » et est décodée via `splitn(2, '.')`.
/// Forme verbatim de la copie canonique (aucune divergence connue entre procs).
pub fn resolve_last_dataset(data: &Option<DatasetRef>, session: &Session) -> Result<DatasetRef> {
    match data {
        Some(r) => Ok(r.clone()),
        None => {
            let last = session.last_dataset.clone().ok_or_else(|| {
                SasError::runtime("There is no default input data set (_LAST_ is undefined).")
            })?;
            let parts: Vec<&str> = last.splitn(2, '.').collect();
            if parts.len() == 2 {
                Ok(DatasetRef {
                    libref: Some(parts[0].to_string()),
                    name: parts[1].to_string(),
                })
            } else {
                Ok(DatasetRef {
                    libref: None,
                    name: last,
                })
            }
        }
    }
}

/// Résout `DATA=`/`_LAST_`, lit la table et relaie ses notes de lecture au
/// log. N'émet PAS la NOTE « observations read » (sa position dans le log
/// varie selon la proc). Renvoie (dataset, libref, table en MAJUSCULES).
///
/// J02-P9 — la fenêtre OBS=/FIRSTOBS= de la session est appliquée à la table
/// lue (doc SAS 9.4, System Options : valables pour toute lecture d'un PROC).
pub fn open_input(
    data: &Option<DatasetRef>,
    session: &mut Session,
) -> Result<(SasDataset, String, String)> {
    let in_ref = resolve_last_dataset(data, session)?;
    let ds = open_resolved(&in_ref, session)?;
    Ok((ds, in_ref.libref_or_work(), in_ref.name.to_uppercase()))
}

/// Comme [`open_input`] mais renvoie le nom d'affichage `LIBREF.TABLE`
/// (cf. [`DatasetRef::display`]) au lieu du couple (libref, table) — la forme
/// dont ont besoin les procs qui n'utilisent libref/table que pour la NOTE
/// « observations read » et les messages d'erreur (MQ6.1).
///
/// J02-P9 — OBS=/FIRSTOBS= appliqués comme pour [`open_input`].
pub fn open_input_display(
    data: &Option<DatasetRef>,
    session: &mut Session,
) -> Result<(SasDataset, String)> {
    let in_ref = resolve_last_dataset(data, session)?;
    let ds = open_resolved(&in_ref, session)?;
    Ok((ds, in_ref.display()))
}

/// Lit la table désignée par un [`DatasetRef`] déjà résolu et relaie ses
/// notes de lecture au log (MQ6.1). Pour les procs qui gardent le
/// `DatasetRef` d'entrée (p.ex. comme défaut de `OUT=`) : résoudre via
/// [`resolve_last_dataset`] puis appeler ceci.
///
/// J02-P9 — la fenêtre OBS=/FIRSTOBS= de la session est appliquée à la table
/// lue : les options système valent pour toute lecture d'un PROC (doc SAS
/// 9.4, System Options, OBS= et FIRSTOBS=), elles n'étaient honorées que par
/// l'étape DATA. Les procs de MÉTADONNÉES qui doivent rapporter le compte
/// stocké (PROC CONTENTS) passent par [`open_resolved_full`].
pub fn open_resolved(in_ref: &DatasetRef, session: &mut Session) -> Result<SasDataset> {
    let ds = open_resolved_full(in_ref, session)?;
    Ok(apply_obs_window(ds, session))
}

/// Comme [`open_resolved`] SANS la fenêtre OBS=/FIRSTOBS= : la table entière
/// est lue. Réservé aux usages de métadonnées (PROC CONTENTS, PROC DATASETS)
/// qui doivent rapporter le compte d'observations STOCKÉ, indépendamment de
/// la fenêtre de lecture.
pub fn open_resolved_full(in_ref: &DatasetRef, session: &mut Session) -> Result<SasDataset> {
    let in_libref = in_ref.libref_or_work();
    let in_table = in_ref.name.to_uppercase();
    let provider = session.libs.get(&in_libref)?;
    let (ds, notes) = provider.read(&in_table)?;
    for note in notes {
        session.log.forward(&note);
    }
    Ok(ds)
}

/// Métadonnée d'une variable NUMÉRIQUE de sortie : longueur SAS 8, ni format
/// ni label. C'est la forme par défaut de toute colonne numérique créée par
/// une PROC (MQ8.1 — unifie 11 copies identiques).
pub fn num_var_meta(name: &str) -> VarMeta {
    VarMeta {
        name: name.to_string(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
        informat: None,
    }
}

/// Métadonnée d'une variable CARACTÈRE de sortie de longueur `length`, ni
/// format ni label (MQ8.1 — unifie 5 copies identiques). `length` est repris
/// tel quel : à l'appelant de garantir `>= 1` s'il calcule une largeur.
pub fn char_var_meta(name: &str, length: usize) -> VarMeta {
    VarMeta {
        name: name.to_string(),
        ty: VarType::Char,
        length,
        format: None,
        label: None,
        informat: None,
    }
}
