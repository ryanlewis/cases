//! Runs the store vectors, `spec/vectors/store/`, against a remote store
//! over HTTP: each vector has a store of its own on the fake server, with a
//! clock the vector sets. A remote store keeps no local listening record,
//! so the vectors marked `local` show as ignored.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod fake;

use std::sync::OnceLock;

use cases_core::Time;
use cases_store_remote::{Client, Error, RemoteStore};
use cases_vectors::Harness;
use fake::{Fake, TOKEN};
use libtest_mimic::Arguments;

/// The one fake server the vectors share, each with a store of its own.
fn fake() -> &'static Fake {
    static FAKE: OnceLock<Fake> = OnceLock::new();
    FAKE.get_or_init(|| Fake::start_kept("store_vectors"))
}

/// A store of its own on the fake.
struct Remote {
    id: String,
    store: RemoteStore,
}

impl Remote {
    fn new(start: Time) -> Remote {
        let fake = fake();
        let id = fake.add_store("vector");
        fake.set_clock(&id, start);
        let client = Client::new(fake.url().parse().unwrap(), TOKEN.parse().unwrap());
        let store = client.store(&id).unwrap();
        Remote { id, store }
    }
}

impl Harness for Remote {
    type Store = RemoteStore;

    fn store(&self) -> &RemoteStore {
        &self.store
    }

    fn set_clock(&self, now: Time) {
        fake().set_clock(&self.id, now);
    }

    fn category(&self, error: &Error) -> Option<&'static str> {
        match error {
            Error::Invalid(_) => Some("invalid"),
            Error::NotListening => Some("not-listening"),
            _ => None,
        }
    }

    const LOCAL: bool = false;
}

fn main() {
    let args = Arguments::from_args();
    libtest_mimic::run(&args, cases_vectors::trials(Remote::new)).exit();
}
