//! Session-only file manifests and progress beneath a transfer queue item.
use arcnix_core::transfer::{FileProgress, FileStatus, TransferPlan};

use super::*;

#[derive(Clone, Default)]
pub(super) struct Details {
    pub expanded: bool,
    pub is_directory: bool,
    planned: bool,
    files: Vec<DetailFile>,
}

#[derive(Clone)]
struct DetailFile {
    path: String,
    total_bytes: u64,
    transferred_bytes: u64,
    status: FileStatus,
}

fn terminal(status: &FileStatus) -> bool {
    matches!(
        status,
        FileStatus::Completed | FileStatus::Skipped | FileStatus::Cancelled | FileStatus::Failed(_)
    )
}

pub(super) fn plan(s: &mut State, id: &str, plan: &TransferPlan) {
    let details = s.transfer_details.entry(id.to_owned()).or_default();
    details.is_directory = plan.is_directory;
    details.planned = true;
    details.files = plan
        .files
        .iter()
        .map(|file| DetailFile {
            path: file.relative_path.clone(),
            total_bytes: file.size,
            transferred_bytes: 0,
            status: FileStatus::Queued,
        })
        .collect();
}

pub(super) fn update(s: &mut State, id: &str, progress: FileProgress) {
    let Some(file) = s
        .transfer_details
        .get_mut(id)
        .and_then(|details| details.files.get_mut(progress.index))
    else {
        return;
    };
    if terminal(&file.status) {
        return;
    }
    file.total_bytes = file.total_bytes.max(progress.total_bytes);
    file.transferred_bytes = file
        .transferred_bytes
        .max(progress.transferred_bytes)
        .min(file.total_bytes);
    file.status = progress.status;
}

pub(super) fn waiting(s: &State, id: &str) -> bool {
    let Some(details) = s.transfer_details.get(id) else {
        return false;
    };
    details
        .files
        .iter()
        .any(|file| file.status == FileStatus::Waiting)
        && !details
            .files
            .iter()
            .any(|file| file.status == FileStatus::Running)
}

pub(super) fn finish(s: &mut State, id: &str, status: &TransferStatus) {
    let Some(details) = s.transfer_details.get_mut(id) else {
        return;
    };
    for file in &mut details.files {
        if terminal(&file.status) {
            continue;
        }
        file.status = match status {
            TransferStatus::Completed => {
                file.transferred_bytes = file.total_bytes;
                FileStatus::Completed
            }
            TransferStatus::Cancelled => FileStatus::Cancelled,
            TransferStatus::Failed(error) if file.status != FileStatus::Queued => {
                FileStatus::Failed(error.clone())
            }
            TransferStatus::Failed(_) => FileStatus::Cancelled,
            _ => continue,
        };
    }
}

pub(super) fn reset(s: &mut State, old_id: &str, new_id: &str) {
    let old = s.transfer_details.remove(old_id).unwrap_or_default();
    s.transfer_details.insert(
        new_id.to_owned(),
        Details {
            expanded: old.expanded,
            is_directory: old.is_directory,
            ..Default::default()
        },
    );
}

fn percent(file: &DetailFile) -> f64 {
    if matches!(file.status, FileStatus::Completed | FileStatus::Skipped) {
        100.
    } else if file.total_bytes == 0 {
        0.
    } else {
        (file.transferred_bytes as f64 / file.total_bytes as f64 * 100.).clamp(0., 99.9)
    }
}

fn status_label(status: &FileStatus) -> String {
    match status {
        FileStatus::Queued => "等待中".into(),
        FileStatus::Running => "正在传输".into(),
        FileStatus::Waiting => "等待文件冲突处理".into(),
        FileStatus::Completed => "已完成".into(),
        FileStatus::Skipped => "已跳过".into(),
        FileStatus::Cancelled => "已取消".into(),
        FileStatus::Failed(error) => format!("失败：{error}"),
    }
}

pub(super) fn toggle(ui: Ui, job: Cage<TransferJob>) -> impl Widget {
    let state = ui.state;
    glory::ui::button()
        .class("transfer-details-toggle")
        .name(i18n::bind(state.map(move |s| {
            if s.transfer_details
                .get(&job.get().id)
                .is_some_and(|details| details.expanded)
            {
                "收起文件进度"
            } else {
                "展开文件进度"
            }
            .to_owned()
        })))
        .label("")
        .fill(icons::disclosure_icon(state.map(move |s| {
            s.transfer_details
                .get(&job.get().id)
                .is_some_and(|details| details.expanded)
        })))
        .semantics(state.map(move |s| {
            Some(Box::new(glory::render::Semantics {
                expanded: Some(
                    s.transfer_details
                        .get(&job.get().id)
                        .is_some_and(|details| details.expanded),
                ),
                ..Default::default()
            }))
        }))
        .on(activate, move |_| {
            state.revise(|s| {
                let details = s.transfer_details.entry(job.get().id).or_default();
                details.expanded = !details.expanded;
            })
        })
}

