use nutype::nutype;
use pulldown_cmark::Event::{Code, End, HardBreak, InlineHtml, Rule, SoftBreak, Start, Text};
use pulldown_cmark::{Options, Parser, Tag, TagEnd};
use std::borrow::Cow;
use std::collections::HashMap;
use std::num::NonZeroU32;

const SHA1_LEN: usize = 40;
const SHORT_SHA1_LEN: usize = 7;

#[nutype(
    sanitize(with = |input| truncate_with_lines::<10000>(&input).into_owned(), trim),
    validate(len_char_min = 1, len_char_max = 10000),
    default = "Release Notes",
    derive(Clone, Default, FromStr, Display, Deserialize, Serialize, PartialEq, Eq, Debug)
)]
pub struct ReleaseNotes(String);

impl ReleaseNotes {
    pub fn format(body: &str, owner: &str, repo: &str) -> Option<Self> {
        let parser = Parser::new_ext(body, Options::ENABLE_STRIKETHROUGH | Options::ENABLE_GFM);
        let mut buffer = String::new();

        let mut ordered_list_map = HashMap::new();
        let mut list_item_level = 0;
        for event in parser {
            match event {
                Start(tag) => match tag {
                    Tag::Heading { .. }
                    | Tag::BlockQuote(_)
                    | Tag::CodeBlock(_)
                    | Tag::Table(_)
                    | Tag::TableHead
                    | Tag::TableRow => {
                        if !buffer.ends_with('\n') {
                            buffer.push('\n');
                        }
                    }
                    Tag::Link {
                        link_type: _,
                        dest_url: _,
                        title,
                        id: _,
                    }
                    | Tag::Image {
                        link_type: _,
                        dest_url: _,
                        title,
                        id: _,
                    } => {
                        buffer.push_str(&title);
                    }
                    Tag::List(first_index) => {
                        if let Some(index) = first_index {
                            ordered_list_map.insert(list_item_level, index);
                        }
                        if !buffer.ends_with('\n') {
                            buffer.push('\n');
                        }
                    }
                    Tag::Item => {
                        for _ in 0..list_item_level {
                            buffer.push_str("    ");
                        }
                        if let Some(index) = ordered_list_map.get_mut(&list_item_level) {
                            buffer.push_str(itoa::Buffer::new().format(*index));
                            buffer.push_str(". ");
                            *index += 1;
                        } else {
                            buffer.push_str("- ");
                        }
                        list_item_level += 1;
                    }
                    _ => (),
                },
                End(tag) => match tag {
                    TagEnd::Heading(_)
                    | TagEnd::BlockQuote(_)
                    | TagEnd::CodeBlock
                    | TagEnd::Table
                    | TagEnd::TableHead
                    | TagEnd::TableRow => {
                        if !buffer.ends_with('\n') {
                            buffer.push('\n');
                        }
                    }
                    TagEnd::List(_) => {
                        ordered_list_map.remove(&list_item_level);
                        if list_item_level >= 1 && buffer.ends_with('\n') {
                            buffer.pop();
                        }
                    }
                    TagEnd::Item => {
                        let second_last_char_pos = buffer
                            .char_indices()
                            .nth_back(1)
                            .map_or(buffer.len(), |(pos, _)| pos);
                        if &buffer[second_last_char_pos..] == "- " {
                            buffer.drain(second_last_char_pos..);
                        } else {
                            buffer.push('\n');
                        }
                        list_item_level -= 1;
                    }
                    _ => (),
                },
                Text(text) => {
                    let mut result = String::new();
                    let mut rest = &*text;
                    let prefix = "https://github.com/";

                    while let Some(start) = rest.find(prefix) {
                        result.push_str(&rest[..start]);
                        rest = &rest[start..];

                        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                        let url = &rest[..end];
                        let mut parts = url.trim_start_matches(prefix).split('/');

                        if let (Some(repo_owner), Some(repo_name), Some(r#type), Some(resource)) =
                            (parts.next(), parts.next(), parts.next(), parts.next())
                        {
                            if r#type == "pull" || r#type == "issues" {
                                let (issue_number, comment) = resource
                                    .split_once("#issuecomment")
                                    .unwrap_or((resource, ""));
                                if issue_number.parse::<NonZeroU32>().is_ok() {
                                    if repo_owner != owner || repo_name != repo {
                                        result.push_str(repo_owner);
                                        result.push('/');
                                        result.push_str(repo_name);
                                    }
                                    result.push('#');
                                    result.push_str(issue_number);
                                    if !comment.is_empty() {
                                        result.push_str(" (comment)");
                                    }
                                }
                            } else if r#type == "compare" || r#type == "releases" {
                                result.push_str(url);
                            } else if r#type == "commit"
                                && resource.len() == SHA1_LEN
                                && resource.bytes().all(|byte| byte.is_ascii_hexdigit())
                            {
                                if let Some(short_sha) = resource.get(..SHORT_SHA1_LEN) {
                                    result.push_str(short_sha);
                                }
                            }
                        }

                        rest = &rest[end..];
                    }
                    result.push_str(rest);
                    buffer.push_str(&remove_sha1(&result));
                }
                Code(code) => buffer.push_str(&code.replace('\t', " ")),
                InlineHtml(html) => buffer.push_str(&html),
                SoftBreak | HardBreak | Rule => buffer.push('\n'),
                _ => (),
            }
        }
        Self::try_new(buffer).ok()
    }
}

