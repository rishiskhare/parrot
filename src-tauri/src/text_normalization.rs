use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

pub fn normalize_text_for_tts(text: &str) -> String {
    if text.trim().is_empty() {
        return String::new();
    }

    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_SMART_PUNCTUATION);

    let parser = Parser::new_ext(text, options);
    let mut renderer = SpeechTextRenderer::default();

    for event in parser {
        renderer.handle_event(event);
    }

    renderer.finish()
}

#[derive(Default)]
struct SpeechTextRenderer {
    output: String,
    pending_breaks: usize,
    list_stack: Vec<ListContext>,
    link_stack: Vec<LinkContext>,
    image_stack: Vec<ImageContext>,
    quote_depth: usize,
    skip_code_block_depth: usize,
    table_cell_index: usize,
}

#[derive(Clone, Debug)]
struct ListContext {
    next_index: Option<u64>,
}

#[derive(Clone, Debug)]
struct LinkContext {
    destination: String,
    text: String,
}

#[derive(Clone, Debug, Default)]
struct ImageContext {
    alt_text: String,
}

impl SpeechTextRenderer {
    fn handle_event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start_tag(tag),
            Event::End(tag) => self.end_tag(tag),
            Event::Text(text) | Event::InlineMath(text) | Event::DisplayMath(text) => {
                if self.skip_code_block_depth == 0 {
                    self.push_text(&text);
                }
            }
            Event::Code(code) => self.push_text(&simplify_inline_code(&code)),
            Event::Html(html) | Event::InlineHtml(html) => {
                if self.skip_code_block_depth == 0 {
                    self.push_text(&strip_html_tags(&html));
                }
            }
            Event::SoftBreak => self.push_space(),
            Event::HardBreak => self.push_block_break(),
            Event::Rule => {
                self.ensure_terminal_punctuation();
                self.push_block_break();
            }
            Event::TaskListMarker(checked) => {
                self.push_text(if checked {
                    "Completed."
                } else {
                    "Not completed."
                });
            }
            Event::FootnoteReference(name) => self.push_text(&normalize_inline_whitespace(&name)),
        }
    }

    fn start_tag(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {}
            Tag::Heading { .. } => self.push_block_break(),
            Tag::BlockQuote(_) => {
                self.push_block_break();
                if self.quote_depth == 0 {
                    self.push_text("Quote:");
                }
                self.quote_depth += 1;
            }
            Tag::CodeBlock(_) => {
                self.push_block_break();
                self.push_text("Code example omitted.");
                self.push_block_break();
                self.skip_code_block_depth += 1;
            }
            Tag::List(start) => {
                self.push_block_break();
                self.list_stack.push(ListContext { next_index: start });
            }
            Tag::Item => {
                self.push_block_break();
                if let Some(list) = self.list_stack.last_mut() {
                    if let Some(index) = list.next_index.as_mut() {
                        let current = *index;
                        *index += 1;
                        self.push_text(&format!("{}.", current));
                    }
                }
            }
            Tag::Link { dest_url, .. } => {
                self.link_stack.push(LinkContext {
                    destination: dest_url.into_string(),
                    text: String::new(),
                });
            }
            Tag::Image { .. } => {
                self.image_stack.push(ImageContext::default());
            }
            Tag::Table(_) => self.push_block_break(),
            Tag::TableHead => self.push_block_break(),
            Tag::TableRow => {
                self.push_block_break();
                self.table_cell_index = 0;
            }
            Tag::TableCell => {
                if self.table_cell_index > 0 {
                    self.push_text(",");
                }
                self.table_cell_index += 1;
            }
            Tag::Emphasis
            | Tag::Strong
            | Tag::Strikethrough
            | Tag::FootnoteDefinition(_)
            | Tag::MetadataBlock(_) => {}
            _ => {}
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.ensure_terminal_punctuation();
                self.push_block_break();
            }
            TagEnd::Heading(_) => {
                self.ensure_terminal_punctuation();
                self.push_block_break();
            }
            TagEnd::BlockQuote(_) => {
                self.quote_depth = self.quote_depth.saturating_sub(1);
                self.ensure_terminal_punctuation();
                self.push_block_break();
            }
            TagEnd::CodeBlock => {
                self.skip_code_block_depth = self.skip_code_block_depth.saturating_sub(1);
            }
            TagEnd::List(_) => {
                self.list_stack.pop();
                self.push_block_break();
            }
            TagEnd::Item => {
                self.ensure_terminal_punctuation();
                self.push_block_break();
            }
            TagEnd::Link => {
                if let Some(link) = self.link_stack.pop() {
                    let label = normalize_inline_whitespace(&link.text);
                    let spoken = if label.is_empty()
                        || normalize_inline_whitespace(&link.destination)
                            .eq_ignore_ascii_case(&label)
                    {
                        simplify_url_for_speech(&link.destination)
                    } else {
                        label
                    };
                    self.push_text(&spoken);
                }
            }
            TagEnd::Image => {
                if let Some(image) = self.image_stack.pop() {
                    let alt_text = normalize_inline_whitespace(&image.alt_text);
                    if !alt_text.is_empty() {
                        self.push_text(&alt_text);
                    }
                }
            }
            TagEnd::Table => self.push_block_break(),
            TagEnd::TableHead => self.push_block_break(),
            TagEnd::TableRow => {
                self.ensure_terminal_punctuation();
                self.push_block_break();
                self.table_cell_index = 0;
            }
            TagEnd::TableCell
            | TagEnd::Emphasis
            | TagEnd::Strong
            | TagEnd::Strikethrough
            | TagEnd::FootnoteDefinition
            | TagEnd::MetadataBlock(_) => {}
            _ => {}
        }
    }

    fn push_text(&mut self, text: &str) {
        let had_leading_whitespace = text.chars().next().map(char::is_whitespace).unwrap_or(false);
        let normalized = normalize_inline_whitespace(text);
        if normalized.is_empty() {
            return;
        }

        if let Some(image) = self.image_stack.last_mut() {
            append_segment(&mut image.alt_text, &normalized, had_leading_whitespace);
            return;
        }

        if let Some(link) = self.link_stack.last_mut() {
            append_segment(&mut link.text, &normalized, had_leading_whitespace);
            return;
        }

        self.flush_breaks();

        if should_preserve_leading_space(had_leading_whitespace, normalized.chars().next())
            && !self.output.is_empty()
            && !self.output.ends_with(char::is_whitespace)
        {
            self.output.push(' ');
        } else if needs_space_between(
            self.output.chars().rev().nth(1),
            self.output.chars().next_back(),
            normalized.chars().next(),
        ) {
            self.output.push(' ');
        }
        self.output.push_str(&normalized);
    }

    fn push_space(&mut self) {
        if self.pending_breaks > 0
            || self.output.is_empty()
            || self.output.ends_with(char::is_whitespace)
        {
            return;
        }
        self.output.push(' ');
    }

    fn push_block_break(&mut self) {
        if self.output.trim().is_empty() {
            return;
        }
        self.pending_breaks = self.pending_breaks.max(2);
    }

    fn flush_breaks(&mut self) {
        if self.pending_breaks == 0 {
            return;
        }

        self.output.truncate(self.output.trim_end().len());
        if self.output.is_empty() {
            self.pending_breaks = 0;
            return;
        }

        for _ in 0..self.pending_breaks {
            self.output.push('\n');
        }
        self.pending_breaks = 0;
    }

    fn ensure_terminal_punctuation(&mut self) {
        if self.output.trim().is_empty() {
            return;
        }

        self.output.truncate(self.output.trim_end().len());

        let Some(last) = self.output.chars().next_back() else {
            return;
        };

        if matches!(last, '.' | '!' | '?' | ':' | ';' | '…') {
            return;
        }

        self.output.push('.');
    }

    fn finish(mut self) -> String {
        self.ensure_terminal_punctuation();

        let mut result = String::new();
        let mut blank_line_count = 0usize;

        for line in self.output.lines().map(str::trim) {
            if line.is_empty() {
                blank_line_count += 1;
                if blank_line_count > 1 {
                    continue;
                }
            } else {
                blank_line_count = 0;
            }

            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(line);
        }

        normalize_quote_spacing(result.trim())
    }
}

