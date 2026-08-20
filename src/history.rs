//! Query-history cursor math for shift-up / shift-down navigation.
//!
//! `fff_search::query_tracker` hands entries back by offset (0 = most recent)
//! but never dedups on write, so opening three files under the same query
//! stores that query three times. Walking has to skip those repeats itself.
//! Kept gpui-free so the stepping rules are unit-tested without a render
//! context.

// Which way a keypress walks the history stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    // shift-up: toward older entries.
    Older,
    // shift-down: back toward the newest entry, then the draft.
    Newer,
}

// What the caller should do with the query field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    // Park the cursor at `offset` and show `query`.
    Move { offset: usize, query: String },
    // Stepped past the newest entry — leave history, restore the draft.
    Draft,
    // Already at the end of the stack in that direction; nothing to show.
    Edge,
}

// Walk one entry from `current` (None = not in history, editing the draft).
//
// `fetch` reads the entry at an offset, returning None past the end of the
// stack. `shown` is the text currently in the field; entries equal to it are
// skipped so a run of identical queries advances by one keypress, not three.
pub fn step(
    mut fetch: impl FnMut(usize) -> Option<String>,
    current: Option<usize>,
    direction: Direction,
    shown: &str,
) -> Step {
    match direction {
        Direction::Older => {
            let mut probe = match current {
                None => 0,
                Some(offset) => offset + 1,
            };
            loop {
                match fetch(probe) {
                    None => return Step::Edge,
                    Some(query) if query == shown => probe += 1,
                    Some(query) => {
                        return Step::Move {
                            offset: probe,
                            query,
                        };
                    }
                }
            }
        }
        Direction::Newer => {
            // Not in history: shift-down is a no-op, the draft is already up.
            let Some(mut probe) = current else {
                return Step::Edge;
            };
            while probe > 0 {
                probe -= 1;
                match fetch(probe) {
                    Some(query) if query == shown => continue,
                    Some(query) => {
                        return Step::Move {
                            offset: probe,
                            query,
                        };
                    }
                    // A hole mid-stack shouldn't happen, but keep walking
                    // toward the newest entry rather than stalling the cursor.
                    None => continue,
                }
            }
            Step::Draft
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Newest-first, matching the offset order `fetch` is queried in.
    fn stack(entries: &[&str]) -> impl FnMut(usize) -> Option<String> + use<> {
        let entries: Vec<String> = entries.iter().map(|s| s.to_string()).collect();
        move |offset| entries.get(offset).cloned()
    }

    #[test]
    fn older_from_draft_lands_on_most_recent() {
        let got = step(stack(&["b", "a"]), None, Direction::Older, "draft");
        assert_eq!(
            got,
            Step::Move {
                offset: 0,
                query: "b".into()
            }
        );
    }

    #[test]
    fn older_advances_one_entry_per_press() {
        let got = step(stack(&["b", "a"]), Some(0), Direction::Older, "b");
        assert_eq!(
            got,
            Step::Move {
                offset: 1,
                query: "a".into()
            }
        );
    }

    #[test]
    fn older_past_oldest_reports_edge() {
        assert_eq!(
            step(stack(&["b", "a"]), Some(1), Direction::Older, "a"),
            Step::Edge
        );
    }

    #[test]
    fn older_on_empty_history_reports_edge() {
        assert_eq!(step(stack(&[]), None, Direction::Older, ""), Step::Edge);
    }

    #[test]
    fn older_skips_consecutive_repeats_of_the_shown_query() {
        // Three opens under "b" collapse into a single step to "a".
        let got = step(stack(&["b", "b", "b", "a"]), Some(0), Direction::Older, "b");
        assert_eq!(
            got,
            Step::Move {
                offset: 3,
                query: "a".into()
            }
        );
    }

    #[test]
    fn older_skips_a_leading_entry_matching_the_draft() {
        // Retyping the last query by hand shouldn't cost a wasted keypress.
        let got = step(stack(&["b", "a"]), None, Direction::Older, "b");
        assert_eq!(
            got,
            Step::Move {
                offset: 1,
                query: "a".into()
            }
        );
    }

    #[test]
    fn newer_walks_back_toward_the_newest_entry() {
        let got = step(stack(&["b", "a"]), Some(1), Direction::Newer, "a");
        assert_eq!(
            got,
            Step::Move {
                offset: 0,
                query: "b".into()
            }
        );
    }

    #[test]
    fn newer_past_newest_restores_the_draft() {
        assert_eq!(
            step(stack(&["b", "a"]), Some(0), Direction::Newer, "b"),
            Step::Draft
        );
    }

    #[test]
    fn newer_outside_history_reports_edge() {
        assert_eq!(step(stack(&["b"]), None, Direction::Newer, "x"), Step::Edge);
    }

    #[test]
    fn newer_skips_repeats_and_falls_through_to_draft() {
        // Everything newer than the cursor repeats what's shown, so there is
        // no distinct entry left between here and the draft.
        assert_eq!(
            step(stack(&["b", "b", "a"]), Some(2), Direction::Newer, "a"),
            Step::Move {
                offset: 1,
                query: "b".into()
            }
        );
        assert_eq!(
            step(stack(&["b", "b", "a"]), Some(1), Direction::Newer, "b"),
            Step::Draft
        );
    }

    #[test]
    fn round_trip_returns_to_the_starting_entry() {
        let older = step(stack(&["c", "b", "a"]), Some(0), Direction::Older, "c");
        let Step::Move { offset, query } = older else {
            panic!("expected a move, got {older:?}");
        };
        assert_eq!(
            step(
                stack(&["c", "b", "a"]),
                Some(offset),
                Direction::Newer,
                &query
            ),
            Step::Move {
                offset: 0,
                query: "c".into()
            }
        );
    }
}
