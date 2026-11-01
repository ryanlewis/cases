//! Errors, and the categories the contract compares them by.

use alloc::format;
use alloc::string::String;
use core::fmt;

/// What kind of error an [`Error`] is.
///
/// The categories, and the order in which a row's checks give them, are part
/// of the contract in `spec/vectors/README.md`. Messages are not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Category {
    /// An event type, kind or urgency this version does not know, perhaps
    /// written by a newer cases.
    Unknown,
    /// The author may not write this event.
    Author,
    /// The record does not read as its event's record.
    Malformed,
    /// The record reads, but breaks a rule of its event.
    Invalid,
    /// The case's state does not allow the event, or its kind does not, for
    /// a park of a case that is not stuck or a note on a message. An event
    /// before the open, and a second open, are transitions too.
    Transition,
    /// The case changed since the writer read it: an append whose
    /// `at_revision` is not the case's revision, or an answer whose
    /// `amend_seq` is below the case's.
    Stale,
    /// The case does not fold: no row is a valid open event.
    NoOpen,
}

impl Category {
    /// The category the contract names `name`, such as `no-open`, or `None`
    /// for a name it does not have: the inverse of [`Category::as_str`], for
    /// a store that is sent a refusal by name, such as a client of a hosted
    /// store.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Category> {
        [
            Category::Unknown,
            Category::Author,
            Category::Malformed,
            Category::Invalid,
            Category::Transition,
            Category::Stale,
            Category::NoOpen,
        ]
        .into_iter()
        .find(|category| category.as_str() == name)
    }

    /// The category as the contract names it, such as `no-open`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Category::Unknown => "unknown",
            Category::Author => "author",
            Category::Malformed => "malformed",
            Category::Invalid => "invalid",
            Category::Transition => "transition",
            Category::Stale => "stale",
            Category::NoOpen => "no-open",
        }
    }
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A row the fold skipped, a write the writer refused, or a case that does
/// not fold: the error's category, and a message for people.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    category: Category,
    message: String,
}

impl Error {
    /// The error of `category`, with `message` for people. A store that runs
    /// the writer somewhere else, such as a client of a hosted store, gives
    /// back the refusal it was sent with this.
    #[must_use]
    pub fn new(category: Category, message: impl Into<String>) -> Error {
        Error {
            category,
            message: message.into(),
        }
    }

    pub(crate) fn malformed(message: impl Into<String>) -> Error {
        Error::new(Category::Malformed, message)
    }

    pub(crate) fn invalid(message: impl Into<String>) -> Error {
        Error::new(Category::Invalid, message)
    }

    pub(crate) fn transition(message: impl Into<String>) -> Error {
        Error::new(Category::Transition, message)
    }

    /// The error for a write made at revision `read` to a case now at
    /// `revision`: a [`Stale`](Category::Stale) one. [`Case::append`] gives
    /// it, and so does a store for a write it checks against the case's
    /// revision itself, such as an archive.
    ///
    /// [`Case::append`]: crate::Case::append
    #[must_use]
    pub fn stale(read: u64, revision: u64) -> Error {
        Error::new(
            Category::Stale,
            format!(
                "the case changed since it was read: read at revision {read}, now at {revision}"
            ),
        )
    }

    /// The error's category.
    #[must_use]
    pub fn category(&self) -> Category {
        self.category
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl core::error::Error for Error {}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;

    use super::*;

    #[test]
    fn a_category_displays_as_the_contract_names_it() {
        assert_eq!(Category::NoOpen.to_string(), "no-open");
        assert_eq!(Category::Stale.to_string(), Category::Stale.as_str());
    }

    #[test]
    fn a_category_reads_from_its_name() {
        for name in [
            "unknown",
            "author",
            "malformed",
            "invalid",
            "transition",
            "stale",
            "no-open",
        ] {
            let category = Category::from_name(name).unwrap();
            assert_eq!(category.as_str(), name);
        }
        assert_eq!(Category::from_name("no_open"), None);
        assert_eq!(Category::from_name(""), None);
    }

    #[test]
    fn a_stale_error_names_both_revisions() {
        let error = Error::stale(3, 5);
        assert_eq!(error.category(), Category::Stale);
        assert_eq!(
            error.to_string(),
            "the case changed since it was read: read at revision 3, now at 5"
        );
    }

    #[test]
    fn an_error_displays_as_its_message() {
        assert_eq!(
            Error::invalid("the title is blank").to_string(),
            "the title is blank"
        );
    }
}
