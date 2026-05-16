//! Random-byte abstraction. The library injects `Rng` so that tests can
//! produce deterministic ULIDs (Principle VI).

/// Fill the destination slice with random bytes.
///
/// `OsRng` is the default implementation (uses `getrandom`). Tests construct a
/// seeded PRNG and pass it to `Store::open_with`.
pub trait Rng: Send + Sync {
    /// Fill `dst` with random bytes.
    fn fill_bytes(&mut self, dst: &mut [u8]);
}

/// Default `Rng` implementation backed by the operating system.
#[derive(Debug, Default, Clone, Copy)]
pub struct OsRng;

impl Rng for OsRng {
    fn fill_bytes(&mut self, dst: &mut [u8]) {
        // ulid 1.x's internal RNG already uses `rand`'s OsRng; we re-implement
        // the same primitive here so callers don't need to depend on `rand`
        // directly. Falling back to `getrandom` keeps the dependency surface
        // small (getrandom is already a transitive dep of ulid).
        getrandom::getrandom(dst).expect("OS RNG failed");
    }
}
