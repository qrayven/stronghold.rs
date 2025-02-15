// Copyright 2020-2022 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

//! The version lock is a special type word sized spin lock, that
//! contains a single bit to indicate a lock, while using the rest
//! of the bits for versioning.

use crate::stm::error::*;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

/// A [`VersionLock`] is a combination of a simple bounded spin-locking mechanism, needing
/// 1-bit of a word-sized value to lock a certain region. The rest of the value is being
/// used to increment a version counter. Use a [`VersionClock`], when you want to
/// implement versioned updates on transactional memory regions.
#[derive(Default, Clone, Debug)]
pub struct VersionLock {
    atomic: Arc<AtomicUsize>,
}

impl VersionLock {
    /// Creates a new [`VersionLock`] with the desired version
    pub fn new(version: usize) -> Self {
        Self {
            atomic: Arc::new(AtomicUsize::new(version)),
        }
    }

    /// Tries to acquire a lock and returns an `Ok(())` on success.
    ///
    /// # Example
    /// ```
    /// use stronghold_rlu::stm::VersionLock;
    /// let lock = VersionLock::default();
    /// lock.try_lock().expect("Failed to acquire lock");
    /// assert!(lock.is_locked());
    /// ```
    pub fn try_lock(&self) -> Result<(), TxError> {
        let bound = 1 << 31;

        // bounded spin-locking
        for n in 0..bound {
            if self.is_locked() {
                // Safe some cpu time.
                #[cfg(feature = "threaded")]
                std::thread::sleep(Duration::from_millis(1));

                // // indicate spin lock to the cpu
                // std::hint::spin_loop();

                continue;
            }

            if n == (bound - 1) {
                // return an error, if lock couldn't be acquire within given bounds
                // this avoids a dead lock, but may create thread starving on the other end
                return Err(TxError::LockPresent);
            }
        }
        // set  lock bit
        self.atomic.fetch_or(!mask(), Ordering::SeqCst);

        Ok(())
    }

    /// Unlocks the [`VersionLock`] by simply clearing the lock bit
    #[inline(always)]
    pub fn unlock(&self) -> Result<(), TxError> {
        self.atomic.fetch_and(mask(), Ordering::SeqCst);

        Ok(())
    }

    /// Returns `true`, if the version lock is present
    pub fn is_locked(&self) -> bool {
        let n = self.atomic.load(Ordering::SeqCst);

        let lock_state = mask() & n;

        // check, if locked
        self.atomic
            .compare_exchange(
                mask() & n, // mask the lockbit and compare. if this is set the operation fails
                n,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .map_err(|_| TxError::LockPresent)
            .is_err()
    }

    /// Release the lock and increment the version
    pub fn release(&self) -> Result<(), TxError> {
        // clear lock bit
        self.unlock()?;

        // increment version
        self.atomic.fetch_add(1, Ordering::SeqCst);

        Ok(())
    }

    pub fn release_set(&self, value: usize) -> Result<(), TxError> {
        // clear the lock
        self.unlock()?;

        // set the new version
        self.atomic.store(value, Ordering::SeqCst);

        Ok(())
    }

    /// Returns the stored version
    pub fn version(&self) -> usize {
        self.atomic.load(Ordering::SeqCst) & mask()
    }
}

/// An atomic `VersionClock` with a simpler interface. This type should be
/// used for keeping track of a global version counter
#[derive(Clone, Default)]
pub struct VersionClock {
    atomic: Arc<AtomicUsize>,
}

impl VersionClock {
    pub fn new(version: usize) -> Self {
        Self {
            atomic: Arc::new(AtomicUsize::new(version)),
        }
    }

    /// Atomically increments the version and returns the old value
    pub fn increment(&self) -> Result<usize, TxError> {
        Ok(self.atomic.fetch_add(1, Ordering::SeqCst))
        // Ok(self.atomic.load(Ordering::SeqCst))
    }

    /// Returns the current version
    pub fn version(&self) -> usize {
        self.atomic.load(Ordering::SeqCst)
    }
}
/// Returns the word size in number of bits
const fn word_size_bits() -> usize {
    std::mem::size_of::<usize>() * 8
}

/// Returns the number of bits to shift left to clear the most significant bit
const fn shift_by() -> usize {
    word_size_bits() - 1
}

/// Returns a bitmask to filter the most significant bit
const fn mask() -> usize {
    !(1 << shift_by())
}

#[cfg(test)]
mod tests {

    use rand::Rng;
    use threadpool::ThreadPool;

    use super::VersionLock;
    use crate::stm::TxError;

    #[test]
    fn test_version_lock() -> Result<(), TxError> {
        let lock = VersionLock::default();

        let max_runs = 0xFFF;
        let runs: u32 = rand::thread_rng().gen_range(0..max_runs);

        for _ in 0..runs {
            lock.try_lock()?;
            assert!(lock.is_locked());
            lock.release()?;
        }

        assert_eq!(lock.version(), runs as usize);

        Ok(())
    }

    #[test]
    #[cfg(feature = "threaded")]
    fn test_version_lock_threaded() -> Result<(), TxError> {
        let lock = VersionLock::default();
        let max_runs = 0xFFFF;
        let runs: u32 = rand::thread_rng().gen_range(0..max_runs);

        let threadpool = ThreadPool::new(8);

        // thread pool
        for i in 0..runs {
            let inner = lock.clone();
            threadpool.execute(move || {
                assert!(inner.try_lock().is_ok(), "Failed to get versioned lock");
                let _ = inner.release();
            })
        }

        threadpool.join();
        let version = lock.version();
        assert_eq!(version, runs as usize);

        Ok(())
    }
}
