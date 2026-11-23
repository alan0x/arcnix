//! Run against an isolated Linux OpenSSH fixture with a root-owned, mode 0600
//! /home/arcnix/review-fixture/notes.txt containing "private fixture data".
use std::net::SocketAddr;
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result, ensure};
use arcnix_core::ServerProfile;
use arcnix_core::file_capabilities::{
    DirectoryReader, DocumentReader, DocumentWriter, ReviewedDocument, ReviewedFileOperations,
    ReviewedPageOpener, TextPage, TextPageReader,
};
use arcnix_core::remote::{
    FileAction, PendingConnection, SudoPasswordRequired, file_permission_denied,
};
use arcnix_core::text_files::TextSnapshot;

#[test]
#[ignore = "requires disposable loopback OpenSSH and sudo fixture"]
fn reviewed_capabilities_keep_read_write_and_page_authorization_scoped() -> Result<()> {
    let address: SocketAddr = std::env::var("ARCNIX_TEST_REVIEW_SSH_ADDR")?.parse()?;
    ensure!(address.ip().is_loopback(), "fixture must be loopback");
    let fingerprint = std::env::var("ARCNIX_TEST_REVIEW_SSH_FINGERPRINT")?;
    let pending = PendingConnection::open_for_known_host(
        &ServerProfile {
            host: address.ip().to_string(),
            port: address.port(),
            username: "arcnix".into(),
            ..Default::default()
        },
        &fingerprint,
    )?;
    let mut session = pending.authenticate("arcnix-test", &fingerprint)?;
    let cancel = AtomicBool::new(false);
    let check = || Ok(());
    let path = "/home/arcnix/review-fixture/notes.txt";
    let denied = session.read_document(path, &check).unwrap_err();
    assert!(file_permission_denied(&denied), "{denied:#}");
    let read = session.prepare_admin_read(path)?;
    let mut reader = ReviewedDocument {
        session: &session,
        retry: &read,
        password: "",
        cancel: &cancel,
    };
    assert!(
        reader
            .read_document(path, &check)
            .unwrap_err()
            .is::<SudoPasswordRequired>()
    );
    reader.password = "arcnix-test";
    let wrong_path = reader
        .read_document("/home/arcnix/review-fixture/different.txt", &check)
        .unwrap_err();
    assert_eq!(wrong_path.to_string(), "Read target changed");
    let original = reader.read_document(path, &check)?;
    assert_eq!(original.content, "private fixture data\n");

    let content = "updated fixture data\n";
    let save = session.prepare_admin_save(&original, content)?;
    let saved = ReviewedDocument {
        retry: &save,
        ..reader
    }
    .save_document(&original, content, &check)?;
    assert_eq!(saved.version.mode, original.version.mode);
    assert_eq!(saved.version.uid, original.version.uid);
    assert!(file_permission_denied(
        &session.read_document(path, &check).unwrap_err()
    ));

    let page = TextPage {
        path,
        snapshot: TextSnapshot {
            size: saved.version.size,
            modified: saved.version.modified,
        },
        reviewed_handle: Some(1),
    };
    let review = session.prepare_admin_page(path)?;
    session.attach_reviewed_page(page, &review, "arcnix-test")?;
    let range = session.read_page(page, 0, None, &check)?;
    assert_eq!(range.text, content);
    session.close_reviewed_page(page.reviewed_handle);
    let error = session
        .read_page(page, 0, None, &check)
        .err()
        .context("reviewed page must close")?;
    assert!(error.to_string().contains("File viewer closed"));

    let directory = "/home/arcnix/review-fixture";
    let copy = session.prepare_managed_retry(FileAction::Copy(None), vec![path.into()])?;
    session.apply_reviewed_file_action(&copy, "arcnix-test", &check)?;
    let (_, entries) = session.list_entries(directory)?;
    let copied = entries
        .iter()
        .find(|entry| entry.name == "notes copy.txt")
        .context("missing reviewed copy")?;
    let target = arcnix_core::deletion::Target::from(copied);
    assert_eq!(
        session.delete_reviewed_entries(&[target], "arcnix-test", &cancel, &check)?,
        1
    );
    let restore = session.prepare_admin_save(&saved, &original.content)?;
    ReviewedDocument {
        session: &session,
        retry: &restore,
        password: "arcnix-test",
        cancel: &cancel,
    }
    .save_document(&saved, &original.content, &check)?;
    Ok(())
}
