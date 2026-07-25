use super::*;

pub enum Block {
    Global(GlobalStmt),
    DataStep(DataStepAst),
    Proc { name: String, ast: ProcAst },
    /// `run;` isolé ou statement vide : écho dans le log, aucune action.
    Empty,
}

/// `title` → 1, `title3` → 3 ; `None` si ce n'est pas un mot-clé TITLEn.
pub(crate) fn title_level(name: &str) -> Option<u8> {
    level_after_prefix(name, "title")
}

/// `footnote` → 1, `footnote3` → 3 ; `None` si ce n'est pas un mot-clé FOOTNOTEn.
pub(crate) fn footnote_level(name: &str) -> Option<u8> {
    level_after_prefix(name, "footnote")
}

/// `prefix` → 1, `prefix3` → 3 (3 = chiffre 1..9) ; `None` sinon.
pub(super) fn level_after_prefix(name: &str, prefix: &str) -> Option<u8> {
    let lower = name.to_ascii_lowercase();
    let rest = lower.strip_prefix(prefix)?;
    match rest {
        "" => Some(1),
        _ if rest.len() == 1 && rest.as_bytes()[0].is_ascii_digit() && rest != "0" => {
            rest.parse().ok()
        }
        _ => None,
    }
}

/// Mot-clé qui ouvre un bloc (frontière de step implicite). Les statements
/// globaux sont des frontières de step en SAS, au même titre que DATA/PROC.
pub(super) fn is_block_head_kw(lower: &str) -> bool {
    matches!(lower, "data" | "proc" | "libname" | "filename" | "options" | "ods")
        || title_level(lower).is_some()
        || footnote_level(lower).is_some()
}

pub(super) fn validate_sas_name(name: &str, span: Span) -> Result<()> {
    if name.len() > 32 {
        return Err(SasError::parse(
            format!("The name {} exceeds the SAS maximum of 32 characters.", name.to_uppercase()),
            span,
        ));
    }
    Ok(())
}
