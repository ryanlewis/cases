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
    /// What the last poll folded again and dropped.
    refolded: Refolded,
}

/// What a poll did to the cases it gives, as [`Poller::refolded`] says.
///
/// It is exact: a case is named when the poll folded its rows again, even
/// when it folded to the same case as before, and a case not named is the
/// very case the poll before gave, not folded again. So a caller that keeps
/// something for each case, made from the cases one poll gave, can make it
/// again for the cases named alone, and keep the rest.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Refolded {
    /// Every case the poll gives was folded again, and any case the poll
    /// before gave and this one does not is gone. So says the poller before
    /// its first poll, after it, after a poll the store answered with every
    /// case (as it does for a store it cannot tell the changes of, such as
    /// another put at its path), after a poll that failed, and after the
    /// poll that follows a failed one.
    All,
    /// Only the cases in `folded` were folded again, in the order the store
    /// gave them, and only the cases in `removed`, which the poll before
    /// gave, were dropped. Every other case is as the poll before gave it.
    /// A key is in one list at most.
    #[non_exhaustive]
    Changed {
        /// The key of each case whose rows the poll read and folded: a case
        /// written to since, or opened since.
        folded: Vec<Key>,
        /// The key of each case the poll before gave and this one does not:
        /// a case archived or deleted since.
        removed: Vec<Key>,
    },
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
            refolded: Refolded::All,
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

    /// What the last poll folded again and dropped, against the cases the
    /// poll before gave: see [`Refolded`]. [`Refolded::All`] before the
    /// first poll.
    #[must_use]
    pub fn refolded(&self) -> &Refolded {
        &self.refolded
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
                self.refolded = Refolded::All;
                return Err(error);
            }
        };
        // Every case is folded again on a first read, as on one after a
        // failed poll: the store gives every case when asked since no
        // version, and so a reader with no version has none to keep.
        let first = self.version.is_none();
        // Folded once the read is done, so a store that reads under a lock
        // does not hold it while the cases fold.
        let mut gone = Vec::new();
        let mut all = first;
        let (version, changed) = match changes {
            Changes::Unchanged { version } => (version, Vec::new()),
            Changes::Since {
                version,
                cases,
                removed,
            } => {
                for key in removed {
                    if self.cases.remove(&key).is_some() {
                        gone.push(key);
                    }
                }
                (version, cases)
            }
            Changes::All { version, cases } => {
                self.cases.clear();
                all = true;
                (version, cases)
            }
        };
        let mut folded = Vec::with_capacity(changed.len());
        for (key, rows) in changed {
            self.cases.insert(key, Case::fold(&rows));
            folded.push(key);
        }
        // A case taken out and opened again under its key, which a store
        // does not do, is folded again rather than gone.
        gone.retain(|key| !folded.contains(key));
        self.refolded = if all {
            Refolded::All
        } else {
            Refolded::Changed {
                folded,
                removed: gone,
            }
        };
        self.version = Some(version);
        Ok(&self.cases)
    }
}