fn append_segment(buffer: &mut String, segment: &str, had_leading_whitespace: bool) {
    if segment.is_empty() {
        return;
    }

    if should_preserve_leading_space(had_leading_whitespace, segment.chars().next())
        && !buffer.is_empty()
        && !buffer.ends_with(char::is_whitespace)
    {
        buffer.push(' ');
    } else if needs_space_between(
        buffer.chars().rev().nth(1),
        buffer.chars().next_back(),
        segment.chars().next(),
    ) {
        buffer.push(' ');
    }
    buffer.push_str(segment);
}

fn normalize_inline_whitespace(text: &str) -> String {
    let mut normalized = String::new();
    let mut last_was_space = false;

    for ch in text.chars() {
        if ch.is_whitespace() {
            if !last_was_space {
                normalized.push(' ');
                last_was_space = true;
            }
        } else {
            normalized.push(ch);
            last_was_space = false;
        }
    }

    normalized.trim().to_string()
}

fn should_preserve_leading_space(had_leading_whitespace: bool, first: Option<char>) -> bool {
    had_leading_whitespace
        && !matches!(
            first,
            Some(',' | '.' | '!' | '?' | ':' | ';' | ')' | ']' | '}' | '"' | '”' | '’')
        )
}

fn normalize_quote_spacing(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());

    for (idx, &ch) in chars.iter().enumerate() {
        let prev = idx.checked_sub(1).and_then(|i| chars.get(i)).copied();
        let next = chars.get(idx + 1).copied();

        if ch == ' ' {
            let prev_non_space = chars[..idx]
                .iter()
                .rev()
                .copied()
                .find(|c| !c.is_whitespace());
            let next_non_space = chars[idx + 1..]
                .iter()
                .copied()
                .find(|c| !c.is_whitespace());

            if let Some(next_quote) = next_non_space.filter(|c| is_quote_char(*c)) {
                let next_quote_idx = idx + 1
                    + chars[idx + 1..]
                        .iter()
                        .position(|c| !c.is_whitespace())
                        .unwrap_or(0);
                let after_next_quote = chars[next_quote_idx + 1..]
                    .iter()
                    .copied()
                    .find(|c| !c.is_whitespace());

                if is_opening_quote(next_quote, prev_non_space, after_next_quote)
                    && prev_non_space
                        .map(should_trim_space_before_opening_quote)
                        .unwrap_or(false)
                {
                    continue;
                }
            }

            if prev
                .filter(|c| is_quote_char(*c))
                .map(|prev_quote| {
                    let before_prev_quote = chars[..idx.saturating_sub(1)]
                        .iter()
                        .rev()
                        .copied()
                        .find(|c| !c.is_whitespace());
                    is_opening_quote(prev_quote, before_prev_quote, next_non_space)
                })
                .unwrap_or(false)
                && next.map(|c| !c.is_whitespace()).unwrap_or(false)
            {
                continue;
            }
        }

        if matches!(ch, '‘' | '’')
            && next.map(|c| c.is_alphanumeric()).unwrap_or(false)
            && prev.map(|c| c == ':' || c == ';').unwrap_or(false)
            && !out.ends_with(' ')
        {
            out.push(' ');
        }

        out.push(ch);
    }

    out.replace(":’", ": ’")
        .replace("’and", "’ and")
        .replace("a“", "a “")
        .replace(" ”", "”")
}

