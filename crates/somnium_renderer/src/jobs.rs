//! CPU job helpers (Phase CR).
//!
//! wgpu 30 exposes one queue. Parallel work is CPU-side only: LOD classify,
//! frustum bits, instance CPU fill. Record still happens on the render thread.
//!
//! Rayon is used when a slice is large enough that fork-join beats a serial
//! loop. The default 16×16 terrain (256 chunks) stays serial — CR-A showed the
//! frame is GPU-bound at ~50 ms shading, so paying rayon overhead on 256 AABB
//! tests would raise Task Manager % without shortening the frame.

use rayon::prelude::*;

/// Chunk / item count at which a parallel loop is worth the fork-join.
pub const PARALLEL_THRESHOLD: usize = 512;

/// Apply `f` to every element, in parallel once `items.len()` clears
/// [`PARALLEL_THRESHOLD`].
pub fn for_each_mut<T, F>(items: &mut [T], f: F)
where
    T: Send,
    F: Fn(&mut T) + Sync + Send,
{
    if items.len() >= PARALLEL_THRESHOLD {
        items.par_iter_mut().for_each(f);
    } else {
        items.iter_mut().for_each(f);
    }
}

/// Map `f` over `items` in parallel regardless of how few there are.
///
/// [`for_each_mut`]'s [`PARALLEL_THRESHOLD`] is a *count*, which is the right
/// question for AABB tests and the wrong one for work measured in hundreds of
/// milliseconds per element. DOOM-I found thirty-two PNG decodes taking 6.9
/// seconds of startup on one thread; thirty-two is far below the threshold and
/// the threshold was never meant to say anything about them.
///
/// Still fork-join inside a single call, not background work: the caller blocks
/// until every element is done. Nothing here is a second scheduler.
pub fn map_expensive<T, R, F>(items: &[T], f: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> R + Sync + Send,
{
    items.par_iter().map(f).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_expensive_preserves_order() {
        let input: Vec<u32> = (0..64).collect();
        let out = map_expensive(&input, |&x| x * x);
        assert_eq!(out, input.iter().map(|x| x * x).collect::<Vec<_>>());
    }

    #[test]
    fn serial_path_visits_every_item() {
        let mut v: Vec<u32> = (0..8).collect();
        for_each_mut(&mut v, |x| *x += 1);
        assert_eq!(v, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn parallel_path_matches_serial() {
        let n = PARALLEL_THRESHOLD + 16;
        let mut parallel: Vec<u32> = (0..n as u32).collect();
        let mut serial: Vec<u32> = (0..n as u32).collect();
        for_each_mut(&mut parallel, |x| *x = x.wrapping_mul(3).wrapping_add(1));
        serial
            .iter_mut()
            .for_each(|x| *x = x.wrapping_mul(3).wrapping_add(1));
        assert_eq!(parallel, serial);
    }

    #[test]
    fn threshold_boundary_matches_serial() {
        // The branch itself is part of the contract: 511 stays serial, while
        // 512 and 513 take Rayon's path. Exercise all three sizes so changing
        // the comparison or losing an edge item cannot hide behind the broad
        // "threshold + 16" coverage above.
        for n in [
            PARALLEL_THRESHOLD - 1,
            PARALLEL_THRESHOLD,
            PARALLEL_THRESHOLD + 1,
        ] {
            let mut actual: Vec<u32> = (0..n as u32).collect();
            let mut expected = actual.clone();
            for_each_mut(&mut actual, |x| {
                *x = x.rotate_left(7).wrapping_add(0x9e37_79b9)
            });
            expected
                .iter_mut()
                .for_each(|x| *x = x.rotate_left(7).wrapping_add(0x9e37_79b9));
            assert_eq!(actual, expected, "job result diverged at length {n}");
        }
    }
}
