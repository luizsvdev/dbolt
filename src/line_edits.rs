use std::ops::Range;

const COMMENT_PREFIX: &str = "--";

/// One change to the text: `replace` is the byte range to swap for `text`,
/// and `selection` is where the selection goes afterwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineEdit {
    pub replace: Range<usize>,
    pub text: String,
    pub selection: Range<usize>,
}

/// The whole lines the selection touches, without the final newline. A
/// selection ending at the start of a line does not include that line.
fn line_block(text: &str, selection: &Range<usize>) -> Range<usize> {
    let start = selection
        .start
        .min(text.len());
    let mut end = selection
        .end
        .min(text.len())
        .max(start);
    if end > start && text[..end].ends_with('\n') {
        end -= 1;
    }
    let block_start = text[..start]
        .rfind('\n')
        .map_or(0, |newline| newline + 1);
    let block_end = text[end..]
        .find('\n')
        .map_or(text.len(), |newline| end + newline);
    block_start..block_end.max(block_start)
}

fn shifted(selection: &Range<usize>, by: isize) -> Range<usize> {
    let shift = |offset: usize| offset.saturating_add_signed(by);
    shift(selection.start)..shift(selection.end)
}

/// Comments the lines with `-- `, or uncomments them when every non-blank
/// line already starts with `--`.
pub fn toggle_comment(text: &str, selection: Range<usize>) -> Option<LineEdit> {
    let block = line_block(text, &selection);
    let lines: Vec<&str> = text
        .get(block.clone())?
        .split('\n')
        .collect();
    let code_lines = || {
        lines
            .iter()
            .filter(|line| !line.trim().is_empty())
    };
    let commented = code_lines().next().is_some()
        && code_lines().all(|line| {
            line.trim_start()
                .starts_with(COMMENT_PREFIX)
        });
    let indent = code_lines()
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);

    let mut cursor_shift = 0isize;
    let cursor_line = text[block.start
        ..selection
            .start
            .min(block.end)
            .max(block.start)]
        .matches('\n')
        .count();
    let new_lines: Vec<String> = lines
        .iter()
        .enumerate()
        .map(|(line_ix, line)| {
            let (new_line, delta) = if line.trim().is_empty() {
                (line.to_string(), 0)
            } else if commented {
                let content_start = line.len() - line.trim_start().len();
                let rest = &line[content_start + COMMENT_PREFIX.len()..];
                let rest = rest
                    .strip_prefix(' ')
                    .unwrap_or(rest);
                let new_line = format!("{}{rest}", &line[..content_start]);
                let delta = new_line.len() as isize - line.len() as isize;
                (new_line, delta)
            } else {
                let indent = indent.min(line.len());
                (
                    format!("{}{COMMENT_PREFIX} {}", &line[..indent], &line[indent..]),
                    COMMENT_PREFIX.len() as isize + 1,
                )
            };
            if line_ix == cursor_line {
                cursor_shift = delta;
            }
            new_line
        })
        .collect();
    let new_text = new_lines.join("\n");
    let selection = if selection.is_empty() {
        shifted(&selection, cursor_shift)
    } else {
        block.start..block.start + new_text.len()
    };
    Some(LineEdit {
        replace: block,
        text: new_text,
        selection,
    })
}

/// Swaps the selected lines with the line above.
pub fn move_lines_up(text: &str, selection: Range<usize>) -> Option<LineEdit> {
    let block = line_block(text, &selection);
    let previous_end = block.start.checked_sub(1)?;
    let previous_start = text[..previous_end]
        .rfind('\n')
        .map_or(0, |newline| newline + 1);
    let previous = &text[previous_start..previous_end];
    let lines = &text[block.clone()];
    Some(LineEdit {
        replace: previous_start..block.end,
        text: format!("{lines}\n{previous}"),
        selection: shifted(&selection, -(previous.len() as isize + 1)),
    })
}

/// Swaps the selected lines with the line below.
pub fn move_lines_down(text: &str, selection: Range<usize>) -> Option<LineEdit> {
    let block = line_block(text, &selection);
    let next_start = block.end + 1;
    if next_start > text.len() {
        return None;
    }
    let next_end = text[next_start..]
        .find('\n')
        .map_or(text.len(), |newline| next_start + newline);
    let next = &text[next_start..next_end];
    let lines = &text[block.clone()];
    Some(LineEdit {
        replace: block.start..next_end,
        text: format!("{next}\n{lines}"),
        selection: shifted(&selection, next.len() as isize + 1),
    })
}

