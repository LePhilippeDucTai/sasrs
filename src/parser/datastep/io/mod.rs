//! Statements d'E/S : datasets (SET/MERGE/UPDATE/MODIFY/BY) et texte (INFILE/INPUT/FILE/PUT/DATALINES).

use super::attrs::ident_begins_format;
use super::*;

mod infile;
mod input;
mod put;
mod set;

pub(crate) use infile::*;
pub(crate) use input::*;
pub(crate) use put::*;
pub(crate) use set::*;
