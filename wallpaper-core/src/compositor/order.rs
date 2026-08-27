//! The order a playlist is played in.
//!
//! A playlist is a list of files and a position in it. Shuffling could have
//! been "pick a random item every time it advances", and that is what makes
//! shuffle feel broken everywhere it is done that way: with four wallpapers
//! it shows the same one twice in a row roughly a quarter of the time, and a
//! user watching their desktop repeat itself concludes the setting does not
//! work.
//!
//! So the order is a permutation of the whole list, played through once and
//! drawn again at the end — every wallpaper gets its turn before any of them
//! gets a second one. The one join worth guarding is where two passes meet,
//! which is the only place a repeat can still happen; `shuffled` is told
//! which item just played and keeps it off the front.
//!
//! No dependency for the randomness. A wallpaper deciding what to show next
//! is not cryptography, and a crate for it would be a crate in the binary
//! every user downloads. The generator below is eight lines and its quality
//! is far past what picking between four files asks of it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// `0..len`, which is a playlist played in the order the user wrote it.
pub fn straight(len: usize) -> Vec<usize> {
    (0..len).collect()
}

/// A permutation of `0..len` that does not begin with `avoid`.
///
/// `avoid` is the item that just finished playing. Keeping it off the front
/// is what stops the same wallpaper appearing twice across the join between
/// one pass through the list and the next. With one item there is nothing to
/// avoid and nothing to shuffle, so it is returned as it is.
pub fn shuffled(len: usize, avoid: Option<usize>, seed: u64) -> Vec<usize> {
    let mut order = straight(len);
    if len < 2 {
        return order;
    }

    // Fisher-Yates, from the back: every permutation equally likely, one
    // pass, no allocation beyond the list itself.
    let mut rng = Rng(seed | 1);
    for i in (1..len).rev() {
        let j = (rng.next() % (i as u64 + 1)) as usize;
        order.swap(i, j);
    }

    // The join. Swapping the front with a later slot keeps the result a
    // permutation, which drawing a fresh number until it differs would also
    // do but not in bounded time.
    if let Some(avoid) = avoid {
        if order[0] == avoid {
            let other = 1 + (rng.next() % (len as u64 - 1)) as usize;
            order.swap(0, other);
        }
    }

    order
}

/// A seed for one shuffle.
///
/// The clock, so two machines starting together do not share an order, and a
/// counter, so two shuffles drawn in the same instant do not either — which
/// is what happens at startup, when every screen with the same playlist on it
/// is set up inside the same tick of the clock.
pub fn seed() -> u64 {
    static DRAWN: AtomicU64 = AtomicU64::new(0);

    let clock = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_nanos() as u64)
        .unwrap_or(0x2545_F491_4F6C_DD1D);

    // Multiplied by an odd constant rather than added, so a counter that is
    // still small moves more than the low bits of the clock.
    clock
        ^ DRAWN
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// xorshift64. Not for anything that matters, and this does not.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_permutation(order: &[usize], len: usize) -> bool {
        let mut seen = vec![false; len];
        for index in order {
            if *index >= len || seen[*index] {
                return false;
            }
            seen[*index] = true;
        }
        order.len() == len
    }

    #[test]
    fn a_straight_order_is_the_list_as_written() {
        assert_eq!(straight(4), vec![0, 1, 2, 3]);
        assert!(straight(0).is_empty());
    }

    /// The property that matters: every wallpaper gets its turn, exactly
    /// once, before any of them gets a second one.
    #[test]
    fn every_item_appears_exactly_once() {
        for len in 1..24usize {
            for seed in 1..40u64 {
                let order = shuffled(len, None, seed * 7919);
                assert!(
                    is_permutation(&order, len),
                    "len {len} seed {seed} gave {order:?}"
                );
            }
        }
    }

    /// The join between two passes is the only place a repeat can survive
    /// the bag, so it is the one worth a test.
    #[test]
    fn the_item_that_just_played_never_starts_the_next_pass() {
        for len in 2..24usize {
            for avoid in 0..len {
                for seed in 1..40u64 {
                    let order = shuffled(len, Some(avoid), seed * 104_729);
                    assert_ne!(order[0], avoid, "len {len} avoid {avoid} seed {seed}");
                    assert!(is_permutation(&order, len));
                }
            }
        }
    }

    /// One item is a playlist with nothing to decide. It has to come back
    /// playable rather than empty, even when it is the item to avoid.
    #[test]
    fn a_single_item_is_left_alone() {
        assert_eq!(shuffled(1, None, 12345), vec![0]);
        assert_eq!(shuffled(1, Some(0), 12345), vec![0]);
        assert!(shuffled(0, None, 12345).is_empty());
    }

    /// A generator that returned the list unchanged would pass every test
    /// above. This is the one that says it actually shuffles.
    #[test]
    fn different_seeds_give_different_orders() {
        let orders: std::collections::HashSet<Vec<usize>> = (1..60u64)
            .map(|seed| shuffled(8, None, seed * 2_654_435_761))
            .collect();
        assert!(orders.len() > 20, "only {} distinct orders", orders.len());
        assert!(orders.iter().any(|order| *order != straight(8)));
    }

    /// A zero seed must not collapse the generator to zero, which xorshift
    /// does if it is ever allowed to reach it.
    #[test]
    fn a_zero_seed_still_shuffles() {
        assert!(is_permutation(&shuffled(8, None, 0), 8));
    }
}