fn is_opening_quote(ch: char, prev: Option<char>, next: Option<char>) -> bool {
    match ch {
        '“' | '‘' => true,
        '”' => false,
        '"' | '’' => {
            !prev.map(is_quote_word_char).unwrap_or(false)
                && next.map(is_quote_word_char).unwrap_or(false)
        }
        _ => false,
    }
}

fn is_quote_char(ch: char) -> bool {
    matches!(ch, '"' | '“' | '”' | '‘' | '’')
}

fn is_quote_word_char(ch: char) -> bool {
    ch.is_alphanumeric()
}

fn should_trim_space_before_opening_quote(prev: char) -> bool {
    matches!(prev, '"' | '“' | '‘' | '(' | '[' | '{')
}

fn needs_space_between(prev_left: Option<char>, left: Option<char>, right: Option<char>) -> bool {
    match (prev_left, left, right) {
        (prev_left, Some('"' | '“' | '‘'), Some(right)) if right.is_alphanumeric() => {
            match prev_left {
                None => false,
                Some(ch)
                    if ch.is_whitespace()
                        || matches!(ch, '(' | '[' | '{' | '"' | '“' | '‘' | ':' | ';' | '—' | '–') =>
                {
                    false
                }
                Some(_) => true,
            }
        }
        (_, Some(left), Some(right @ ('"' | '”' | '’'))) if left.is_alphanumeric() => {
            !matches!(right, '’')
        }
        (_, Some(left), Some(right))
            if left.is_alphanumeric() && matches!(right, '&' | '\'') =>
        {
            false
        }
        (_, Some('&'), Some(right)) if right.is_alphanumeric() => false,
        (Some(prev_left), Some('\'' | '’'), Some(right))
            if prev_left.is_alphanumeric() && right.is_alphanumeric() =>
        {
            false
        }
        (_, Some(left), Some(right)) => {
            if (left.is_alphanumeric() && matches!(right, '&' | '\'' | '’'))
                || (left == '&' && right.is_alphanumeric())
            {
                false
            } else {
                !left.is_whitespace()
                    && !matches!(right, ',' | '.' | '!' | '?' | ':' | ';' | ')' | ']' | '}' | '"' | '”' | '’')
                    && !matches!(left, '(' | '[' | '{' | '/' | '\n')
            }
        }
        _ => false,
    }
}

