//! Stable identifier for a task within an executor.

use core::fmt;
use std::sync::Arc;

/// Identifier for a task added to an [`Executor`](crate::Executor).
///
/// Cheap to clone (`Arc<str>` under the hood). Displayable. Intended to be
/// shown in logs and to correlate observer/monitor callbacks.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct TaskId(Arc<str>);

impl TaskId {
    /// Construct a [`TaskId`] from any string-like value.
    pub fn new(s: impl Into<String>) -> Self {
        Self(Arc::from(s.into()))
    }

    /// Borrow the underlying string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for TaskId {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

impl From<&str> for TaskId {
    fn from(s: &str) -> Self {
        Self(Arc::from(s))
    }
}

impl From<&String> for TaskId {
    fn from(s: &String) -> Self {
        Self(Arc::from(s.as_str()))
    }
}

impl AsRef<str> for TaskId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_display() {
        let id = TaskId::new("hello");
        assert_eq!(id.to_string(), "hello");
        assert_eq!(id.as_str(), "hello");
    }

    #[test]
    fn clone_yields_equal_id() {
        let a = TaskId::new("x");
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn from_various_string_kinds() {
        let lit: TaskId = "lit".into();
        assert_eq!(lit.as_str(), "lit");

        let owned: TaskId = String::from("owned").into();
        assert_eq!(owned.as_str(), "owned");

        let referenced_owned = String::from("ref");
        let from_ref: TaskId = (&referenced_owned).into();
        assert_eq!(from_ref.as_str(), "ref");
    }
}
