//! Scrollback search: state and the matcher.  Drawing and key handling live
//! in `x.rs`, history access in `term.rs`.

use crate::app::Rune;

#[derive(Clone, Debug, Default)]
pub struct Search {
    pub query: Vec<Rune>,
}