fn simplify_inline_code(code: &str) -> String {
    let mut simplified = String::new();
    let chars: Vec<char> = code.chars().collect();

    for (index, ch) in chars.iter().enumerate() {
        let prev = index.checked_sub(1).and_then(|idx| chars.get(idx)).copied();
        let next = chars.get(index + 1).copied();
        let adjacent_word_chars = prev.map(is_code_word_char).unwrap_or(false)
            && next.map(is_code_word_char).unwrap_or(false);

        if adjacent_word_chars && matches!(ch, '_' | '-' | '/' | '\\' | '.' | ':') {
            if !simplified.ends_with(' ') {
                simplified.push(' ');
            }
            continue;
        }

        simplified.push(*ch);
    }

    normalize_inline_whitespace(&simplified)
}

fn is_code_word_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
}

fn simplify_url_for_speech(url: &str) -> String {
    let trimmed = url
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("mailto:");

    let mut spoken = String::new();
    let mut previous_was_space = false;

    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() {
            spoken.push(ch);
            previous_was_space = false;
            continue;
        }

        if matches!(
            ch,
            '.' | '/' | '_' | '-' | '?' | '&' | '=' | '#' | ':' | '%'
        ) && !previous_was_space
        {
            spoken.push(' ');
            previous_was_space = true;
        }
    }

    normalize_inline_whitespace(&spoken)
}

fn strip_html_tags(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    let mut entity = String::new();
    let mut in_entity = false;

    for ch in html.chars() {
        if in_entity {
            if ch == ';' {
                if let Some(decoded) = decode_html_entity(&entity) {
                    out.push_str(&decoded);
                } else {
                    out.push('&');
                    out.push_str(&entity);
                    out.push(';');
                }
                entity.clear();
                in_entity = false;
                continue;
            }

            if entity.len() < 16 {
                entity.push(ch);
            }
            continue;
        }

        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            '&' if !in_tag => {
                in_entity = true;
                entity.clear();
            }
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }

    if in_entity {
        out.push('&');
        out.push_str(&entity);
    }

    normalize_inline_whitespace(&out)
}

fn decode_html_entity(entity: &str) -> Option<String> {
    match entity {
        "amp" => Some("&".to_string()),
        "lt" => Some("<".to_string()),
        "gt" => Some(">".to_string()),
        "quot" => Some("\"".to_string()),
        "apos" => Some("'".to_string()),
        "nbsp" => Some(" ".to_string()),
        _ => decode_numeric_html_entity(entity),
    }
}

fn decode_numeric_html_entity(entity: &str) -> Option<String> {
    let code = entity.strip_prefix('#')?;

    let (radix, digits) = match code.as_bytes() {
        [b'x', rest @ ..] | [b'X', rest @ ..] => (16, std::str::from_utf8(rest).ok()?),
        _ => (10, code),
    };

    if digits.is_empty() {
        return None;
    }

    let value = u32::from_str_radix(digits, radix).ok()?;
    char::from_u32(value).map(|decoded| decoded.to_string())
}

