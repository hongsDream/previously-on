use std::io::{self, BufRead};

use tokio::io::{AsyncBufRead, AsyncBufReadExt};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BoundedLine {
    Eof,
    Line(Vec<u8>),
    TooLong,
}

/// Read one newline-delimited frame while never retaining more than `limit` bytes.
///
/// When `drain_oversized` is false, an oversized frame is rejected as soon as the limit is
/// crossed. Queue readers pass true so one corrupt record cannot prevent later records from being
/// replayed; the discarded suffix is consumed directly from the reader buffer and is never copied.
pub(crate) fn read_bounded_line(
    reader: &mut dyn BufRead,
    limit: usize,
    drain_oversized: bool,
) -> io::Result<BoundedLine> {
    let mut line = Vec::with_capacity(limit.min(8 * 1024));
    let mut saw_input = false;
    let mut too_long = false;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(if too_long {
                BoundedLine::TooLong
            } else if saw_input {
                BoundedLine::Line(line)
            } else {
                BoundedLine::Eof
            });
        }
        saw_input = true;
        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |index| index + 1);
        if !too_long {
            let remaining = limit.saturating_sub(line.len());
            if consumed <= remaining {
                line.extend_from_slice(&available[..consumed]);
            } else {
                line.extend_from_slice(&available[..remaining]);
                too_long = true;
            }
        }
        reader.consume(consumed);
        if newline.is_some() {
            return Ok(if too_long {
                BoundedLine::TooLong
            } else {
                BoundedLine::Line(line)
            });
        }
        if too_long && !drain_oversized {
            return Ok(BoundedLine::TooLong);
        }
    }
}

pub(crate) async fn read_bounded_line_async<R>(
    reader: &mut R,
    limit: usize,
    drain_oversized: bool,
) -> io::Result<BoundedLine>
where
    R: AsyncBufRead + Unpin + ?Sized,
{
    let mut line = Vec::with_capacity(limit.min(8 * 1024));
    let mut saw_input = false;
    let mut too_long = false;
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok(if too_long {
                BoundedLine::TooLong
            } else if saw_input {
                BoundedLine::Line(line)
            } else {
                BoundedLine::Eof
            });
        }
        saw_input = true;
        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |index| index + 1);
        if !too_long {
            let remaining = limit.saturating_sub(line.len());
            if consumed <= remaining {
                line.extend_from_slice(&available[..consumed]);
            } else {
                line.extend_from_slice(&available[..remaining]);
                too_long = true;
            }
        }
        reader.consume(consumed);
        if newline.is_some() {
            return Ok(if too_long {
                BoundedLine::TooLong
            } else {
                BoundedLine::Line(line)
            });
        }
        if too_long && !drain_oversized {
            return Ok(BoundedLine::TooLong);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};

    #[test]
    fn sync_reader_discards_an_oversized_record_without_losing_the_next_line() {
        let mut input = vec![b'x'; 32 * 1024];
        input.extend_from_slice(b"\nnext\n");
        let mut reader = BufReader::with_capacity(17, Cursor::new(input));

        assert_eq!(
            read_bounded_line(&mut reader, 128, true).unwrap(),
            BoundedLine::TooLong
        );
        assert_eq!(
            read_bounded_line(&mut reader, 128, true).unwrap(),
            BoundedLine::Line(b"next\n".to_vec())
        );
    }

    #[tokio::test]
    async fn async_reader_rejects_an_oversized_unterminated_frame_at_the_limit() {
        let input = vec![b'x'; 32 * 1024];
        let mut reader = tokio::io::BufReader::with_capacity(13, Cursor::new(input));

        assert_eq!(
            read_bounded_line_async(&mut reader, 128, false)
                .await
                .unwrap(),
            BoundedLine::TooLong
        );
        assert!(reader.buffer().len() <= 13);
    }
}
