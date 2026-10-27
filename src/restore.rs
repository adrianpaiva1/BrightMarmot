//! Persists each monitor's "normal" colours so a crashed run can be undone at next start.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ramp::GammaRamp;

#[derive(Clone, Debug, PartialEq)]
pub struct SavedNormal {
    pub ramp: Option<GammaRamp>,
    pub vibrance: Option<i32>,
    pub hue: Option<u32>,
}

/// On-disk form; serde can't derive for `[u16; 256]`.
#[derive(Serialize, Deserialize)]
struct DiskEntry {
    ramp: Option<Vec<Vec<u16>>>,
    vibrance: Option<i32>,
    hue: Option<u32>,
}

impl DiskEntry {
    fn from_saved(s: &SavedNormal) -> DiskEntry {
        DiskEntry {
            ramp: s.ramp.as_ref().map(|r| r.iter().map(|c| c.to_vec()).collect()),
            vibrance: s.vibrance,
            hue: s.hue,
        }
    }

    /// `None` if the ramp is not exactly 3 channels of 256 entries.
    fn into_saved(self) -> Option<SavedNormal> {
        let ramp = match self.ramp {
            None => None,
            Some(channels) => {
                if channels.len() != 3 || channels.iter().any(|c| c.len() != 256) {
                    return None;
                }
                let mut ramp: GammaRamp = [[0; 256]; 3];
                for (dst, src) in ramp.iter_mut().zip(&channels) {
                    dst.copy_from_slice(src);
                }
                Some(ramp)
            }
        };
        Some(SavedNormal { ramp, vibrance: self.vibrance, hue: self.hue })
    }
}

pub struct RestoreFile {
    path: Option<PathBuf>,
    entries: BTreeMap<String, SavedNormal>,
}

impl RestoreFile {
    /// Missing, unreadable or malformed files yield an empty set; malformed entries are dropped.
    pub fn load(path: PathBuf) -> RestoreFile {
        let entries = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<BTreeMap<String, serde_json::Value>>(&text).ok())
            .map(|raw| {
                raw.into_iter()
                    .filter_map(|(name, value)| {
                        let saved = serde_json::from_value::<DiskEntry>(value).ok()?.into_saved()?;
                        Some((name, saved))
                    })
                    .collect()
            })
            .unwrap_or_default();
        RestoreFile { path: Some(path), entries }
    }

    /// Never touches disk: `save` is a no-op.
    pub fn in_memory() -> RestoreFile {
        RestoreFile { path: None, entries: BTreeMap::new() }
    }

    pub fn entries(&self) -> &BTreeMap<String, SavedNormal> {
        &self.entries
    }

    pub fn insert(&mut self, monitor: &str, saved: SavedNormal) {
        self.entries.insert(monitor.to_string(), saved);
    }

    pub fn remove(&mut self, monitor: &str) {
        self.entries.remove(monitor);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Writes the entries atomically, or deletes the file when there are none.
    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        if self.entries.is_empty() {
            return match std::fs::remove_file(path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            };
        }
        let disk: BTreeMap<&str, DiskEntry> =
            self.entries.iter().map(|(k, v)| (k.as_str(), DiskEntry::from_saved(v))).collect();
        let json = serde_json::to_string_pretty(&disk).map_err(std::io::Error::other)?;
        crate::config::write_atomic(path, &json)
    }
}

/// `%LOCALAPPDATA%\BrightMarmot\restore.json`.
pub fn default_path() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    Some(PathBuf::from(base).join("BrightMarmot").join("restore.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SavedNormal { SavedNormal { ramp: Some(crate::ramp::identity()), vibrance: Some(-3), hue: None } }
    #[test] fn roundtrip_save_load() {
        let d = tempfile::tempdir().unwrap(); let p = d.path().join("restore.json");
        let mut f = RestoreFile::load(p.clone()); f.insert(r"\\.\DISPLAY1", sample()); f.save().unwrap();
        assert_eq!(RestoreFile::load(p).entries().get(r"\\.\DISPLAY1"), Some(&sample()));
    }
    #[test] fn save_when_empty_deletes_file() {
        let d = tempfile::tempdir().unwrap(); let p = d.path().join("restore.json");
        let mut f = RestoreFile::load(p.clone()); f.insert("m", sample()); f.save().unwrap();
        f.clear(); f.save().unwrap(); assert!(!p.exists());
    }
    #[test] fn missing_file_is_empty() { let d = tempfile::tempdir().unwrap(); assert!(RestoreFile::load(d.path().join("x.json")).entries().is_empty()); }
    #[test] fn truncated_json_is_empty() {
        let d = tempfile::tempdir().unwrap(); let p = d.path().join("restore.json");
        std::fs::write(&p, r#"{"\\\\.\\DISPLAY1": {"ramp": [[1,2"#).unwrap();
        assert!(RestoreFile::load(p).entries().is_empty());
    }
    #[test] fn bad_ramp_shape_entry_dropped() {
        let d = tempfile::tempdir().unwrap(); let p = d.path().join("restore.json");
        std::fs::write(&p, r#"{"bad": {"ramp": [[1,2,3],[1],[1]], "vibrance": 1, "hue": null},
                              "good": {"ramp": null, "vibrance": 7, "hue": 10}}"#).unwrap();
        let f = RestoreFile::load(p);
        assert_eq!(f.entries().len(), 1);
        assert_eq!(f.entries()["good"], SavedNormal { ramp: None, vibrance: Some(7), hue: Some(10) });
    }
}
