use crate::{SyntaxKind, SyntaxNode, SyntaxToken};

pub fn extract_leading_comments(node: &SyntaxNode, source_text: &str) -> Option<Vec<String>> {
    let node_start: usize = node.text_range().start().into();
    extract_leading_comments_at_offset(node_start, source_text)
}

pub fn extract_leading_comments_at_offset(offset: usize, source_text: &str) -> Option<Vec<String>> {
    if offset > source_text.len() {
        return None;
    }

    let text_before_node = &source_text[..offset];

    let mut comments = Vec::new();

    // Walk lines backwards from the offset. The documentation block is only
    // the contiguous run of `//` lines directly above the method: a blank
    // line or code breaks it, so an unrelated comment further up (e.g. a
    // change-log marker) is never attached as the method's documentation.
    let mut lines = text_before_node.rsplit('\n');

    // The first fragment is the method's own line up to the offset
    // (indentation before the keyword or annotation); it is not a separate
    // source line and must not terminate the scan.
    if let Some(fragment) = lines.next() {
        let trimmed = fragment.trim();
        if !trimmed.is_empty() && !trimmed.starts_with("//") {
            return None;
        }
    }

    for line in lines {
        let trimmed = line.trim();

        if let Some(comment_text) = trimmed.strip_prefix("//") {
            let comment_text = comment_text.trim();
            if !comment_text.is_empty() {
                comments.push(comment_text.to_string());
            }
        } else {
            break;
        }
    }

    if comments.is_empty() {
        return None;
    }

    comments.reverse();
    Some(comments)
}

pub fn has_trailing_comment(node: &SyntaxNode, source_text: &str) -> bool {
    let node_range = node.text_range();
    let node_end: usize = node_range.end().into();

    if node_end >= source_text.len() {
        return false;
    }

    let text_after = &source_text[node_end..];
    let mut chars = text_after.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\n' | '\r' => return false,
            '/' => {
                if chars.peek() == Some(&'/') {
                    return true;
                }
                return false;
            }
            ' ' | '\t' => continue,
            _ => return false,
        }
    }
    false
}

pub fn has_variable_leading_description(
    var_keyword_offset: usize,
    source_text: &str,
    first_annotation_offset: Option<usize>,
) -> bool {
    let check_from = first_annotation_offset.unwrap_or(var_keyword_offset);

    if check_from == 0 || check_from > source_text.len() {
        return false;
    }

    let text_before = &source_text[..check_from];

    // Backwards scan from the anchor: the loop exits on the first decisive
    // line, so the cost is bounded by the annotation/comment block, not by
    // the length of the preceding text.
    let mut rev_lines = text_before.rsplit('\n');
    let last_line = rev_lines.next().unwrap_or("");
    let trimmed_last = last_line.trim();
    let skip_last = trimmed_last.is_empty() || trimmed_last.starts_with('&');

    for line in (!skip_last).then_some(last_line).into_iter().chain(rev_lines) {
        let line = line.trim();

        if line.starts_with("//") {
            return true;
        }

        if line.is_empty() {
            return false;
        }

        if line.starts_with('&') {
            continue;
        }

        return false;
    }

    false
}

pub fn has_variable_description(
    node: &SyntaxNode,
    var_keyword_offset: usize,
    source_text: &str,
    first_annotation_offset: Option<usize>,
) -> bool {
    if has_trailing_comment(node, source_text) {
        return true;
    }

    if first_annotation_offset.is_some()
        && has_annotation_comments(var_keyword_offset, source_text, first_annotation_offset)
    {
        return true;
    }

    has_variable_leading_description(var_keyword_offset, source_text, first_annotation_offset)
}

pub fn extract_variable_comments_at_offset(
    file_text: &str,
    var_keyword_offset: usize,
    var_end_offset: usize,
    first_annotation_offset: Option<usize>,
) -> Option<Vec<String>> {
    debug_assert!(
        var_keyword_offset == 0 || file_text.is_char_boundary(var_keyword_offset),
        "var_keyword_offset {var_keyword_offset} not on a char boundary"
    );
    debug_assert!(
        var_end_offset == 0 || file_text.is_char_boundary(var_end_offset),
        "var_end_offset {var_end_offset} not on a char boundary"
    );
    debug_assert!(
        first_annotation_offset.is_none_or(|o| o == 0 || file_text.is_char_boundary(o)),
        "first_annotation_offset {first_annotation_offset:?} not on a char boundary"
    );

    let mut comments: Vec<String> = Vec::new();

    let leading_anchor = first_annotation_offset.unwrap_or(var_keyword_offset);
    if let Some(leading) = collect_variable_leading_comments(file_text, leading_anchor) {
        comments.extend(leading);
    }

    if let Some(first_ann) = first_annotation_offset {
        if first_ann < var_keyword_offset && var_keyword_offset <= file_text.len() {
            let block = &file_text[first_ann..var_keyword_offset];
            for line in block.lines() {
                let trimmed = line.trim();
                if let Some(rest) = trimmed.strip_prefix("//") {
                    let comment_text = rest.trim();
                    if !comment_text.is_empty() {
                        comments.push(comment_text.to_string());
                    }
                }
            }
        }
    }

    if let Some(trailing) = scan_variable_trailing_comment(file_text, var_end_offset) {
        comments.push(trailing);
    }

    if comments.is_empty() {
        None
    } else {
        Some(comments)
    }
}