pub(super) fn listing(ui: Ui, job: Cage<TransferJob>) -> impl Widget {
    let state = ui.state;
    col()
        .class(state.map(move |s| {
            if s.transfer_details
                .get(&job.get().id)
                .is_some_and(|details| details.expanded)
            {
                "transfer-details"
            } else {
                "hidden"
            }
            .to_owned()
        }))
        .fill((
            text()
                .class("transfer-files-summary")
                .content(state.map(move |s| {
                    let Some(details) = s.transfer_details.get(&job.get().id) else {
                        return String::new();
                    };
                    if !details.planned {
                        return String::new();
                    }
                    let completed = details
                        .files
                        .iter()
                        .filter(|file| {
                            matches!(file.status, FileStatus::Completed | FileStatus::Skipped)
                        })
                        .count();
                    i18n::translate(&format!(
                        "{} 个文件 · 已处理 {}",
                        details.files.len(),
                        completed
                    ))
                })),
            text()
                .class(state.map(move |s| {
                    if s.transfer_details
                        .get(&job.get().id)
                        .is_none_or(|details| details.files.is_empty())
                    {
                        "transfer-files-empty"
                    } else {
                        "hidden"
                    }
                    .to_owned()
                }))
                .content(i18n::bind(state.map(move |s| {
                    let current = job.get();
                    let details = s.transfer_details.get(&current.id);
                    if details.is_some_and(|details| details.planned) {
                        "没有需要传输的文件".to_owned()
                    } else {
                        match &current.status {
                            TransferStatus::Queued => "等待扫描文件…".into(),
                            TransferStatus::Running => "正在扫描文件…".into(),
                            TransferStatus::Completed => "没有需要传输的文件".into(),
                            TransferStatus::Cancelled => "已取消".into(),
                            TransferStatus::Failed(error) => format!("失败：{error}"),
                        }
                    }
                }))),
            each(state.map(move |s| {
                s.transfer_details
                    .get(&job.get().id)
                    .filter(|details| details.expanded)
                    .map(|details| details.files.clone())
                    .unwrap_or_default()
            }))
            .key(|file| file.path.clone())
            .children(move |item| {
                let file = item.value;
                row().class("transfer-file-row").fill((
                    text()
                        .class("transfer-file-path")
                        .selectable(true)
                        .content(file.map(|file| file.path.clone())),
                    text()
                        .class("transfer-file-status")
                        .content(i18n::bind(file.map(|file| status_label(&file.status)))),
                    col().class("transfer-file-progress-content").fill(
                        col()
                            .class(file.map(|file| {
                                match file.status {
                                    FileStatus::Failed(_) => {
                                        "job-progress transfer-file-progress failed"
                                    }
                                    FileStatus::Cancelled => {
                                        "job-progress transfer-file-progress cancelled"
                                    }
                                    _ => "job-progress transfer-file-progress",
                                }
                                .to_owned()
                            }))
                            .name(i18n::bind("文件传输进度"))
                            .role(glory::render::Role::ProgressBar)
                            .semantics(file.map(|file| {
                                Some(Box::new(glory::render::Semantics {
                                    progress: Some(glory::render::ProgressValue {
                                        min: 0.,
                                        max: 100.,
                                        current: Some(percent(file)),
                                        value_text: None,
                                    }),
                                    ..Default::default()
                                }))
                            }))
                            .fill(col().style(file.map(|file| {
                                glory::style::Style::new()
                                    .width(glory::style::Length::Percent(percent(file)))
                            }))),
                    ),
                    text()
                        .class("transfer-file-bytes")
                        .content(file.map(|file| {
                            format!(
                                "{} / {}",
                                transfer_progress::bytes(file.transferred_bytes),
                                transfer_progress::bytes(file.total_bytes),
                            )
                        })),
                    text()
                        .class("transfer-file-percent")
                        .content(file.map(|file| format!("{:.1}%", percent(file)))),
                ))
            }),
        ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unfinished_and_failed_files_never_claim_full_progress() {
        let mut file = DetailFile {
            path: "nested/file.txt".into(),
            total_bytes: 10,
            transferred_bytes: 10,
            status: FileStatus::Running,
        };
        assert_eq!(percent(&file), 99.9);
        file.status = FileStatus::Failed("publish failed".into());
        assert_eq!(percent(&file), 99.9);
        file.status = FileStatus::Completed;
        assert_eq!(percent(&file), 100.);
        file.total_bytes = 0;
        file.transferred_bytes = 0;
        assert_eq!(percent(&file), 100.);
        file.status = FileStatus::Cancelled;
        assert_eq!(percent(&file), 0.);
    }
}
