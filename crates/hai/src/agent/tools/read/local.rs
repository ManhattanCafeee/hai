use std::{io::SeekFrom, ops::Range, path::Path};

use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, BufReader},
};

use super::{
    Read, ReadArgs, ReadOutput,
    paging::{DEFAULT_LINES, MAX_LINE_CHARS, MAX_LIST_ENTRIES, MAX_WINDOW_LINES, Page, paginate},
};
use crate::{
    agent::multimodal::{ImageSource, MediaSource, Payload},
    agentcore::tool::{ToolError, tool_err},
    domain::vo::{AttachmentParser, ImageFormat, MediaCodec},
};

/// The prefix a file is classified through.
const MAX_SNIFF_BYTES: u64 = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    Dir,
    File,
    Other,
}

#[derive(Debug, Clone, Copy)]
struct EntryStat {
    kind: EntryKind,
    size: u64,
}

#[derive(Debug, Clone)]
struct Entry {
    name: String,
    kind: EntryKind,
    /// Files only.
    size: Option<u64>,
}

fn failed(path: &Path, e: std::io::Error) -> ToolError {
    tool_err(format!("{}: {e}", path.display()))
}

async fn stat(path: &Path) -> Result<EntryStat, ToolError> {
    let meta = tokio::fs::metadata(path)
        .await
        .map_err(|e| failed(path, e))?;
    let kind = if meta.is_dir() {
        EntryKind::Dir
    } else if meta.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    Ok(EntryStat {
        kind,
        size: meta.len(),
    })
}

async fn list(path: &Path) -> Result<Vec<Entry>, ToolError> {
    let mut reader = tokio::fs::read_dir(path)
        .await
        .map_err(|e| failed(path, e))?;
    let mut entries = Vec::new();
    while let Some(entry) = reader.next_entry().await.map_err(|e| failed(path, e))? {
        let meta = match entry.metadata().await {
            Ok(meta) => Some(meta),
            Err(e) => {
                tracing::warn!(
                    entry = %entry.file_name().to_string_lossy(),
                    "directory entry metadata unavailable: {e}"
                );
                None
            }
        };
        let kind = match meta.as_ref() {
            Some(meta) if meta.is_dir() => EntryKind::Dir,
            Some(meta) if meta.is_file() => EntryKind::File,
            _ => EntryKind::Other,
        };
        entries.push(Entry {
            name: entry.file_name().to_string_lossy().into_owned(),
            size: meta.filter(|_| kind == EntryKind::File).map(|m| m.len()),
            kind,
        });
    }
    Ok(entries)
}

/// Bytes of `range` (0-based, end exclusive), clamped to what the file holds.
async fn read_range(path: &Path, range: Range<u64>) -> Result<Vec<u8>, ToolError> {
    let len = range.end.saturating_sub(range.start);
    if len == 0 {
        return Ok(Vec::new());
    }
    let mut file = File::open(path).await.map_err(|e| failed(path, e))?;
    file.seek(SeekFrom::Start(range.start))
        .await
        .map_err(|e| failed(path, e))?;
    let mut buf = vec![0u8; len as usize];
    let mut filled = 0;
    while filled < buf.len() {
        let read = file
            .read(&mut buf[filled..])
            .await
            .map_err(|e| failed(path, e))?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    buf.truncate(filled);
    Ok(buf)
}

/// A 1-based half-open line window: `3..6` is lines 3, 4, 5 (the end is exclusive).
async fn lines(path: &Path, range: Range<u64>) -> Result<Vec<String>, ToolError> {
    if range.end <= range.start {
        return Ok(Vec::new());
    }
    let file = File::open(path).await.map_err(|e| failed(path, e))?;
    let mut lines = BufReader::new(file).lines();
    let mut window = Vec::new();
    let mut index = 0;
    while let Some(line) = lines.next_line().await.map_err(|e| failed(path, e))? {
        index += 1;
        if index < range.start {
            continue;
        }
        if index >= range.end {
            break;
        }
        window.push(line);
    }
    Ok(window)
}

/// The file's own line count, keeping a last line that has no newline.
async fn line_count(path: &Path) -> Result<u64, ToolError> {
    let file = File::open(path).await.map_err(|e| failed(path, e))?;
    let mut lines = BufReader::new(file).lines();
    let mut count = 0;
    while lines
        .next_line()
        .await
        .map_err(|e| failed(path, e))?
        .is_some()
    {
        count += 1;
    }
    Ok(count)
}

fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|b| *b == 0)
}

