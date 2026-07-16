//! Toy deterministic guest: FNV-1a register + BLAKE3 chain.
//!
//! Chain widening decision (plan 00-overview grounding note 1): the chain is
//! seeded `blake3("mock-statehash-v1" ‖ machine_config_hash ‖
//! base_snapshot_ref)` and folded per epoch as `blake3(prev ‖
//! fnv_register_le)` — the same *shape* as the hypervisor's chained state
//! hash (its ARCHITECTURE §8.5), never its values. No spec pins the mock's
//! internals; the committed fixtures freeze this choice.

/// Epoch length of the toy guest, in icounts. Small on purpose: fixture
/// segments carry multiple `EPOCH_HASH` records so M2's native bisection has
/// something to binary-search.
pub const MOCK_EPOCH_ICOUNTS: u64 = 4096;

/// Bytes folded into the register by the fixture-time `RecordedSkew` defect.
/// Shared constant so M2's bisection oracle re-simulates the exact same
/// recorded execution the xtask fixture generator produced.
pub const SKEW_FOLD: &[u8] = b"mock-recorded-skew-v1";

/// Injectable defect, verbatim from IMPLEMENTATION-PLAN §M2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InjectedDefect {
    None,
    /// Run-dependent value folded in from that icount on.
    ReplayNondet {
        segment: u32,
        at_icount: u64,
        flake_period: u32,
    },
    /// Fixture hashes computed WITH the skew, replay computes WITHOUT
    /// ⇒ recorded-divergence.
    RecordedSkew {
        segment: u32,
        at_icount: u64,
    },
}

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// The toy guest. Deterministic by construction: state is a pure function of
/// (machine_config_hash, base_snapshot_ref, the fold calls made in order).
#[derive(Clone, Debug)]
pub struct GuestSim {
    fnv: u64,
    chain: [u8; 32],
    icount: u64,
}

impl GuestSim {
    pub fn new(machine_config_hash: &[u8; 32], base_snapshot_ref: &[u8; 32]) -> Self {
        let mut h = blake3::Hasher::new();
        h.update(b"mock-statehash-v1");
        h.update(machine_config_hash);
        h.update(base_snapshot_ref);
        GuestSim {
            fnv: FNV_OFFSET_BASIS,
            chain: *h.finalize().as_bytes(),
            icount: 0,
        }
    }

    pub fn icount(&self) -> u64 {
        self.icount
    }

    /// The raw FNV state register (M2's mock synthesizes deterministic
    /// register diffs from it).
    pub fn register(&self) -> u64 {
        self.fnv
    }

    /// Chain value after the most recent epoch-boundary fold (the payload of
    /// an `EPOCH_HASH` record at that boundary).
    pub fn epoch_chain(&self) -> [u8; 32] {
        self.chain
    }

    /// Advance to `target`, folding the chain at every epoch boundary
    /// crossed or landed on (boundary b: `old_icount < b <= target`,
    /// b a multiple of [`MOCK_EPOCH_ICOUNTS`]). Events scheduled exactly at
    /// a boundary are applied AFTER that boundary's fold (drivers call
    /// `step_to(ev.icount)` then `apply_event`).
    pub fn step_to(&mut self, target: u64) {
        assert!(
            target >= self.icount,
            "GuestSim cannot step backwards ({} -> {})",
            self.icount,
            target
        );
        let mut next = (self.icount / MOCK_EPOCH_ICOUNTS + 1) * MOCK_EPOCH_ICOUNTS;
        while next <= target {
            let mut h = blake3::Hasher::new();
            h.update(&self.chain);
            h.update(&self.fnv.to_le_bytes());
            self.chain = *h.finalize().as_bytes();
            next += MOCK_EPOCH_ICOUNTS;
        }
        self.icount = target;
    }

    /// FNV-1a fold of arbitrary bytes into the state register.
    pub fn fold_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.fnv ^= u64::from(b);
            self.fnv = self.fnv.wrapping_mul(FNV_PRIME);
        }
    }

    /// State hash at the current icount: `blake3(chain ‖ fnv_le ‖
    /// icount_le)`. Well-defined at any icount (H3 exact-stop probes), and
    /// equals the recorded `end_state_hash` when probed at `end_icount`.
    pub fn chain_value(&self) -> [u8; 32] {
        let mut h = blake3::Hasher::new();
        h.update(&self.chain);
        h.update(&self.fnv.to_le_bytes());
        h.update(&self.icount.to_le_bytes());
        *h.finalize().as_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_value_is_deterministic_across_instances() {
        let mcfg = [7u8; 32];
        let base = [9u8; 32];
        let run = |skew: bool| {
            let mut sim = GuestSim::new(&mcfg, &base);
            sim.step_to(100);
            sim.fold_bytes(b"pad");
            sim.step_to(5000);
            if skew {
                sim.fold_bytes(SKEW_FOLD);
            }
            sim.step_to(9000);
            sim.chain_value()
        };
        assert_eq!(run(false), run(false));
        assert_eq!(run(true), run(true));
        assert_ne!(run(false), run(true));
    }

    #[test]
    fn epoch_fold_happens_exactly_at_boundaries() {
        let mut a = GuestSim::new(&[0; 32], &[0; 32]);
        let mut b = a.clone();
        // No boundary between 1 and 4095: epoch chains stay equal.
        a.step_to(1);
        b.step_to(4095);
        assert_eq!(a.epoch_chain(), b.epoch_chain());
        // Landing exactly on 4096 folds.
        b.step_to(4096);
        assert_ne!(a.epoch_chain(), b.epoch_chain());
        // ...and only once for the whole epoch.
        let folded = b.epoch_chain();
        b.step_to(8191);
        assert_eq!(folded, b.epoch_chain());
    }

    #[test]
    fn chain_value_differs_by_icount_even_without_folds() {
        let mut a = GuestSim::new(&[0; 32], &[0; 32]);
        let mut b = a.clone();
        a.step_to(10);
        b.step_to(11);
        assert_ne!(a.chain_value(), b.chain_value());
    }
}
