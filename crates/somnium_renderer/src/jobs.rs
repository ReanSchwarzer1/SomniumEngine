//! CPU job helpers (Phase CR).
//!
//! wgpu 29 exposes one queue. Parallel work is CPU-side only: LOD classify,
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