fn remove_sha1(input: &str) -> String {
    let mut result = String::new();
    let mut buffer = heapless::String::<SHA1_LEN>::new();

    for character in input.chars() {
        if character.is_ascii_hexdigit() && buffer.len() < SHA1_LEN {
            buffer.push(character).unwrap();
        } else if !character.is_ascii_hexdigit() && buffer.len() == SHA1_LEN {
            buffer.clear();
        } else {
            result.push_str(&buffer);
            buffer.clear();
            result.push(character);
        }
    }

    if buffer.len() != SHA1_LEN {
        result.push_str(&buffer);
    }

    result
}

fn truncate_with_lines<const N: usize>(input: &str) -> Cow<str> {
    if input.chars().count() <= N {
        return Cow::Borrowed(input);
    }

    let mut result = String::with_capacity(N);
    let mut current_size = 0;

    for (iter_count, line) in input.lines().enumerate() {
        let prospective_size = current_size + line.chars().count() + "\n".len();
        if prospective_size > N {
            break;
        }
        if iter_count != 0 {
            result.push('\n');
        }
        result.push_str(line);
        current_size = prospective_size;
    }

    Cow::Owned(result)
}

#[cfg(test)]
mod tests {
    use crate::types::release_notes::{truncate_with_lines, ReleaseNotes, SHORT_SHA1_LEN};
    use indoc::indoc;
    use rand::random;
    use sha1::{Digest, Sha1};

    #[test]
    fn issue() {
        let value = "- Issue https://github.com/owner/repo/issues/123";
        assert_eq!(
            ReleaseNotes::format(value, "owner", "repo"),
            ReleaseNotes::try_new("- Issue #123").ok()
        )
    }

    #[test]
    fn issue_comment() {
        let value = "- Issue https://github.com/owner/repo/issues/123#issuecomment-1234567890";
        assert_eq!(
            ReleaseNotes::format(value, "owner", "repo"),
            ReleaseNotes::try_new("- Issue #123 (comment)").ok()
        )
    }

    #[test]
    fn pull_request() {
        let value = "- Pull request https://github.com/owner/repo/pull/123";
        assert_eq!(
            ReleaseNotes::format(value, "owner", "repo"),
            ReleaseNotes::try_new("- Pull request #123").ok()
        )
    }

    #[test]
    fn pull_request_comment() {
        let value = "- Pull request https://github.com/owner/repo/pull/123#issuecomment-1234567890";
        assert_eq!(
            ReleaseNotes::format(value, "owner", "repo"),
            ReleaseNotes::try_new("- Pull request #123 (comment)").ok()
        )
    }

    #[test]
    fn different_repo_issue() {
        let value = "- Issue https://github.com/different/repo/issues/123";
        assert_eq!(
            ReleaseNotes::format(value, "owner", "repo"),
            ReleaseNotes::try_new("- Issue different/repo#123").ok()
        )
    }

    #[test]
    fn different_repo_issue_comment() {
        let value = "- Issue https://github.com/different/repo/issues/123#issuecomment-1234567890";
        assert_eq!(
            ReleaseNotes::format(value, "owner", "repo"),
            ReleaseNotes::try_new("- Issue different/repo#123 (comment)").ok()
        )
    }

    #[test]
    fn multiple_issues() {
        let value = "- Issue https://github.com/owner/repo/issues/123 and https://github.com/owner/repo/issues/321";
        assert_eq!(
            ReleaseNotes::format(value, "owner", "repo"),
            ReleaseNotes::try_new("- Issue #123 and #321").ok()
        )
    }

    #[test]
    fn no_urls() {
        let value = "- No issue link";
        assert_eq!(
            ReleaseNotes::format(value, "owner", "repo"),
            ReleaseNotes::try_new(value).ok()
        )
    }

