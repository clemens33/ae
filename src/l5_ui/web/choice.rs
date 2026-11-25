//! The choices a DECIDE text offers, in the form the orchestrator writes:
//! `Question? 1) a 2) b (recommend 2) #slug`. Pure. A text that does not read
//! as this form offers no choices, and the page then takes a free-text answer.

use crate::topic;

/// The choices of one DECIDE text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choices {
    /// The text before the first option.
    pub question: String,
    /// The labels of options 1 to `options.len()`, 2 to 9 of them.
    pub options: Vec<String>,
    /// The number of the recommended option, when it names one of them.
    pub recommend: Option<usize>,
}

/// The choices `text` offers: a question, then the options `1)`, `2)` and so
/// on to at most `9)`, each at the start of a word, numbered in order and each
/// with a label. `None` for any other text. The trailing `#slug` words and one
/// trailing recommendation belong to no label.
#[must_use]
pub fn parse(text: &str) -> Option<Choices> {
    let untagged = without_tags(text);
    let (body, recommend) = recommendation(untagged).unwrap_or((untagged, None));
    let mut starts: Vec<usize> = Vec::new();
    for (at, number) in markers(body) {
        if number != starts.len() + 1 {
            return None;
        }
        starts.push(at);
    }
    if starts.len() < 2 {
        return None;
    }
    let question = body.get(..*starts.first()?)?.trim();
    if question.is_empty() {
        return None;
    }
    let ends = starts.iter().skip(1).copied().chain([body.len()]);
    let options = starts
        .iter()
        .zip(ends)
        .map(|(start, end)| {
            body.get(start + 2..end)
                .map(str::trim)
                .filter(|label| !label.is_empty())
                .map(str::to_owned)
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Choices {
        question: question.to_owned(),
        recommend: recommend.filter(|number| (1..=options.len()).contains(number)),
        options,
    })
}

/// `text` without its trailing words that are `#` and a whole slug.
fn without_tags(text: &str) -> &str {
    let mut rest = text.trim_end();
    loop {
        let word = rest.rsplit(char::is_whitespace).next().unwrap_or_default();
        if !word.strip_prefix('#').is_some_and(topic::is_slug) {
            return rest;
        }
        rest = rest
            .get(..rest.len() - word.len())
            .unwrap_or_default()
            .trim_end();
    }
}

/// `text` without a trailing `(recommend N)` or `(Empfehlung N)`, ASCII case
/// ignored, that starts a word, and the number it names; `None` when `text`
/// does not end with one.
fn recommendation(text: &str) -> Option<(&str, Option<usize>)> {
    let (head, inner) = text.strip_suffix(')')?.rsplit_once('(')?;
    if !head.is_empty() && !head.ends_with(char::is_whitespace) {
        return None;
    }
    let mut words = inner.split_whitespace();
    let (word, number) = (words.next()?, words.next()?);
    let named = word.eq_ignore_ascii_case("recommend") || word.eq_ignore_ascii_case("empfehlung");
    (named && words.next().is_none() && number.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| (head.trim_end(), number.parse().ok()))
}

/// Each `N)` at the start of a word, `N` from 1 to 9: where it starts and `N`.
fn markers(text: &str) -> impl Iterator<Item = (usize, usize)> + '_ {
    text.char_indices().filter_map(|(at, ch)| {
        let number = ch.to_digit(10).filter(|number| *number > 0)?;
        let opens = text
            .get(..at)?
            .chars()
            .next_back()
            .is_none_or(char::is_whitespace);
        if !opens || text.as_bytes().get(at + 1) != Some(&b')') {
            return None;
        }
        Some((at, usize::try_from(number).ok()?))
    })
}

#[cfg(test)]
mod tests {
    use super::{Choices, parse};

    fn choices(question: &str, options: &[&str], recommend: Option<usize>) -> Choices {
        Choices {
            question: question.to_owned(),
            options: options.iter().map(|label| (*label).to_owned()).collect(),
            recommend,
        }
    }

    /// The ruled form: numbered options, a recommendation in either word and
    /// the tags after it.
    #[test]
    fn the_ruled_form_reads_with_either_recommend_word() {
        assert_eq!(
            parse("Ship it? 1) Yes 2) No 3) Later (recommend 3) #ship #reserved"),
            Some(choices("Ship it?", &["Yes", "No", "Later"], Some(3))),
        );
        assert_eq!(
            parse("Wohin? 1) Hier 2) Dort (EMPFEHLUNG 1) #calm"),
            Some(choices("Wohin?", &["Hier", "Dort"], Some(1))),
        );
        assert_eq!(parse("Q? 1)a 2)b"), Some(choices("Q?", &["a", "b"], None)));
    }

    /// Only a whole trailing `#slug` word is a tag; every other `#` word and
    /// every `N)` inside a word stays in its label.
    #[test]
    fn only_whole_trailing_tags_leave_the_last_label() {
        assert_eq!(
            parse("Agree? 1) part2) #C 2) Ordinary #reserved_extra #calm"),
            Some(choices(
                "Agree?",
                &["part2) #C", "Ordinary #reserved_extra"],
                None
            )),
        );
        for kept in ["#smoke.", "#Smoke", "#", "a#smoke"] {
            assert_eq!(
                parse(&format!("Q? 1) a 2) b {kept}")),
                Some(choices("Q?", &["a", &format!("b {kept}")], None)),
                "{kept}"
            );
        }
    }

    /// A recommendation outside the options names none, and other words in
    /// the brackets are part of the last label.
    #[test]
    fn a_recommendation_counts_only_in_its_form_and_in_range() {
        for out in [
            "(recommend 0)",
            "(recommend 3)",
            "(recommend 99999999999999999999999)",
        ] {
            assert_eq!(
                parse(&format!("Q? 1) a 2) b {out}")),
                Some(choices("Q?", &["a", "b"], None)),
                "{out}"
            );
        }
        for kept in [
            "(recommend two)",
            "(recommend 2 now)",
            "(recommend)",
            "(see b)",
        ] {
            assert_eq!(
                parse(&format!("Q? 1) a 2) b {kept}")),
                Some(choices("Q?", &["a", &format!("b {kept}")], None)),
                "{kept}"
            );
        }
    }

    /// Anything else is free text: one option, a gap or a repeat in the
    /// numbers, no question or an empty label. A bracketed `N)` that is not
    /// the one trailing recommendation is a number out of order.
    #[test]
    fn any_other_text_offers_no_choices() {
        for text in [
            "Q? 1) a 2) b (recommend 1) (recommend 2)",
            "Q? 1) a 2) b(recommend 2)",
            "Q? 1) a 2) b (prefer 2)",
            "Legacy prose #smoke",
            "Only? 1) one",
            "Gap? 1) a 3) b",
            "Late start? 2) a 3) b",
            "Repeat? 1) a 2) b 2) c",
            "1) a 2) b",
            "Q? 1) 2) b",
            "Q? 1) a 2) #smoke",
            "Q? 1) a 2) (recommend 1)",
            "Zero? 0) a 1) b",
            "",
        ] {
            assert_eq!(parse(text), None, "{text:?}");
        }
    }
}