fn collect_variable_leading_comments(file_text: &str, anchor: usize) -> Option<Vec<String>> {
    if anchor == 0 || anchor > file_text.len() {
        return None;
    }

    let text_before = &file_text[..anchor];

    // Same backwards bounded scan as `has_variable_leading_description`.
    let mut rev_lines = text_before.rsplit('\n');
    let last_line = rev_lines.next().unwrap_or("");
    let trimmed_last = last_line.trim();
    let skip_last = trimmed_last.is_empty() || trimmed_last.starts_with('&');

    let mut comments: Vec<String> = Vec::new();
    for line in (!skip_last).then_some(last_line).into_iter().chain(rev_lines) {
        let line = line.trim();

        if let Some(rest) = line.strip_prefix("//") {
            let comment_text = rest.trim();
            if !comment_text.is_empty() {
                comments.push(comment_text.to_string());
            }
            continue;
        }

        if line.is_empty() {
            break;
        }

        if line.starts_with('&') {
            continue;
        }

        break;
    }

    if comments.is_empty() {
        None
    } else {
        comments.reverse();
        Some(comments)
    }
}

fn scan_variable_trailing_comment(file_text: &str, var_end_offset: usize) -> Option<String> {
    if var_end_offset >= file_text.len() {
        return None;
    }
    let text_after = &file_text[var_end_offset..];
    for (i, ch) in text_after.char_indices() {
        match ch {
            '\n' | '\r' => return None,
            ' ' | '\t' => continue,
            '/' => {
                let after_first = &text_after[i + ch.len_utf8()..];
                if !after_first.starts_with('/') {
                    return None;
                }
                let after_slashes = &after_first['/'.len_utf8()..];
                let line = after_slashes.lines().next().unwrap_or("").trim();
                if line.is_empty() {
                    return None;
                }
                return Some(line.to_string());
            }
            _ => return None,
        }
    }
    None
}

fn has_annotation_comments(
    var_keyword_offset: usize,
    source_text: &str,
    first_annotation_offset: Option<usize>,
) -> bool {
    let first_ann = match first_annotation_offset {
        Some(off) => off,
        None => return false,
    };

    if first_ann >= var_keyword_offset {
        return false;
    }

    let annotation_block = &source_text[first_ann..var_keyword_offset];

    for line in annotation_block.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") {
            return true;
        }
    }

    false
}

pub fn field_tail_name_token(field_expr: &SyntaxNode) -> Option<SyntaxToken> {
    if field_expr.kind() != SyntaxKind::FIELD_EXPR {
        return None;
    }
    let mut saw_dot = false;
    field_expr.children_with_tokens().filter_map(|el| el.into_token()).find(|tok| {
        if !saw_dot {
            saw_dot = tok.kind() == SyntaxKind::DOT;
            return false;
        }
        tok.kind().is_name_token()
    })
}

pub fn new_expr_type_name_token(new_expr: &SyntaxNode) -> Option<SyntaxToken> {
    if new_expr.kind() != SyntaxKind::NEW_EXPR {
        return None;
    }
    let mut saw_new = false;
    new_expr.children_with_tokens().filter_map(|el| el.into_token()).find(|tok| {
        if !saw_new {
            saw_new = tok.kind() == SyntaxKind::KW_NEW;
            return false;
        }
        tok.kind().is_name_token()
    })
}

#[cfg(test)]
mod variable_comment_extractor_tests {
    use super::extract_variable_comments_at_offset;

    fn off(text: &str, marker: &str) -> usize {
        text.find(marker).unwrap_or_else(|| panic!("marker {marker:?} not found in {text:?}"))
    }

    #[test]
    fn no_comments_returns_none() {
        let text = "Перем X;";
        let var_kw = off(text, "Перем");
        let var_end = text.len();
        assert_eq!(extract_variable_comments_at_offset(text, var_kw, var_end, None), None);
    }

