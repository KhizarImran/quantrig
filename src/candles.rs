//! Revisions become visible only when the Parquet file and its summary are ready.
use crate::{fetcher, store};
use rustix::{
    fs::{CWD, RenameFlags, renameat_with},
    io::Errno,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    error::Error,
    fs::{self, File},
    io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const PARQUET: &str = "candles.parquet";
const SUMMARY: &str = "summary.json";
// A legacy import can finish after a download of the same dataset. Download
// revisions sort above imports so the old file never becomes current again.
enum Publication {
    Import,
    Download,
}

impl Publication {
    fn prefix(&self) -> char {
        match self {
            Self::Import => '0',
            Self::Download => '1',
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub rows: u64,
    pub start: String,
    pub end: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct Revision {
    pub id: String,
    #[serde(flatten)]
    pub summary: Summary,
}

pub struct Resolved {
    pub path: PathBuf,
    pub revision: Revision,
}

pub struct Dataset {
    pub name: String,
    pub bytes: u64,
    pub summary: Option<Summary>,
    pub revision: Option<String>,
}

fn revisions_root() -> PathBuf {
    store::candles_dir().join("revisions")
}

fn dataset_dir(name: &str) -> Result<PathBuf> {
    if !store::is_safe_name(name) || name == "." {
        return Err("bad dataset name".into());
    }
    Ok(revisions_root().join(name))
}

fn valid_id(id: &str) -> bool {
    id.len() == 33 && id.starts_with(['0', '1']) && id.bytes().all(|byte| byte.is_ascii_digit())
}

fn read_revision(dir: &Path, id: &str) -> Result<Resolved> {
    if !valid_id(id) {
        return Err("bad candle revision id".into());
    }
    let path = dir.join(id).join(PARQUET);
    if !path.is_file() {
        return Err("candle revision file is missing".into());
    }
    let summary = serde_json::from_reader(File::open(dir.join(id).join(SUMMARY))?)?;
    Ok(Resolved {
        path,
        revision: Revision {
            id: id.into(),
            summary,
        },
    })
}

fn latest_id(dir: &Path) -> Result<Option<String>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut ids = Vec::new();
    for entry in entries {
        let entry = entry?;
        if let Some(id) = entry.file_name().to_str()
            && valid_id(id)
            && entry.file_type()?.is_dir()
        {
            ids.push(id.to_owned());
        }
    }
    Ok(ids.into_iter().max())
}

fn latest(dir: &Path) -> Result<Option<Resolved>> {
    latest_id(dir)?
        .map(|id| read_revision(dir, &id))
        .transpose()
}

/// The fetcher writes into a private sibling directory on the same filesystem.
pub fn publish(name: &str, write: impl FnOnce(&Path) -> Result<Summary>) -> Result<Revision> {
    let dir = dataset_dir(name)?;
    publish_in(&dir, Publication::Download, write)
}

fn publish_in(
    dir: &Path,
    publication: Publication,
    write: impl FnOnce(&Path) -> Result<Summary>,
) -> Result<Revision> {
    fs::create_dir_all(dir)?;
    let stage = tempfile::Builder::new()
        .prefix(".staging-")
        .tempdir_in(dir)?;
    let path = stage.path().join(PARQUET);
    let summary = write(&path)?;
    if summary.rows == 0 || summary.start.is_empty() || summary.end.is_empty() {
        return Err("candle summary has no rows or date span".into());
    }
    File::open(path)?.sync_all()?;
    let metadata = File::create(stage.path().join(SUMMARY))?;
    serde_json::to_writer(&metadata, &summary)?;
    metadata.sync_all()?;
    File::open(stage.path())?.sync_all()?;

    loop {
        let prefix = publication.prefix();
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let previous = latest_id(dir)?
            .and_then(|id| id.strip_prefix(prefix).map(str::parse::<u128>))
            .transpose()?;
        // Keep revision IDs increasing if the system clock moves backward.
        let stamp = previous.map_or(now, |stamp| now.max(stamp + 1));
        let id = format!("{prefix}{stamp:032}");
        match renameat_with(
            CWD,
            stage.path(),
            CWD,
            dir.join(&id),
            RenameFlags::NOREPLACE,
        ) {
            Ok(()) => {
                File::open(dir)?.sync_all()?;
                return Ok(Revision { id, summary });
            }
            Err(Errno::EXIST) => continue,
            Err(error) => return Err(error.into()),
        }
    }
}

pub fn latest_for_run(name: &str) -> Result<Resolved> {
    let dir = dataset_dir(name)?;
    if let Some(resolved) = latest(&dir)? {
        return Ok(resolved);
    }
    let legacy = store::dataset_path(name)
        .filter(|path| path.is_file())
        .ok_or("no such dataset. Download it first.")?;
    publish_in(&dir, Publication::Import, |path| {
        fs::copy(legacy, path)?;
        fetcher::inspect(path)
    })?;
    latest(&dir)?.ok_or_else(|| "candle revision unavailable".into())
}

pub fn restore(name: &str, revision: &Revision) -> Result<Resolved> {
    let unavailable = || format!("original candle revision {} is unavailable", revision.id);
    let resolved = read_revision(&dataset_dir(name)?, &revision.id).map_err(|_| unavailable())?;
    if resolved.revision != *revision {
        return Err(unavailable().into());
    }
    Ok(resolved)
}

pub fn list() -> Vec<Dataset> {
    let mut names = BTreeSet::new();
    for entry in fs::read_dir(store::candles_dir())
        .into_iter()
        .flatten()
        .flatten()
    {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "parquet")
            && path.is_file()
            && let Some(name) = path.file_stem().and_then(|stem| stem.to_str())
        {
            names.insert(name.to_owned());
        }
    }
    for entry in fs::read_dir(revisions_root())
        .into_iter()
        .flatten()
        .flatten()
    {
        if let Ok(name) = entry.file_name().into_string() {
            names.insert(name);
        }
    }
    names
        .into_iter()
        .filter_map(|name| {
            if let Some(resolved) = latest(&dataset_dir(&name).ok()?).ok()? {
                return Some(Dataset {
                    name,
                    bytes: fs::metadata(resolved.path).ok()?.len(),
                    summary: Some(resolved.revision.summary),
                    revision: Some(resolved.revision.id),
                });
            }
            let path = store::dataset_path(&name)?;
            let bytes = fs::metadata(&path).ok()?.len();
            let summary = File::open(path.with_extension("json"))
                .ok()
                .and_then(|file| serde_json::from_reader(file).ok());
            Some(Dataset {
                name,
                bytes,
                summary,
                revision: None,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_fixture(path: &Path, content: &str, start: &str) -> Result<Summary> {
        fs::write(path, content)?;
        Ok(Summary {
            rows: 1,
            start: start.into(),
            end: start.into(),
        })
    }

    #[test]
    fn publishing_a_new_revision_never_replaces_prior_data() {
        let dir = tempfile::tempdir().unwrap();
        let prior = publish_in(dir.path(), Publication::Download, |path| {
            write_fixture(path, "old candles", "2024-01-01")
        })
        .unwrap();
        let next = publish_in(dir.path(), Publication::Download, |path| {
            assert_eq!(latest(dir.path())?.unwrap().revision, prior);
            write_fixture(path, "new candles", "2025-02-02")
        })
        .unwrap();
        assert_ne!(prior.id, next.id);
        assert_eq!(latest(dir.path()).unwrap().unwrap().revision, next);
        let replay = read_revision(dir.path(), &prior.id).unwrap();
        assert_eq!(replay.revision, prior);
        assert_eq!(fs::read_to_string(replay.path).unwrap(), "old candles");
    }

    #[test]
    fn failed_staging_and_a_competing_legacy_import_cannot_replace_current_data() {
        let dir = tempfile::tempdir().unwrap();
        let original = publish_in(dir.path(), Publication::Download, |path| {
            write_fixture(path, "original", "2024-01-01")
        })
        .unwrap();
        let failed = publish_in(dir.path(), Publication::Download, |path| {
            fs::write(path, "partial")?;
            Err("download interrupted".into())
        });
        assert!(failed.is_err());
        let imported = publish_in(dir.path(), Publication::Import, |path| {
            write_fixture(path, "different", "2025-02-02")
        })
        .unwrap();
        assert_ne!(imported.id, original.id);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
        let resolved = latest(dir.path()).unwrap().unwrap();
        assert_eq!(resolved.revision, original);
        assert_eq!(fs::read_to_string(resolved.path).unwrap(), "original");
    }
}
