//! The poller: every case in a store, folded again only where it changed.

use alloc::vec::Vec;

use crate::{Case, Cases, Changes, Key, Store, Version};

/// A read of what changed since a version, as [`Store::rows_since`] is.
type Read<S> = fn(&S, Option<&Version>) -> Result<Changes<Key>, <S as Store>::Error>;

/// Reads a store over and over, as `cases wait` and `cases serve` do, and
/// folds again only the cases that changed.
///
/// A poll asks the store for what changed since the version the poller
/// last read, as [`Store::rows_since`] gives it, so what it gives is one
/// moment of the store. When nothing changed, it gives the cases it has.
/// Otherwise it folds the cases changed and drops those removed, or, when
/// the store gives every case, it folds them all. After a poll that fails,
/// the next one asks for every case.
///
/// ```
/// # use cases_core::{Key, Poller, Store};
/// fn count<S: Store<Key = Key>>(store: &S) -> Result<usize, S::Error> {
///     let mut poller = Poller::new(store);
///     Ok(poller.poll()?.len())
/// }
/// ```
#[derive(Debug)]
pub struct Poller<'a, S: Store<Key = Key>> {
    store: &'a S,
    /// How the poller reads what changed: [`Store::rows_since`], or the
    /// store's own read.
    read: Read<S>,
    /// The version the cases were read at: `None` before the first read,
    /// and after a read that failed.
    version: Option<Version>,
    cases: Cases,
}

impl<'a, S: Store<Key = Key>> Poller<'a, S> {
    /// A poller over `store`, which reads what changed with
    /// [`Store::rows_since`].
    #[must_use]
    pub fn new(store: &'a S) -> Self {
        Self::with_read(store, S::rows_since)
    }

    /// A poller over `store`, which reads what changed with `read` in place
    /// of [`Store::rows_since`], such as a store's own read that gives an
    /// error for a store not made yet, where `rows_since` gives no cases.
    /// `read` answers as `rows_since` does.
    #[must_use]
    pub fn with_read(store: &'a S, read: Read<S>) -> Self {
        Poller {
            store,
            read,
            version: None,
            cases: Cases::new(),
        }
    }

    /// The version the cases were last read at: `None` before the first
    /// poll, and after a poll that failed. A store that can wait for a
    /// change, such as a held poll, waits from this version.
    #[must_use]
    pub fn version(&self) -> Option<&Version> {
        self.version.as_ref()
    }

    /// The cases the last poll gave, read at [`version`](Self::version),
    /// without reading the store: none before the first poll, and after a
    /// poll that failed.
    #[must_use]
    pub fn cases(&self) -> &Cases {
        &self.cases
    }

    /// Every case in the store, as [`Store::list`] gives them, each folded,
    /// from what the poller has where nothing has changed.
    ///
    /// # Errors
    ///
    /// Gives the store's error when the read fails. The next poll then
    /// reads every case again.
    pub fn poll(&mut self) -> Result<&Cases, S::Error> {
        let changes = match (self.read)(self.store, self.version.as_ref()) {
            Ok(changes) => changes,
            Err(error) => {
                self.version = None;
                self.cases.clear();
                return Err(error);
            }
        };
        // Folded once the read is done, so a store that reads under a lock
        // does not hold it while the cases fold.
        let (version, changed) = match changes {
            Changes::Unchanged { version } => (version, Vec::new()),
            Changes::Since {
                version,
                cases,
                removed,
            } => {
                for key in removed {
                    self.cases.remove(&key);
                }
                (version, cases)
            }
            Changes::All { version, cases } => {
                self.cases.clear();
                (version, cases)
            }
        };
        for (key, rows) in changed {
            self.cases.insert(key, Case::fold(&rows));
        }
        self.version = Some(version);
        Ok(&self.cases)
    }
}