#[cfg(test)]
mod tests {
    use super::normalize_text_for_tts;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn strips_common_markdown_formatting_without_losing_flow() {
        let markdown = r#"
## Heading

This is **important** and includes a [helpful link](https://example.com/docs/getting-started).

- First bullet
- Second bullet with `snake_case` inline code

> Quoted thought
"#;

        let spoken = normalize_text_for_tts(markdown);
        assert!(spoken.contains("Heading."));
        assert!(spoken.contains("This is important and includes a helpful link."));
        assert!(spoken.contains("First bullet."));
        assert!(spoken.contains("Second bullet with snake case inline code."));
        assert!(spoken.contains("Quote: Quoted thought."));
        assert!(!spoken.contains("##"));
        assert!(!spoken.contains("**"));
        assert!(!spoken.contains('`'));
    }

    #[test]
    fn handles_code_fences_and_fallback_links_intentionally() {
        let markdown = r#"
See <https://example.com/docs/api-reference>.

```rust
fn main() {
    println!("hello");
}
```
"#;

        let spoken = normalize_text_for_tts(markdown);
        assert!(spoken.contains("example com docs api reference"));
        assert!(spoken.contains("Code example omitted."));
        assert!(!spoken.contains("```"));
        assert!(!spoken.contains("println!"));
    }

    #[test]
    fn preserves_ordered_list_and_task_list_structure() {
        let markdown = r#"
1. Install dependencies
2. Run tests

- [x] Ship fix
- [ ] Write follow-up
"#;

        let spoken = normalize_text_for_tts(markdown);
        assert!(spoken.contains("1. Install dependencies."));
        assert!(spoken.contains("2. Run tests."));
        assert!(spoken.contains("Completed. Ship fix."));
        assert!(spoken.contains("Not completed. Write follow-up."));
    }

    #[test]
    fn normalizes_real_repository_markdown_without_raw_markup_artifacts() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let readme = fs::read_to_string(manifest_dir.join("../README.md")).expect("read README");
        let spoken = normalize_text_for_tts(&readme);

        assert!(spoken.contains("Parrot"));
        assert!(!spoken.contains("```"));
        assert!(!spoken.contains("## "));
        assert!(!spoken.contains("**"));
        assert!(spoken.len() > 1000);
    }

    #[test]
    fn decodes_named_and_numeric_html_entities_in_html_blocks() {
        let markdown = r#"
<span>AT&amp;T uses smart quotes: &#8217;hello&#8217; and &#x2014; dash.</span>
"#;

        let spoken = normalize_text_for_tts(markdown);
        assert!(spoken.contains("AT&T uses smart quotes: ’hello’ and — dash."));
    }

    #[test]
    fn preserves_unknown_or_incomplete_entities_as_literal_text() {
        let markdown = r#"
Keep &custom; visible and preserve dangling &entity text.
"#;

        let spoken = normalize_text_for_tts(markdown);
        assert!(spoken.contains("Keep &custom; visible and preserve dangling &entity text."));
    }

    #[test]
    fn keeps_apostrophes_inside_words_without_inserting_spaces() {
        let markdown = "Feedback doesn't live in one place. Feedback doesn’t live in one place.";

        let spoken = normalize_text_for_tts(markdown);
        assert!(spoken.contains("Feedback doesn’t live in one place."));
        assert!(!spoken.contains("doesn 't"));
        assert!(!spoken.contains("doesn ’t"));
    }

    #[test]
    fn keeps_quoted_phrases_tight_without_inserting_inner_quote_spaces() {
        let markdown = r#""This isn't a "nice to have""#;

        let spoken = normalize_text_for_tts(markdown);
        assert!(spoken.contains(r#"“This isn’t a “nice to have”."#));
        assert!(!spoken.contains("“ This"));
        assert!(!spoken.contains("“ nice"));
        assert!(!spoken.contains("have ”"));
    }

    #[test]
    fn preserves_spaces_around_adjacent_quoted_terms() {
        let markdown =
            r#"**'Navigate to Settings/Integrations:** Look for "CSV" or "NPS" settings."#;

        let spoken = normalize_text_for_tts(markdown);
        assert!(spoken.contains(r#"Navigate to Settings/Integrations: Look for “CSV” or “NPS” settings."#));
        assert!(!spoken.contains("”or“"));
        assert!(!spoken.contains("”settings"));
    }
}
