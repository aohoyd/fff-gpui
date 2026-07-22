use std::borrow::Cow;
use std::path::{Component, Path};

#[derive(Clone, Copy)]
pub enum PathShortenStrategy {
    MiddleNumber,
}

impl PathShortenStrategy {
    // Shorten a path to fit within the requested character budget.
    pub fn shorten_path(&self, path: &Path, max_size: usize) -> String {
        const MIN_SMART_SHORTEN_SIZE: usize = 8;

        let sep = std::path::MAIN_SEPARATOR;
        let path_str = path.to_string_lossy();
        if path_str.len() <= max_size {
            return path_str.to_string();
        }

        if max_size < MIN_SMART_SHORTEN_SIZE {
            return Self::truncate_str(&path_str, max_size);
        }

        let components: Vec<&str> = path
            .components()
            .filter_map(|component| match component {
                Component::Normal(segment) => segment.to_str(),
                _ => None,
            })
            .collect();

        match components.len() {
            0 => path_str.to_string(),
            1 => Self::truncate_str(components[0], max_size),
            2 => Self::shorten_pair(&components, max_size, sep),
            _ => self.shorten_middle(&components, max_size, sep),
        }
    }

    // Shorten a two-component path while preserving the filename when possible.
    fn shorten_pair(components: &[&str], max_size: usize, sep: char) -> String {
        let joined = components.join(&sep.to_string());
        if joined.len() <= max_size {
            return joined;
        }

        let last = components[1];
        let available_for_first = max_size.saturating_sub(1 + last.len());
        if available_for_first > 0 && last.len() < max_size {
            return format!(
                "{}{}{}",
                Self::truncate_str(components[0], available_for_first),
                sep,
                last
            );
        }

        Self::truncate_str(last, max_size)
    }

    // Shorten a multi-component path by replacing middle components with a marker.
    fn shorten_middle(&self, components: &[&str], max_size: usize, sep: char) -> String {
        let total = components.len();
        let first = components[0];
        let last = components[total - 1];
        let hidden = total - 2;
        let ellipsis = Self::make_ellipsis(hidden);
        let min_overhead = 2 + ellipsis.len();

        if first.len() + last.len() + min_overhead <= max_size {
            return self.expand_middle(components, max_size, sep);
        }

        let needed_for_last = last.len() + 1 + ellipsis.len() + 1;
        if needed_for_last <= max_size {
            let available_for_first = max_size - needed_for_last;
            return format!(
                "{}{}{}{}{}",
                Self::truncate_str(first, available_for_first),
                sep,
                ellipsis,
                sep,
                last
            );
        }

        let needed_for_ellipsis_last = ellipsis.len() + 1 + last.len();
        if needed_for_ellipsis_last <= max_size {
            return format!("{}{}{}", ellipsis, sep, last);
        }

        Self::truncate_str(last, max_size)
    }

    // Expand the visible prefix and suffix while the path still fits.
    fn expand_middle(&self, components: &[&str], max_size: usize, sep: char) -> String {
        let total = components.len();
        let mut left_end = 1;
        let mut right_start = total - 1;

        loop {
            if right_start <= left_end {
                break;
            }

            let mut added = false;
            if right_start > left_end + 1 {
                let hidden = right_start - 1 - left_end;
                let candidate =
                    Self::build_middle_result(components, left_end, right_start - 1, hidden, sep);
                if candidate.len() <= max_size {
                    right_start -= 1;
                    added = true;
                }
            }

            if left_end < right_start - 1 {
                let hidden = right_start - (left_end + 1);
                let candidate =
                    Self::build_middle_result(components, left_end + 1, right_start, hidden, sep);
                if candidate.len() <= max_size {
                    left_end += 1;
                    added = true;
                }
            }

            if !added {
                break;
            }
        }

        Self::build_middle_result(
            components,
            left_end,
            right_start,
            right_start - left_end,
            sep,
        )
    }

    // Build a shortened path from visible prefix, marker, and suffix components.
    fn build_middle_result(
        components: &[&str],
        left_end: usize,
        right_start: usize,
        hidden_count: usize,
        sep: char,
    ) -> String {
        let ellipsis = Self::make_ellipsis(hidden_count);
        let mut result = String::new();

        for (idx, part) in components[..left_end].iter().enumerate() {
            if idx > 0 {
                result.push(sep);
            }
            result.push_str(part);
        }

        if left_end > 0 {
            result.push(sep);
        }
        result.push_str(&ellipsis);
        result.push(sep);

        for (idx, part) in components[right_start..].iter().enumerate() {
            if idx > 0 {
                result.push(sep);
            }
            result.push_str(part);
        }

        result
    }

