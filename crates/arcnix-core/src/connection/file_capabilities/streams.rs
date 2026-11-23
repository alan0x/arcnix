use anyhow::{Result, ensure};

use crate::local_desktop::{ReadSeek, Target};
use crate::remote::{AdminTextRetry, RemoteSession};
use crate::text_files::{TextRange, TextSnapshot};

pub trait SeekableFileReader {
    /// Size and bytes refer to the same open handle throughout playback.
    fn open_seekable_file(&self, path: &str) -> Result<(Box<dyn ReadSeek + Send>, u64)>;
}

#[derive(Clone, Copy)]
pub struct TextPage<'a> {
    pub path: &'a str,
    pub snapshot: TextSnapshot,
    pub reviewed_handle: Option<u64>,
}

pub trait TextPageReader {
    fn read_page(
        &mut self,
        page: TextPage<'_>,
        offset: u64,
        cursor: Option<u64>,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<TextRange>;

    fn sample_page_tail(
        &mut self,
        page: TextPage<'_>,
        stamp: Option<[u8; 32]>,
    ) -> Result<(TextSnapshot, [u8; 32])>;

    fn close_reviewed_page(&mut self, _handle: Option<u64>) {}
    fn close_page_cursor(&mut self, _cursor: Option<u64>) {}
}

pub trait ReviewedPageOpener {
    fn attach_reviewed_page(
        &mut self,
        page: TextPage<'_>,
        retry: &AdminTextRetry,
        password: &str,
    ) -> Result<(TextSnapshot, [u8; 32])>;
}

impl SeekableFileReader for Target {
    fn open_seekable_file(&self, path: &str) -> Result<(Box<dyn ReadSeek + Send>, u64)> {
        let (file, snapshot) = self.open_binary(path)?;
        Ok((file, snapshot.size))
    }
}

impl SeekableFileReader for RemoteSession {
    fn open_seekable_file(&self, path: &str) -> Result<(Box<dyn ReadSeek + Send>, u64)> {
        let (file, size) = self.open_media(path)?;
        Ok((Box::new(file), size))
    }
}

impl TextPageReader for &Target {
    fn read_page(
        &mut self,
        page: TextPage<'_>,
        offset: u64,
        _cursor: Option<u64>,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<TextRange> {
        ensure!(
            page.reviewed_handle.is_none(),
            "Local editing uses the current account"
        );
        check()?;
        self.read_text_range(page.path, offset, page.snapshot)
    }

    fn sample_page_tail(
        &mut self,
        page: TextPage<'_>,
        stamp: Option<[u8; 32]>,
    ) -> Result<(TextSnapshot, [u8; 32])> {
        ensure!(
            page.reviewed_handle.is_none(),
            "Local editing uses the current account"
        );
        self.poll_text_tail(page.path, page.snapshot, stamp)
    }
}

impl TextPageReader for RemoteSession {
    fn read_page(
        &mut self,
        page: TextPage<'_>,
        offset: u64,
        cursor: Option<u64>,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<TextRange> {
        if let Some(handle) = page.reviewed_handle {
            check()?;
            self.admin_text_range(handle, page.path, offset, page.snapshot)
        } else {
            self.read_text_range(page.path, offset, page.snapshot, cursor, check)
        }
    }

    fn sample_page_tail(
        &mut self,
        page: TextPage<'_>,
        stamp: Option<[u8; 32]>,
    ) -> Result<(TextSnapshot, [u8; 32])> {
        if let Some(handle) = page.reviewed_handle {
            self.poll_admin_page(handle, page.path, page.snapshot, stamp)
        } else {
            self.poll_text_tail(page.path, page.snapshot, stamp)
        }
    }

    fn close_reviewed_page(&mut self, handle: Option<u64>) {
        self.close_admin_pages(handle);
    }

    fn close_page_cursor(&mut self, cursor: Option<u64>) {
        self.close_search_reader(cursor);
    }
}

impl ReviewedPageOpener for RemoteSession {
    fn attach_reviewed_page(
        &mut self,
        page: TextPage<'_>,
        retry: &AdminTextRetry,
        password: &str,
    ) -> Result<(TextSnapshot, [u8; 32])> {
        ensure!(retry.path == page.path, "Read target changed");
        let handle = page
            .reviewed_handle
            .ok_or_else(|| anyhow::anyhow!("Missing reviewed file viewer handle"))?;
        self.open_admin_page(handle, retry, password, page.snapshot)
    }
}
