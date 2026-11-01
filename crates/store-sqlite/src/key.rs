//! Keys as SQLite keeps them: a case's [`Key`] is its row id.

use std::path::Path;

use cases_core::Key;

use crate::error::Error;

/// The key a row id names, when a store can give it.
pub(crate) fn from_sql(n: i64) -> Option<Key> {
    u64::try_from(n).ok().and_then(Key::new)
}

/// The key a case's row id in the store at `path` names, or
/// [`Error::Damaged`] when it is one no store gives.
pub(crate) fn from_row(n: i64, path: &Path) -> Result<Key, Error> {
    from_sql(n).ok_or_else(|| Error::Damaged {
        path: path.to_owned(),
        what: format!("a case has the key {n}"),
    })
}

/// A key as a row id.
pub(crate) trait RowId {
    /// The key as a row id. Every key fits: [`Key::new`] allows none above
    /// `i64::MAX`.
    fn sql(self) -> i64;
}

impl RowId for Key {
    fn sql(self) -> i64 {
        i64::try_from(self.get()).unwrap_or(i64::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_its_row_id() {
        assert_eq!(from_sql(0), None);
        assert_eq!(from_sql(-1), None);
        assert_eq!(from_sql(7), Key::new(7));
        assert_eq!(Key::new(Key::MAX).map(RowId::sql), Some(i64::MAX));
        assert_eq!(Key::new(7).map(RowId::sql), Some(7));
        let path = Path::new("cases.db");
        assert_eq!(from_row(7, path).ok(), Key::new(7));
        let Err(Error::Damaged { what, .. }) = from_row(0, path) else {
            panic!("key 0 read");
        };
        assert_eq!(what, "a case has the key 0");
    }
}