/// Copies the selected lines below themselves and moves the selection onto
/// the copy.
pub fn duplicate_lines(text: &str, selection: Range<usize>) -> Option<LineEdit> {
    let block = line_block(text, &selection);
    let lines = &text[block.clone()];
    Some(LineEdit {
        replace: block.end..block.end,
        text: format!("\n{lines}"),
        selection: shifted(&selection, lines.len() as isize + 1),
    })
}

/// Removes the selected lines and leaves the cursor at the start of the line
/// that takes their place.
pub fn delete_lines(text: &str, selection: Range<usize>) -> Option<LineEdit> {
    if text.is_empty() {
        return None;
    }
    let block = line_block(text, &selection);
    let (replace, cursor) = if block.end < text.len() {
        (block.start..block.end + 1, block.start)
    } else {
        // The last line has no newline after it, so the one before it goes.
        let start = block.start.saturating_sub(1);
        let cursor = text[..start]
            .rfind('\n')
            .map_or(0, |newline| newline + 1);
        (start..block.end, cursor)
    };
    Some(LineEdit {
        replace,
        text: String::new(),
        selection: cursor..cursor,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        LineEdit, delete_lines, duplicate_lines, move_lines_down, move_lines_up, toggle_comment,
    };

    fn apply(text: &str, edit: &LineEdit) -> String {
        format!(
            "{}{}{}",
            &text[..edit.replace.start],
            edit.text,
            &text[edit.replace.end..]
        )
    }

    #[test]
    fn toggle_comment_adds_and_removes_the_prefix_at_the_shared_indent() -> Result<(), &'static str>
    {
        let text = "  select 1\n    from t";
        let edit = toggle_comment(text, 0..text.len()).ok_or("no edit")?;
        let commented = apply(text, &edit);
        assert_eq!(commented, "  -- select 1\n  --   from t");
        let edit = toggle_comment(&commented, 0..commented.len()).ok_or("no edit")?;
        assert_eq!(apply(&commented, &edit), text);
        Ok(())
    }

    #[test]
    fn toggle_comment_keeps_the_cursor_on_its_character() -> Result<(), &'static str> {
        let text = "select 1";
        let edit = toggle_comment(text, 7..7).ok_or("no edit")?;
        assert_eq!(edit.selection, 10..10);
        assert_eq!(&apply(text, &edit)[10..], "1");
        Ok(())
    }

    #[test]
    fn lines_move_up_and_down_with_the_cursor() -> Result<(), &'static str> {
        let text = "a\nbb\nc";
        let edit = move_lines_up(text, 3..3).ok_or("no edit")?;
        assert_eq!(apply(text, &edit), "bb\na\nc");
        assert_eq!(edit.selection, 1..1);
        let edit = move_lines_down(text, 3..3).ok_or("no edit")?;
        assert_eq!(apply(text, &edit), "a\nc\nbb");
        assert_eq!(edit.selection, 5..5);
        assert!(move_lines_up(text, 0..0).is_none());
        assert!(move_lines_down(text, 5..5).is_none());
        Ok(())
    }

    #[test]
    fn duplicate_copies_the_selected_lines_below() -> Result<(), &'static str> {
        let text = "a\nb";
        let edit = duplicate_lines(text, 0..1).ok_or("no edit")?;
        assert_eq!(apply(text, &edit), "a\na\nb");
        assert_eq!(edit.selection, 2..3);
        Ok(())
    }

    #[test]
    fn delete_removes_whole_lines_including_the_last_one() -> Result<(), &'static str> {
        let text = "a\nb\nc";
        let edit = delete_lines(text, 2..2).ok_or("no edit")?;
        assert_eq!(apply(text, &edit), "a\nc");
        assert_eq!(edit.selection, 2..2);
        let edit = delete_lines(text, 4..4).ok_or("no edit")?;
        assert_eq!(apply(text, &edit), "a\nb");
        assert_eq!(edit.selection, 2..2);
        Ok(())
    }

    #[test]
    fn a_selection_ending_at_a_line_start_leaves_that_line_alone() -> Result<(), &'static str> {
        let text = "a\nb\nc";
        let edit = delete_lines(text, 0..2).ok_or("no edit")?;
        assert_eq!(apply(text, &edit), "b\nc");
        Ok(())
    }
}
