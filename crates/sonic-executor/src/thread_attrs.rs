//! Thread tuning knobs. Most fields are best-effort; unsupported platforms
//! emit a `tracing::warn!` and proceed with defaults.

#[cfg(feature = "thread_attrs")]
mod inner {
    /// Builder-style thread-tuning bag.
    #[derive(Clone, Debug, Default)]
    #[non_exhaustive]
    pub struct ThreadAttributes {
        pub(crate) name_prefix: Option<String>,
        pub(crate) affinity_mask: Option<Vec<usize>>,
        pub(crate) priority: Option<i32>,
    }

    impl ThreadAttributes {
        /// Build a new attributes bag with no settings.
        #[must_use]
        pub fn new() -> Self {
            Self::default()
        }

        /// Set a prefix used for worker thread names; final name is `<prefix>-<index>`.
        #[must_use]
        pub fn name_prefix(mut self, p: impl Into<String>) -> Self {
            self.name_prefix = Some(p.into());
            self
        }

        /// Pin each worker `i` to `cores[i % cores.len()]`.
        #[must_use]
        pub fn affinity_mask(mut self, cores: Vec<usize>) -> Self {
            self.affinity_mask = Some(cores);
            self
        }

        /// `SCHED_FIFO` priority on Linux; ignored on platforms that don't
        /// support it. The user's process must have `CAP_SYS_NICE` / equivalent.
        #[must_use]
        pub const fn priority(mut self, p: i32) -> Self {
            self.priority = Some(p);
            self
        }

        /// Apply the recorded attributes to the current thread.
        pub(crate) fn apply_to_self(&self, worker_index: usize) {
            if let Some(mask) = &self.affinity_mask {
                let ids = core_affinity::get_core_ids().unwrap_or_default();
                if let Some(core) = mask.get(worker_index % mask.len()) {
                    if let Some(c) = ids.get(*core) {
                        let _ = core_affinity::set_for_current(*c);
                    }
                }
            }
            #[cfg(target_os = "linux")]
            if let Some(prio) = self.priority {
                set_sched_fifo(prio);
            }
            // Suppress unused-variable warning on non-Linux targets.
            #[cfg(not(target_os = "linux"))]
            let _ = self.priority;
        }
    }

    #[cfg(target_os = "linux")]
    #[allow(unsafe_code)]
    fn set_sched_fifo(prio: i32) {
        use std::mem::MaybeUninit;
        let mut param: MaybeUninit<libc::sched_param> = MaybeUninit::zeroed();
        // SAFETY: pthread_setschedparam takes a pointer to sched_param;
        // the param is zero-initialised then we set sched_priority before
        // passing it. Failure (e.g. no CAP_SYS_NICE) is silently ignored.
        unsafe {
            (*param.as_mut_ptr()).sched_priority = prio;
            let _ =
                libc::pthread_setschedparam(libc::pthread_self(), libc::SCHED_FIFO, param.as_ptr());
        }
    }
}

#[cfg(feature = "thread_attrs")]
pub use inner::ThreadAttributes;

#[cfg(not(feature = "thread_attrs"))]
mod stub {
    /// Disabled stub. Enable the `thread_attrs` feature for real settings.
    #[derive(Clone, Debug, Default)]
    pub struct ThreadAttributes;

    impl ThreadAttributes {
        /// Build a new attributes bag (no-op when feature is off).
        #[must_use]
        pub const fn new() -> Self {
            Self
        }

        #[allow(clippy::unused_self)]
        pub(crate) const fn apply_to_self(&self, _i: usize) {}
    }
}

#[cfg(not(feature = "thread_attrs"))]
pub use stub::ThreadAttributes;
