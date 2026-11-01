//! Runs the store vectors, `spec/vectors/store/`, against a forwarded
//! store: through the pipe to `cases dial-stdio`, which serves them from a
//! store file of its own. A forwarded store keeps the local listening
//! record, so the vectors marked `local` run too. Each runs three times:
//! with `dial-stdio` as the child, as for ssh, run by `sh -c`, as for an
//! `exec:` forward, and over HTTP to `cases serve` on loopback, as for an
//! `http://` forward.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod far;

use cases_core::Time;
use cases_store_remote::{Error, RemoteStore};
use cases_vectors::Harness;
use far::{Far, Over};
use libtest_mimic::Arguments;

impl Harness for Far {
    type Store = RemoteStore;

    fn store(&self) -> &RemoteStore {
        &self.store
    }

    fn set_clock(&self, now: Time) {
        Far::set_clock(self, now);
    }

    fn category(&self, error: &Error) -> Option<&'static str> {
        match error {
            Error::Invalid(_) => Some("invalid"),
            Error::NotListening => Some("not-listening"),
            Error::Missing => Some("missing"),
            _ => None,
        }
    }

    const LOCAL: bool = true;
}

fn main() {
    let args = Arguments::from_args();
    let mut trials = cases_vectors::trials(Far::new);
    trials.extend(
        cases_vectors::trials(|now| Far::over(now, Over::Exec))
            .into_iter()
            .map(|trial| trial.with_kind("exec")),
    );
    trials.extend(
        cases_vectors::trials(|now| Far::over(now, Over::Http))
            .into_iter()
            .map(|trial| trial.with_kind("http")),
    );
    libtest_mimic::run(&args, trials).exit();
}
