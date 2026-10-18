//! A problem found in story content: where, what, and whether it stops the content loading.

use serde::Serialize;
use std::fmt;

/// One finding. Errors stop an asset compiling; warnings are reported and the asset still plays.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Problem {
    /// The node, stage or field it is in (`n3`, `stage recon / objective guards`). Empty: the asset itself.
    pub at: String,
    pub message: String,
    pub warning: bool,
}

impl Problem {
    pub fn error(at: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            at: at.into(),
            message: message.into(),
            warning: false,
        }
    }

    pub fn warning(at: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            at: at.into(),
            message: message.into(),
            warning: true,
        }
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = if self.warning { "warning" } else { "error" };
        if self.at.is_empty() {
            write!(f, "{kind}: {}", self.message)
        } else {
            write!(f, "{kind} at {}: {}", self.at, self.message)
        }
    }
}

/// Whether any of `problems` is an error.
#[must_use]
pub fn has_errors(problems: &[Problem]) -> bool {
    problems.iter().any(|p| !p.warning)
}
