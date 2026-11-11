//! Errors, and the status the process exits with for each (CL-01).

use std::fmt;
use std::io;
use std::process::ExitCode;

use cases_core::{Category, StoreError};

use crate::store::Failure;

/// What kind of failure an [`Error`] is, which the exit status tells an agent
/// without it reading stderr.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    /// Anything else: bad input, an unknown case, a store that fails or is
    /// locked. Exit 1.
    Failure,
    /// `wait` gave up because nothing came. Exit 2.
    Timeout,
    /// The case's state does not allow the event, and nothing was written.
    /// Exit 3.
    Transition,
    /// The case changed since the revision the write was made at, and
    /// nothing was written. Exit 4.
    Stale,
}

impl Status {
    /// The status the process exits with.
    pub(crate) fn code(self) -> u8 {
        match self {
            Status::Failure => 1,
            Status::Timeout => 2,
            Status::Transition => 3,
            Status::Stale => 4,
        }
    }
}

/// Why a command failed: a message for stderr, and the status to exit with.
#[derive(Debug)]
pub(crate) struct Error {
    status: Status,
    message: String,
}

impl Error {
    /// A failure, exit 1, with `message`.
    pub(crate) fn new(message: impl Into<String>) -> Error {
        Error::with_status(Status::Failure, message)
    }

    /// An error with its own status.
    pub(crate) fn with_status(status: Status, message: impl Into<String>) -> Error {
        Error {
            status,
            message: message.into(),
        }
    }

    /// The error's status.
    pub(crate) fn status(&self) -> Status {
        self.status
    }

    /// Prints the error to stderr and gives the status to exit with. A
    /// timeout is not a fault, so it is printed as it is; everything else
    /// after `Error: `. An error with no message prints nothing.
    pub(crate) fn report(&self) -> ExitCode {
        match self.status {
            _ if self.message.is_empty() => {}
            Status::Timeout => eprintln!("{}", self.message),
            _ => eprintln!("Error: {}", self.message),
        }
        ExitCode::from(self.status.code())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// A refusal from the core exits by its category: 3 for a transition, 4 for
/// a stale write, and 1 for the rest.
impl From<cases_core::Error> for Error {
    fn from(error: cases_core::Error) -> Error {
        let status = match error.category() {
            Category::Transition => Status::Transition,
            Category::Stale => Status::Stale,
            _ => Status::Failure,
        };
        Error::with_status(status, error.to_string())
    }
}

impl From<StoreError<cases_store_sqlite::Error>> for Error {
    fn from(error: StoreError<cases_store_sqlite::Error>) -> Error {
        match error {
            StoreError::Refused(error) => error.into(),
            StoreError::Store(error) => error.into(),
            other => Error::new(other.to_string()),
        }
    }
}

/// A write that a remote store gave no answer to may or may not have been
/// stored, and the error says so.
impl From<StoreError<Failure>> for Error {
    fn from(error: StoreError<Failure>) -> Error {
        match error {
            StoreError::Refused(error) => error.into(),
            StoreError::Store(error) if error.no_answer() => Error::new(format!(
                "{error}; the write may or may not have been stored: check with cases show"
            )),
            StoreError::Store(error) => error.into(),
            other => Error::new(other.to_string()),
        }
    }
}

impl From<Failure> for Error {
    fn from(error: Failure) -> Error {
        Error::new(error.to_string())
    }
}

impl From<cases_store_sqlite::Error> for Error {
    fn from(error: cases_store_sqlite::Error) -> Error {
        Error::new(error.to_string())
    }
}

/// Stdout closed early, as `cases list | head` does, is a quiet failure:
/// the reader has what it wanted, and there is nowhere to say more.
impl From<io::Error> for Error {
    fn from(error: io::Error) -> Error {
        if error.kind() == io::ErrorKind::BrokenPipe {
            return Error::new("");
        }
        Error::new(error.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Error {
        if error.io_error_kind() == Some(io::ErrorKind::BrokenPipe) {
            return Error::new("");
        }
        Error::new(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use cases_core::{Append, Author, Case, Open, Pickup, Record, Time};

    use super::*;

    fn now() -> Time {
        Time::from_unix(1_790_000_000, 0).unwrap()
    }

    /// An open fyi case.
    fn fyi() -> Case {
        let mut open = Open::default();
        open.kind = "fyi".into();
        open.urgency = "today".into();
        open.title = "Nightly build moved".into();
        Case::open(
            &Append::new(Author::Agent, &Record::Open(open)),
            now(),
            None,
        )
        .unwrap()
        .0
    }

    fn pickup() -> Append {
        Append::new(Author::Agent, &Record::Pickup(Pickup::default()))
    }

    #[test]
    fn a_refused_transition_exits_3() {
        let error = fyi().append(&pickup(), now(), None, None).unwrap_err();
        let error = Error::from(StoreError::<cases_store_sqlite::Error>::Refused(error));
        assert_eq!(error.status(), Status::Transition);
        assert_eq!(error.status().code(), 3);
    }

    #[test]
    fn a_stale_write_exits_4() {
        let error = fyi().append(&pickup(), now(), None, Some(5)).unwrap_err();
        assert_eq!(Error::from(error).status().code(), 4);
    }

    #[test]
    fn every_other_error_exits_1() {
        let mut open = Open::default();
        open.kind = "poll".into();
        let unknown = Case::open(
            &Append::new(Author::Agent, &Record::Open(open)),
            now(),
            None,
        )
        .unwrap_err();
        assert_eq!(Error::from(unknown).status().code(), 1);
        let missing = cases_store_sqlite::Error::Missing {
            path: "cases.db".into(),
        };
        assert_eq!(Error::from(missing).status().code(), 1);
        let no_case = StoreError::<cases_store_sqlite::Error>::NoCase;
        assert_eq!(Error::from(no_case).status().code(), 1);
        assert_eq!(Error::new("bad input").status().code(), 1);
        assert_eq!(Status::Timeout.code(), 2);
    }

    #[test]
    fn a_broken_pipe_fails_quietly() {
        let pipe = Error::from(io::Error::from(io::ErrorKind::BrokenPipe));
        assert_eq!(
            (pipe.to_string().as_str(), pipe.status()),
            ("", Status::Failure)
        );
        let other = Error::from(io::Error::from(io::ErrorKind::PermissionDenied));
        assert!(!other.to_string().is_empty());
    }
}
