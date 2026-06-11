//! Text-transformation stage that runs BEFORE the lexer.
//!
//! This is the architectural seam where the SAS macro processor will live
//! (phase M8+): `%let`, `&var` resolution, `%macro`/`%mend` expansion.
//! Today only the identity stage exists. The contract is incremental by
//! design: the executor feeds source to the stage and lexes the result
//! block by block, because macro execution can affect downstream source
//! (`%let` evaluated mid-program, `CALL SYMPUT`).

pub trait TextStage {
    /// Transform submitted source text before lexing.
    fn process(&mut self, source: &str) -> String;
}

/// No-op stage used until the macro facility is implemented.
pub struct IdentityMacroStage;

impl TextStage for IdentityMacroStage {
    fn process(&mut self, source: &str) -> String {
        source.to_string()
    }
}
