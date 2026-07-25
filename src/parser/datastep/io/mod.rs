//! Statements d'E/S : datasets (SET/MERGE/UPDATE/MODIFY/BY) et texte (INFILE/INPUT/FILE/PUT/DATALINES).

use super::*;
use super::attrs::ident_begins_format;


mod set;
mod infile;
mod input;
mod put;

pub(crate) use set::*;
pub(crate) use infile::*;
pub(crate) use input::*;
pub(crate) use put::*;