/// The lines one window reads: the request, bounded to what one page may carry.
fn window_lines(limit: Option<u64>) -> u64 {
    limit.unwrap_or(DEFAULT_LINES).clamp(1, MAX_WINDOW_LINES)
}

/// A directory entry as the model reads it: a trailing slash marks a directory, a size follows a file.
fn render_entry(entry: &Entry) -> String {
    let slash = if entry.kind == EntryKind::Dir {
        "/"
    } else {
        ""
    };
    let size = entry
        .size
        .map(|size| format!(" ({size})"))
        .unwrap_or_default();
    format!("{}{slash}{size}", entry.name)
}

fn truncate_line(line: String) -> String {
    let chars = line.chars().count();
    if chars <= MAX_LINE_CHARS {
        return line;
    }
    let kept: String = line.chars().take(MAX_LINE_CHARS).collect();
    format!("{kept} … [truncated {} chars]", chars - MAX_LINE_CHARS)
}

fn name_of(path: &Path) -> Option<&str> {
    path.file_name().and_then(std::ffi::OsStr::to_str)
}

impl Read {
    pub(super) async fn read_local(
        &self,
        path: &Path,
        args: &ReadArgs,
    ) -> Result<ReadOutput, ToolError> {
        self.sandbox.ensure_readable(path)?;
        let stat = stat(path).await?;
        match stat.kind {
            EntryKind::Dir => self.list_dir(path, args).await,
            EntryKind::File => self.read_file(path, stat.size, args).await,
            EntryKind::Other => Err(tool_err(format!(
                "{} is neither a file nor a directory",
                path.display()
            ))),
        }
    }

    async fn list_dir(&self, path: &Path, args: &ReadArgs) -> Result<ReadOutput, ToolError> {
        let mut entries = list(path).await?;
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        if entries.is_empty() {
            return Ok(ReadOutput::Text {
                content: "(empty directory)".to_owned(),
            });
        }
        let listing = entries
            .iter()
            .map(render_entry)
            .collect::<Vec<_>>()
            .join("\n");
        Ok(ReadOutput::Paged(paginate(
            &listing,
            args.offset,
            Some(
                args.limit
                    .map_or(MAX_LIST_ENTRIES, |limit| limit.min(MAX_LIST_ENTRIES)),
            ),
            None,
        )))
    }

    async fn read_file(
        &self,
        path: &Path,
        size: u64,
        args: &ReadArgs,
    ) -> Result<ReadOutput, ToolError> {
        let name = name_of(path);
        let head = read_range(path, 0..MAX_SNIFF_BYTES.min(size)).await?;
        let parser = AttachmentParser::from_bytes(&head)
            .or_else(|| name.and_then(AttachmentParser::from_filename));
        if let Some(parser) = parser {
            let payload = self.payload(parser, &head, path, name, size).await?;
            let content = self
                .analyzer
                .analyze(payload, parser, args.parser, None, args.focus.as_deref())
                .await?;
            return Ok(ReadOutput::Text { content });
        }
        if looks_binary(&head) {
            return Err(tool_err(format!("Binary file: {}", path.display())));
        }
        self.window(path, args).await
    }

    /// The payload a local file's parser needs; the format comes from the same detection chain that
    /// chose the parser, so the image lookup cannot fail.
    async fn payload(
        &self,
        parser: AttachmentParser,
        head: &[u8],
        path: &Path,
        name: Option<&str>,
        size: u64,
    ) -> Result<Payload, ToolError> {
        match parser {
            AttachmentParser::Image | AttachmentParser::Ocr => {
                let format = ImageFormat::sniff(head)
                    .or_else(|| name.and_then(ImageFormat::from_filename))
                    .expect("detection matched an image format");
                Ok(Payload::Image(ImageSource::bytes(
                    self.whole(path, size).await?,
                    format,
                )))
            }
            AttachmentParser::Audio | AttachmentParser::Video => {
                let codec =
                    MediaCodec::sniff(head).or_else(|| name.and_then(MediaCodec::from_filename));
                Ok(Payload::Media(
                    MediaSource::Bytes(self.whole(path, size).await?),
                    codec,
                ))
            }
        }
    }

    /// A file's bytes, refused before they are read once they exceed what one request may carry.
    async fn whole(&self, path: &Path, size: u64) -> Result<Vec<u8>, ToolError> {
        self.analyzer.inline_limit().refuse(size)?;
        read_range(path, 0..size).await
    }