    // Format the marker that represents hidden path components.
    fn make_ellipsis(hidden_count: usize) -> Cow<'static, str> {
        match hidden_count {
            1 => ".".into(),
            2 => "..".into(),
            3 => "...".into(),
            n => format!(".{}.", n).into(),
        }
    }

    // Truncate a string to a maximum number of characters.
    fn truncate_str(s: &str, max_len: usize) -> String {
        if max_len == 0 {
            return String::new();
        }

        s.chars().take(max_len).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::MAIN_SEPARATOR;

    // Join components with the platform separator so expectations match the
    // algorithm's own `MAIN_SEPARATOR` usage on every target.
    fn joined(parts: &[&str]) -> String {
        parts.join(&MAIN_SEPARATOR.to_string())
    }

    fn shorten(path: &str, max: usize) -> String {
        // `path` is written with '/'; rebuild it with the platform separator so
        // the input parses into the same components everywhere.
        let native = joined(&path.split('/').collect::<Vec<_>>());
        PathShortenStrategy::MiddleNumber.shorten_path(Path::new(&native), max)
    }

    #[test]
    fn returns_path_unchanged_when_it_already_fits() {
        assert_eq!(shorten("src/main.rs", 20), joined(&["src", "main.rs"]));
    }

    #[test]
    fn exact_fit_is_returned_verbatim() {
        // "abc/def" is exactly 7 chars for a budget of 7.
        assert_eq!(shorten("abc/def", 7), joined(&["abc", "def"]));
    }

    #[test]
    fn budget_below_smart_threshold_raw_truncates_the_whole_string() {
        // Budget < MIN_SMART_SHORTEN_SIZE (8): raw first-`max` chars of the path.
        let native = joined(&["aaaa", "bbbb", "cccc"]);
        assert_eq!(
            shorten("aaaa/bbbb/cccc", 5),
            native.chars().take(5).collect::<String>()
        );
    }

    #[test]
    fn zero_budget_yields_empty_string() {
        assert_eq!(shorten("abc/def", 0), "");
    }

    #[test]
    fn single_component_truncates_to_budget() {
        assert_eq!(shorten("abcdefghijklmnop", 10), "abcdefghij");
    }

    #[test]
    fn two_component_preserves_filename_and_truncates_dir() {
        assert_eq!(
            shorten("longdirname/file.rs", 15),
            joined(&["longdir", "file.rs"])
        );
    }

    #[test]
    fn two_component_truncates_filename_when_it_cannot_fit() {
        assert_eq!(shorten("dir/verylongfilename.txt", 10), "verylongfi");
    }

    #[test]
    fn multi_component_elides_middle_with_marker() {
        // Six components, budget 9: prefix + numeric-free marker + suffix.
        let out = shorten("a/b/c/d/e/f", 9);
        assert_eq!(out, joined(&["a", "...", "e", "f"]));
        assert!(out.len() <= 9);
    }

    #[test]
    fn multi_component_keeps_last_when_prefix_wont_fit() {
        // first+last+overhead too large, but the last still fits after a
        // truncated first: "aa/../dddddd".
        let out = shorten("aaaaaa/bb/cc/dddddd", 12);
        assert_eq!(out, joined(&["aa", "..", "dddddd"]));
        assert!(out.len() <= 12);
    }

    #[test]
    fn multi_component_drops_prefix_when_only_marker_and_last_fit() {
        let out = shorten("aaaa/bb/cc/dddddddd", 11);
        assert_eq!(out, joined(&["..", "dddddddd"]));
        assert!(out.len() <= 11);
    }

    #[test]
    fn multi_component_truncates_last_when_it_alone_overflows() {
        assert_eq!(shorten("aa/bb/superlongfilename", 10), "superlongf");
    }

    #[test]
    fn multibyte_single_component_truncates_by_char_not_byte() {
        // Greek letters are two bytes each; truncation counts characters and
        // must never split a code point.
        let out = shorten("αβγδεζηθικλμν", 8);
        assert_eq!(out, "αβγδεζηθ");
        assert_eq!(out.chars().count(), 8);
    }
}
