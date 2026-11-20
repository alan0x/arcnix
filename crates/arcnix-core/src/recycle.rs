//! Reversible file operations, separate from permanent deletion.
use anyhow::{Context, Result, ensure};

use crate::deletion::Target;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    pub original: String,
    pub deleted: String,
    pub payload: Option<Target>,
}

#[derive(Clone, Debug)]
pub enum Operation {
    List,
    Move(Vec<Target>),
    Restore(Item),
    Purge(Item),
}

pub fn timestamp(time: time::OffsetDateTime) -> String {
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        time.year(),
        u8::from(time.month()),
        time.day(),
        time.hour(),
        time.minute(),
        time.second()
    )
}

pub(crate) fn move_roots(
    targets: &[Target],
    check: &impl Fn() -> Result<()>,
    mut move_one: impl FnMut(&Target) -> Result<()>,
) -> Result<()> {
    ensure!(
        !targets.is_empty() && targets.len() <= 1000,
        "Select 1–1000 items"
    );
    for (index, target) in targets.iter().enumerate() {
        check()?;
        move_one(target)
            .with_context(|| format!("Moved {index} items to Trash; failed at {}", target.path))?;
    }
    Ok(())
}

pub fn local(operation: &Operation) -> Result<Vec<Item>> {
    if let Operation::Move(targets) = operation {
        move_roots(targets, &|| Ok(()), crate::local::recycle)?;
        return Ok(Vec::new());
    }
    #[cfg(target_os = "macos")]
    anyhow::bail!("Use Finder to manage the local Trash");
    #[cfg(not(target_os = "macos"))]
    {
        use trash::os_limited::{list, purge_all, restore_all};
        fn item(native: &trash::TrashItem) -> Option<Item> {
            let deleted = time::OffsetDateTime::from_unix_timestamp(native.time_deleted)
                .ok()
                .map(|t| {
                    timestamp(t.to_offset(
                        time::UtcOffset::local_offset_at(t).unwrap_or(time::UtcOffset::UTC),
                    ))
                })
                .unwrap_or_default();
            Some(Item {
                id: native.id.to_str()?.into(),
                original: native.original_path().to_string_lossy().into_owned(),
                deleted,
                payload: None,
            })
        }
        let native = list()?;
        match operation {
            Operation::List => Ok(native.iter().filter_map(item).collect()),
            Operation::Restore(expected) | Operation::Purge(expected) => {
                let entry = native
                    .into_iter()
                    .find(|entry| item(entry).as_ref() == Some(expected))
                    .context("Trash item changed; refresh the list")?;
                if matches!(operation, Operation::Restore(_)) {
                    ensure!(
                        std::fs::symlink_metadata(entry.original_path())
                            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
                        "Original path is occupied or inaccessible"
                    );
                    restore_all([entry])?;
                } else {
                    purge_all([entry])?;
                }
                Ok(Vec::new())
            }
            Operation::Move(_) => unreachable!(),
        }
    }
}