    /// The text page: the file's own line count with the requested window read from it.
    async fn window(&self, path: &Path, args: &ReadArgs) -> Result<ReadOutput, ToolError> {
        let total = line_count(path).await?;
        if total == 0 {
            return Ok(ReadOutput::Paged(Page::lines(String::new(), 0, 0, 0)));
        }
        let start = args.offset.unwrap_or(1).max(1);
        if start > total {
            return Ok(ReadOutput::Paged(Page::lines_beyond(
                start as usize - 1,
                total as usize,
            )));
        }
        let count = window_lines(args.limit);
        let lines = lines(path, start..start + count).await?;
        let shown = lines.len();
        let content = lines
            .into_iter()
            .map(truncate_line)
            .collect::<Vec<_>>()
            .join("\n");
        Ok(ReadOutput::Paged(Page::lines(
            content,
            start as usize - 1,
            total as usize,
            shown,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agent::runtime::sandbox::Sandbox, config::schema::SandboxConfig, test_support::TempDir,
    };

    #[test]
    fn a_zero_limit_reads_one_line() {
        assert_eq!(window_lines(Some(0)), 1);
        assert_eq!(window_lines(None), DEFAULT_LINES);
        assert_eq!(window_lines(Some(MAX_WINDOW_LINES + 1)), MAX_WINDOW_LINES);
    }

    #[test]
    fn binary_detection() {
        assert!(looks_binary(b"PK\x03\x04\x00rest"));
        assert!(!looks_binary("café — naïve".as_bytes()));
    }

    #[test]
    fn a_long_line_is_cut_and_marked() {
        let line = "→".repeat(MAX_LINE_CHARS + 4);
        let cut = truncate_line(line);
        assert!(cut.starts_with('→'));
        assert!(cut.ends_with("… [truncated 4 chars]"));
        assert_eq!(truncate_line("short".to_owned()), "short");
    }

    #[test]
    fn an_entry_shows_its_kind_and_size() {
        let dir = Entry {
            name: "notes".to_owned(),
            kind: EntryKind::Dir,
            size: None,
        };
        let file = Entry {
            name: "a.md".to_owned(),
            kind: EntryKind::File,
            size: Some(12),
        };
        assert_eq!(render_entry(&dir), "notes/");
        assert_eq!(render_entry(&file), "a.md (12)");
    }

    #[tokio::test]
    async fn the_file_operations_read_windows_ranges_and_counts() {
        let dir = TempDir::create("hai-read-window");
        let file = dir.path().join("notes.md");
        tokio::fs::write(&file, "alpha\nbeta\ngamma")
            .await
            .expect("write");

        let file_stat = stat(&file).await.expect("stat file");
        assert_eq!(file_stat.kind, EntryKind::File);
        assert_eq!(file_stat.size, 16);
        assert_eq!(
            stat(dir.path()).await.expect("stat dir").kind,
            EntryKind::Dir
        );

        // The window is 1-based and half-open, and the count keeps a last line with no newline.
        assert_eq!(line_count(&file).await.expect("count"), 3);
        assert_eq!(
            lines(&file, 2..4).await.expect("window"),
            vec!["beta".to_owned(), "gamma".to_owned()]
        );
        assert_eq!(read_range(&file, 1..3).await.expect("range"), b"lp");
        assert_eq!(read_range(&file, 100..200).await.expect("past end"), b"");
    }

    #[tokio::test]
    async fn a_path_outside_the_mount_is_refused() {
        let mount = TempDir::create("hai-read-policy-mount");
        let (workspace, dir) = crate::agent::test_support::created_workspace().await;
        let notes = dir.path().join("notes.md");
        tokio::fs::write(&notes, "hi").await.expect("write");

        let cfg = SandboxConfig {
            mounts: vec![mount.path().to_path_buf()],
            ..SandboxConfig::default()
        };
        let sandbox = Sandbox::container(&cfg, Vec::new(), workspace);

        assert!(sandbox.ensure_readable(&notes).is_ok());
        assert!(sandbox.ensure_readable(mount.path()).is_ok());
        assert!(sandbox.ensure_readable(Path::new("/etc/hostname")).is_err());
        assert!(
            sandbox
                .ensure_readable(&dir.path().join("missing.md"))
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_symlink_out_of_the_workspace_is_refused() {
        let (workspace, dir) = crate::agent::test_support::created_workspace().await;

        // A link out of the workspace resolves outside it, so it is refused too.
        let escape = dir.path().join("escape");
        std::os::unix::fs::symlink("/etc/hostname", &escape).expect("symlink");
        let sandbox = Sandbox::container(&SandboxConfig::default(), Vec::new(), workspace);
        assert!(sandbox.ensure_readable(&escape).is_err());
    }
}