    #[test]
    fn full_changelog_url() {
        let value = "Full Changelog: https://github.com/owner/repo/compare/v1.0.0...v1.1.0";
        assert_eq!(
            ReleaseNotes::format(value, "owner", "repo"),
            ReleaseNotes::try_new(value).ok()
        )
    }

    #[test]
    fn release_url() {
        let value = "Previous release: https://github.com/owner/repo/releases/tag/1.2.3";
        assert_eq!(
            ReleaseNotes::format(value, "owner", "repo"),
            ReleaseNotes::try_new(value).ok()
        )
    }

    #[test]
    fn commit_url() {
        let mut random_hash =
            base16ct::lower::encode_string(&Sha1::digest(random::<[u8; 1 << 4]>()));
        let commit_url = format!("https://github.com/owner/repo/commit/{random_hash}");
        random_hash.truncate(SHORT_SHA1_LEN);
        assert_eq!(
            ReleaseNotes::format(&commit_url, "owner", "repo"),
            ReleaseNotes::try_new(random_hash).ok()
        )
    }

    #[test]
    fn header_syntax_removed() {
        let value = indoc! {"
        # Header 1
        ## Header 2
        ### Header 3
        "};
        let expected = indoc! {"
        Header 1
        Header 2
        Header 3
        "};
        assert_eq!(
            ReleaseNotes::format(value, "owner", "repo"),
            ReleaseNotes::try_new(expected).ok()
        )
    }

    #[test]
    fn strikethrough_removed() {
        assert_eq!(
            ReleaseNotes::format("~~Strikethrough text~~", "owner", "repo"),
            ReleaseNotes::try_new("Strikethrough text").ok()
        )
    }

    #[test]
    fn bold_removed() {
        assert_eq!(
            ReleaseNotes::format("**Bold text**", "owner", "repo"),
            ReleaseNotes::try_new("Bold text").ok()
        )
    }

    #[test]
    fn inline_html() {
        let value = indoc! {"
            ```
            <html>
            ```
        "};
        assert_eq!(
            ReleaseNotes::format(value, "owner", "repo"),
            ReleaseNotes::try_new("<html>").ok()
        )
    }

    #[test]
    fn test_asterisk_bullet_points() {
        let value = indoc! {"
        * Bullet point 1
        * Bullet point 2
        "};
        let expected = indoc! {"
        - Bullet point 1
        - Bullet point 2
        "};
        assert_eq!(
            ReleaseNotes::format(value, "owner", "repo"),
            ReleaseNotes::try_new(expected).ok()
        )
    }

    #[test]
    fn test_ordered_list() {
        let value = indoc! {"
        1. Item number 1
            1. Item number 1.1
            2. Item number 1.2
                1. Item number 1.2.1
                2. Item number 1.2.2
                3. Item number 1.2.3
        2. Item number 2
            1. Item number 2.1
            2. Item number 2.2
        "};
        assert_eq!(
            ReleaseNotes::format(value, "owner", "repo"),
            ReleaseNotes::try_new(value).ok()
        )
    }

    #[test]
    fn test_nested_list_items() {
        let value = indoc! {"
        - Bullet point 1
            - 2nd level nested bullet point 1
            - 2nd level nested bullet point 2
                - 3rd level nested bullet point 1
                - 3rd level nested bullet point 2
                    - 4th level nested bullet point 1
                    - 4th level nested bullet point 2
        - Bullet point 2
        "};
        assert_eq!(
            ReleaseNotes::format(value, "owner", "repo"),
            ReleaseNotes::try_new(value).ok()
        )
    }

    #[test]
    fn test_sha1_removed() {
        use rand::random;
        use sha1::{Digest, Sha1};

        let random_hash = base16ct::lower::encode_string(&Sha1::digest(random::<[u8; 1 << 4]>()));
        let value = format!("- {random_hash} Bullet point 1 {random_hash}");
        assert_eq!(
            ReleaseNotes::format(&value, "owner", "repo"),
            ReleaseNotes::try_new("- Bullet point 1").ok()
        )
    }

    #[test]
    fn test_truncate_to_lines() {
        use std::fmt::Write;

        const CHAR_LIMIT: usize = 100;

        let mut buffer = String::new();
        let mut line_count = 0;
        while buffer.chars().count() <= CHAR_LIMIT {
            line_count += 1;
            writeln!(buffer, "Line {line_count}").unwrap();
        }
        let formatted = truncate_with_lines::<CHAR_LIMIT>(&buffer);
        let formatted_char_count = formatted.chars().count();
        assert!(formatted_char_count < buffer.chars().count());
        assert_eq!(formatted.trim().chars().count(), formatted_char_count);
    }
}