    #[test]
    fn leading_single_line() {
        let text = "// purpose\nПерем X;";
        let var_kw = off(text, "Перем");
        let var_end = text.len();
        let got = extract_variable_comments_at_offset(text, var_kw, var_end, None).unwrap();
        assert_eq!(got, vec!["purpose".to_string()]);
    }

    #[test]
    fn leading_multiline_block() {
        let text = "// first\n// second\nПерем X;";
        let var_kw = off(text, "Перем");
        let var_end = text.len();
        let got = extract_variable_comments_at_offset(text, var_kw, var_end, None).unwrap();
        assert_eq!(got, vec!["first".to_string(), "second".to_string()]);
    }

    #[test]
    fn blank_line_breaks_leading() {
        let text = "// far away\n\nПерем X;";
        let var_kw = off(text, "Перем");
        let var_end = text.len();
        assert_eq!(extract_variable_comments_at_offset(text, var_kw, var_end, None), None);
    }

    #[test]
    fn trailing_only() {
        let text = "Перем X; // trailing";
        let var_kw = off(text, "Перем");
        let var_end = off(text, ";") + 1;
        let got = extract_variable_comments_at_offset(text, var_kw, var_end, None).unwrap();
        assert_eq!(got, vec!["trailing".to_string()]);
    }

    #[test]
    fn empty_trailing_marker_filtered() {
        let text = "Перем X; //";
        let var_kw = off(text, "Перем");
        let var_end = off(text, ";") + 1;
        assert_eq!(extract_variable_comments_at_offset(text, var_kw, var_end, None), None);
    }

    #[test]
    fn empty_leading_marker_filtered() {
        let text = "//\nПерем X;";
        let var_kw = off(text, "Перем");
        let var_end = text.len();
        assert_eq!(extract_variable_comments_at_offset(text, var_kw, var_end, None), None);
    }

    #[test]
    fn leading_then_trailing_combined() {
        let text = "// purpose\nПерем X; // remark";
        let var_kw = off(text, "Перем");
        let var_end = off(text, ";") + 1;
        let got = extract_variable_comments_at_offset(text, var_kw, var_end, None).unwrap();
        assert_eq!(got, vec!["purpose".to_string(), "remark".to_string()]);
    }

    #[test]
    fn inter_annotation_capture() {
        let text = "&Идентификатор\n// inter\n&Колонка\nПерем X;";
        let var_kw = off(text, "Перем");
        let first_ann = off(text, "&Идентификатор");
        let var_end = text.len();
        let got =
            extract_variable_comments_at_offset(text, var_kw, var_end, Some(first_ann)).unwrap();
        assert_eq!(got, vec!["inter".to_string()]);
    }

    #[test]
    fn leading_above_first_annotation() {
        let text = "// header\n&Идентификатор\nПерем X;";
        let var_kw = off(text, "Перем");
        let first_ann = off(text, "&Идентификатор");
        let var_end = text.len();
        let got =
            extract_variable_comments_at_offset(text, var_kw, var_end, Some(first_ann)).unwrap();
        assert_eq!(got, vec!["header".to_string()]);
    }

    #[test]
    fn trailing_with_annotations() {
        let text = "&Идентификатор\nПерем X; // tail";
        let var_kw = off(text, "Перем");
        let first_ann = off(text, "&Идентификатор");
        let var_end = off(text, ";") + 1;
        let got =
            extract_variable_comments_at_offset(text, var_kw, var_end, Some(first_ann)).unwrap();
        assert_eq!(got, vec!["tail".to_string()]);
    }

    #[test]
    fn leading_blank_above_annotation_breaks_connection() {
        let text = "// orphan\n\n&Идентификатор\nПерем X;";
        let var_kw = off(text, "Перем");
        let first_ann = off(text, "&Идентификатор");
        let var_end = text.len();
        assert_eq!(
            extract_variable_comments_at_offset(text, var_kw, var_end, Some(first_ann)),
            None
        );
    }

    #[test]
    fn crlf_line_endings_are_handled() {
        let text = "// purpose\r\nПерем X;\r\n";
        let var_kw = off(text, "Перем");
        let var_end = off(text, ";") + 1;
        let got = extract_variable_comments_at_offset(text, var_kw, var_end, None).unwrap();
        assert_eq!(got, vec!["purpose".to_string()]);
    }

    #[test]
    fn cyrillic_variable_name_offsets() {
        let text = "// заголовок\nПерем СчётчикВызовов; // примечание";
        let var_kw = off(text, "Перем");
        let var_end = off(text, ";") + 1;
        let got = extract_variable_comments_at_offset(text, var_kw, var_end, None).unwrap();
        assert_eq!(got, vec!["заголовок".to_string(), "примечание".to_string()]);
    }

