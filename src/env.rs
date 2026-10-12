//! A copy of the process environment that the code can read and change.
//! Tests make one by hand, so no test changes the real environment.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Env(BTreeMap<OsString, OsString>);

impl Env {
    /// Copies the environment of this process.
    pub fn from_process() -> Self {
        Env(std::env::vars_os().collect())
    }

    /// Returns the value of `key`. An empty value counts as not set.
    pub fn get(&self, key: &str) -> Option<&OsStr> {
        self.0
            .get(OsStr::new(key))
            .map(OsString::as_os_str)
            .filter(|value| !value.is_empty())
    }

    /// Returns true when `key` is in the environment, also with an empty value.
    pub fn contains(&self, key: &str) -> bool {
        self.0.contains_key(OsStr::new(key))
    }

    pub fn set(&mut self, key: &str, value: impl Into<OsString>) {
        self.0.insert(key.into(), value.into());
    }

    pub fn remove(&mut self, key: &str) -> Option<OsString> {
        self.0.remove(OsStr::new(key))
    }

    pub fn iter(&self) -> impl Iterator<Item = (&OsStr, &OsStr)> {
        self.0.iter().map(|(k, v)| (k.as_os_str(), v.as_os_str()))
    }
}

impl<K: Into<OsString>, V: Into<OsString>> FromIterator<(K, V)> for Env {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Env(iter
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_value_counts_as_not_set() {
        let env: Env = [("A", ""), ("B", "x")].into_iter().collect();
        assert_eq!(env.get("A"), None);
        assert!(env.contains("A"));
        assert_eq!(env.get("B"), Some(OsStr::new("x")));
        assert_eq!(env.get("C"), None);
    }
}
