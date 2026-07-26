use super::*;

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
pub fn open_resolved(in_ref: &DatasetRef, session: &mut Session) -> Result<SasDataset> {
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
    }
}
