//! Seekable reads from one Linux file descriptor owned by the WSL login account.
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::STANDARD};

use crate::text_files::TextSnapshot;

pub(super) struct Reader {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    offset: u64,
    size: u64,
}

impl Drop for Reader {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub(super) fn open(mut shell: Command, path: &str) -> Result<(Reader, TextSnapshot)> {
    // A line-delimited response also terminates on short reads if the open file shrinks.
    let script = r#"set -eu
export LC_ALL=C PATH=/usr/bin:/bin
[ -f "$1" ] && [ ! -L "$1" ] || exit 1
exec 3< "$1"
[ -f /proc/self/fd/3 ] || exit 1
stat -Lc '%s %Y' /proc/self/fd/3
while read -r offset count; do
    dd if=/proc/self/fd/3 bs=128K iflag=skip_bytes,count_bytes skip="$offset" count="$count" status=none | base64 -w0
    printf '\n'
done"#;
    let mut child = shell
        .args(["-c", script, "arcnix", path])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut reader = Reader {
        input: child.stdin.take().unwrap(),
        output: BufReader::new(child.stdout.take().unwrap()),
        child,
        offset: 0,
        size: 0,
    };
    let mut header = String::new();
    reader.output.by_ref().take(80).read_line(&mut header)?;
    let (size, modified) = header
        .trim_end()
        .split_once(' ')
        .context("Cannot read the selected regular file in WSL")?;
    let snapshot = TextSnapshot {
        size: size.parse()?,
        modified: Some(modified.parse()?),
    };
    reader.size = snapshot.size;
    Ok((reader, snapshot))
}

impl Read for Reader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = (self.size.saturating_sub(self.offset))
            .min(buffer.len() as u64)
            .min(1024 * 1024) as usize;
        if count == 0 {
            return Ok(0);
        }
        writeln!(self.input, "{} {count}", self.offset)?;
        let mut encoded = Vec::new();
        let bound = 4 * count.div_ceil(3) + 1;
        self.output
            .by_ref()
            .take(bound as u64)
            .read_until(b'\n', &mut encoded)?;
        let encoded = encoded
            .strip_suffix(b"\n")
            .ok_or_else(|| io::Error::other("Incomplete WSL file response"))?;
        let bytes = STANDARD.decode(encoded).map_err(io::Error::other)?;
        if bytes.len() > count {
            return Err(io::Error::other("Invalid WSL file range"));
        }
        buffer[..bytes.len()].copy_from_slice(&bytes);
        self.offset += bytes.len() as u64;
        Ok(bytes.len())
    }
}

impl Seek for Reader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.offset = match pos {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::Current(delta) => self.offset.checked_add_signed(delta),
            SeekFrom::End(delta) => self.size.checked_add_signed(delta),
        }
        .ok_or_else(|| io::Error::other("Invalid file offset"))?;
        Ok(self.offset)
    }
}
