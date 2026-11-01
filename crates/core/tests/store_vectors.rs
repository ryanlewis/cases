//! Runs the store vectors, `spec/vectors/store/`, against a store kept in
//! memory: the smallest store the trait allows, written from its docs. It
//! shows that the vectors ask only what the trait says, of any store.

mod memory;

use cases_core::Time;
use cases_vectors::Harness;
use libtest_mimic::Arguments;

use memory::{Fault, Memory};

impl Harness for Memory {
    type Store = Memory;

    fn store(&self) -> &Memory {
        self
    }

    fn set_clock(&self, now: Time) {
        self.now.set(now);
    }

    fn category(&self, error: &Fault) -> Option<&'static str> {
        Some(match error {
            Fault::Missing => "missing",
            Fault::Unnamed | Fault::Doing(_) => "invalid",
            Fault::NotListening => "not-listening",
        })
    }

    const LOCAL: bool = true;
}

fn main() {
    let args = Arguments::from_args();
    libtest_mimic::run(&args, cases_vectors::trials(Memory::new)).exit();
}