    #[test]
    fn all_three_regions_combined() {
        let text = "// header\n&Идентификатор\n// inter\n&Колонка\nПерем X; // tail";
        let var_kw = off(text, "Перем");
        let first_ann = off(text, "&Идентификатор");
        let var_end = off(text, ";") + 1;
        let got =
            extract_variable_comments_at_offset(text, var_kw, var_end, Some(first_ann)).unwrap();
        assert_eq!(got, vec!["header".to_string(), "inter".to_string(), "tail".to_string()]);
    }
}

#[cfg(test)]
mod leading_comment_scan_tests {
    use super::{extract_leading_comments_at_offset, has_variable_leading_description};

    fn off(text: &str, marker: &str) -> usize {
        text.find(marker).unwrap_or_else(|| panic!("marker {marker:?} not found in {text:?}"))
    }

    #[test]
    fn comment_block_above_method() {
        let text = "// Описание.\n// Вторая строка.\nПроцедура П()";
        let got = extract_leading_comments_at_offset(off(text, "Процедура"), text).unwrap();
        assert_eq!(got, vec!["Описание.".to_string(), "Вторая строка.".to_string()]);
    }

    #[test]
    fn blank_line_above_method_detaches_the_block() {
        let text = "// первый\n\n// второй\n\nПроцедура П()";
        assert_eq!(extract_leading_comments_at_offset(off(text, "Процедура"), text), None);
    }

    #[test]
    fn blank_line_inside_block_keeps_only_adjacent_part() {
        let text = "// далёкий\n\n// ближний\nПроцедура П()";
        let got = extract_leading_comments_at_offset(off(text, "Процедура"), text).unwrap();
        assert_eq!(got, vec!["ближний".to_string()]);
    }

    #[test]
    fn code_line_stops_the_scan() {
        let text = "// далёкий\nКонецПроцедуры\n\n// ближний\nПроцедура П()";
        let got = extract_leading_comments_at_offset(off(text, "Процедура"), text).unwrap();
        assert_eq!(got, vec!["ближний".to_string()]);
    }

    #[test]
    fn crlf_comments_are_trimmed() {
        let text = "// заметка\r\nПроцедура П()";
        let got = extract_leading_comments_at_offset(off(text, "Процедура"), text).unwrap();
        assert_eq!(got, vec!["заметка".to_string()]);
    }

    #[test]
    fn comment_at_file_start() {
        let text = "// шапка\nПроцедура П()";
        let got = extract_leading_comments_at_offset(off(text, "Процедура"), text).unwrap();
        assert_eq!(got, vec!["шапка".to_string()]);
    }

    #[test]
    fn no_comments_returns_none() {
        let text = "КонецПроцедуры\nПроцедура П()";
        assert_eq!(extract_leading_comments_at_offset(off(text, "Процедура П"), text), None);
        assert_eq!(extract_leading_comments_at_offset(0, text), None);
    }

    #[test]
    fn comments_above_annotation_anchor_are_attached() {
        // Callers anchor the offset at the annotation when a method has one,
        // so the doc block right above the annotation is found.
        let text = "// Описание.\n&НаСервере\nПроцедура П()";
        let got = extract_leading_comments_at_offset(off(text, "&НаСервере"), text).unwrap();
        assert_eq!(got, vec!["Описание.".to_string()]);
    }

    #[test]
    fn code_before_offset_on_same_line_returns_none() {
        let text = "// Описание.\nПерем А; Процедура П()";
        assert_eq!(extract_leading_comments_at_offset(off(text, "Процедура"), text), None);
    }

    #[test]
    fn empty_marker_comments_are_dropped() {
        let text = "//\n// текст\n//\nПроцедура П()";
        let got = extract_leading_comments_at_offset(off(text, "Процедура"), text).unwrap();
        assert_eq!(got, vec!["текст".to_string()]);
    }

    #[test]
    fn offset_past_text_returns_none() {
        assert_eq!(extract_leading_comments_at_offset(100, "короткий"), None);
    }

    #[test]
    fn variable_description_above_annotation() {
        let text = "// назначение\n&НаКлиенте\nПерем X;";
        assert!(has_variable_leading_description(off(text, "Перем"), text, Some(off(text, "&"))));
    }

    #[test]
    fn variable_without_description() {
        let text = "КонецПроцедуры\n&НаКлиенте\nПерем X;";
        assert!(!has_variable_leading_description(off(text, "Перем"), text, Some(off(text, "&"))));
    }

    #[test]
    fn annotation_on_first_line_only() {
        let text = "&НаКлиенте\nПерем X;";
        assert!(!has_variable_leading_description(off(text, "Перем"), text, Some(0)));
    }

    #[test]
    fn anchor_at_zero_returns_false() {
        assert!(!has_variable_leading_description(0, "Перем X;", None));
    }
}
