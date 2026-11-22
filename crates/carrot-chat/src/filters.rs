//! Pure chat-filter functions: Brazilian Portuguese slang replacement,
//! message fixing, anti-caps and URL/content censorship.
//!
//! Behavior mirrors the reference server's `nChat`: replacements are
//! word-wise and case-insensitive, shouty messages are lowercased (with a
//! skip list), fixed messages start capitalized and end with a period, and
//! censored messages are silently swallowed.

use crate::config::FiltersConfig;

/// A parsed replacement rule (`vc,voce->você`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replacer {
    pub from: Vec<String>,
    pub to: String,
}

/// Parses the raw `from1,from2->to` rules.
#[must_use]
pub fn parse_replacers(raw: &[String]) -> Vec<Replacer> {
    raw.iter()
        .filter_map(|rule| {
            let (from, to) = rule.split_once("->")?;
            let from: Vec<String> = from
                .split(',')
                .map(str::trim)
                .filter(|term| !term.is_empty())
                .map(ToString::to_string)
                .collect();
            if from.is_empty() || to.trim().is_empty() {
                return None;
            }
            Some(Replacer {
                from,
                to: to.trim().to_string(),
            })
        })
        .collect()
}

/// Applies the replacers word-wise, keeping trailing punctuation.
#[must_use]
pub fn apply_replacers(message: &str, replacers: &[Replacer]) -> String {
    message
        .split_whitespace()
        .map(|word| {
            let (core, suffix) = split_trailing_punct(word);
            for replacer in replacers {
                if replacer
                    .from
                    .iter()
                    .any(|term| term.eq_ignore_ascii_case(core))
                {
                    return format!("{}{suffix}", replacer.to);
                }
            }
            word.to_string()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Splits `word` into its alphabetic core and trailing punctuation.
fn split_trailing_punct(word: &str) -> (&str, &str) {
    let mut split = word.len();
    for (index, ch) in word.char_indices().rev() {
        if ch.is_ascii_alphanumeric() || !ch.is_ascii_punctuation() {
            split = index + ch.len_utf8();
            break;
        }
        split = index;
    }
    word.split_at(split.min(word.len()))
}

/// Capitalizes the first letter and appends a period when the message does
/// not already end with sentence punctuation.
#[must_use]
pub fn fix_message(message: &str) -> String {
    let trimmed = message.trim();
    let mut result = String::with_capacity(trimmed.len() + 1);
    let mut chars = trimmed.chars();
    if let Some(first) = chars.next() {
        for upper in first.to_uppercase() {
            result.push(upper);
        }
        result.push_str(chars.as_str());
    }
    if !matches!(
        result.chars().next_back(),
        Some('.' | '!' | '?' | '…' | ':' | ')')
    ) {
        result.push('.');
    }
    result
}

/// Lowercases shouty messages: past `max_percent` uppercase letters
/// (messages shorter than `min_length` are ignored), every word is
/// lowercased except the (case-insensitive) skip words.
#[must_use]
pub fn anti_caps(message: &str, min_length: usize, max_percent: u32, skip: &[String]) -> String {
    let letters: Vec<char> = message.chars().filter(|ch| ch.is_alphabetic()).collect();
    if letters.len() < min_length {
        return message.to_string();
    }
    let uppercase = letters.iter().filter(|ch| ch.is_uppercase()).count();
    let percent = u32::try_from(uppercase * 100).unwrap_or(u32::MAX)
        / u32::try_from(letters.len()).unwrap_or(u32::MAX);
    if percent <= max_percent {
        return message.to_string();
    }
    message
        .split_whitespace()
        .map(|word| {
            if skip.iter().any(|term| term.eq_ignore_ascii_case(word)) {
                word.to_string()
            } else {
                word.to_lowercase()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Returns the first unauthorized URL host in the message, if any.
///
/// A host is a dotted token whose labels start and end with alphanumeric
/// characters; `youtu.be/watch` checks the host `youtu.be`. Paths after the
/// host and common wrappers (`www.`) are ignored.
#[must_use]
pub fn find_disallowed_url(message: &str, allowed_domains: &[String]) -> Option<String> {
    let allowed: Vec<String> = allowed_domains
        .iter()
        .map(|domain| domain.trim().to_lowercase())
        .filter(|domain| !domain.is_empty())
        .collect();

    for token in message.split_whitespace() {
        let token = token
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_start_matches("www.");
        let host = token.split(['/', '?', '#']).next().unwrap_or_default();
        if !looks_like_host(host) {
            continue;
        }
        let host = host.to_lowercase();
        if !allowed
            .iter()
            .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
        {
            return Some(host);
        }
    }
    None
}

fn looks_like_host(host: &str) -> bool {
    if !host.contains('.') {
        return false;
    }
    let labels: Vec<&str> = host.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && label.chars().next().is_some_and(char::is_alphanumeric)
                && label.chars().next_back().is_some_and(char::is_alphanumeric)
        })
        && labels
            .last()
            .is_some_and(|tld| tld.chars().all(char::is_alphabetic) && tld.len() >= 2)
}

/// Whether the message contains any blocked fragment.
///
/// Matching strips every character that is neither alphanumeric nor part
/// of the fragment, so `a,t.a.q,ue b,o.t` still matches `ataque bot`
/// (bypass protection).
#[must_use]
pub fn contains_blocked(message: &str, blocked: &[String]) -> bool {
    blocked.iter().any(|term| {
        let term = term.trim();
        if term.is_empty() {
            return false;
        }
        let allowed_extra: Vec<char> = term.chars().filter(|ch| !ch.is_alphanumeric()).collect();
        let squeezed: String = message
            .chars()
            .filter(|ch| ch.is_alphanumeric() || allowed_extra.contains(ch))
            .collect();
        squeezed.contains(term)
    })
}

/// Result of running the full filter pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterOutcome {
    /// The message may be sent, possibly rewritten.
    Send(String),
    /// The message must be swallowed (spoofed on the sender's side).
    Block,
}

/// Runs censorship first, then the rewriters.
#[must_use]
pub fn apply_filters(message: &str, config: &FiltersConfig) -> FilterOutcome {
    if config.censor_urls && find_disallowed_url(message, &config.allowed_domains).is_some() {
        return FilterOutcome::Block;
    }
    if contains_blocked(message, &config.blocked_content) {
        return FilterOutcome::Block;
    }

    let replacers = parse_replacers(&config.replacers);
    let mut result = apply_replacers(message, &replacers);
    if config.anti_caps {
        result = anti_caps(
            &result,
            config.anti_caps_min_length,
            config.anti_caps_max_percent,
            &config.anti_caps_skip,
        );
    }
    if config.fix_message {
        result = fix_message(&result);
    }
    FilterOutcome::Send(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::default_allowed_domains;

    fn filters() -> FiltersConfig {
        FiltersConfig::default()
    }

    #[test]
    fn replacer_fixes_slang_and_keeps_punctuation() {
        let replacers = parse_replacers(&filters().replacers);
        assert_eq!(apply_replacers("vc, nao sei", &replacers), "você, não sei");
        assert_eq!(apply_replacers("vc!", &replacers), "você!");
        assert_eq!(apply_replacers("amiga s2", &replacers), "amiga ❤");
        assert_eq!(apply_replacers("vcs", &replacers), "vcs");
    }

    #[test]
    fn fix_message_capitalizes_and_finishes() {
        assert_eq!(fix_message("oi gente"), "Oi gente.");
        assert_eq!(fix_message("tudo bem?"), "Tudo bem?");
        assert_eq!(fix_message("massa"), "Massa.");
    }

    #[test]
    fn anti_caps_lowercases_shouty_but_skips_words() {
        let skip = vec!["VIP".to_string(), "Master".to_string()];
        assert_eq!(
            anti_caps("ISSO EH MUITO BOM", 4, 25, &skip),
            "isso eh muito bom"
        );
        assert_eq!(
            anti_caps("VIP GANHOU Master", 4, 25, &skip),
            "VIP ganhou Master"
        );
        // Short and calm messages stay untouched.
        assert_eq!(anti_caps("OK", 4, 25, &skip), "OK");
        assert_eq!(
            anti_caps("So UM pouquinho", 4, 25, &skip),
            "So UM pouquinho"
        );
    }

    #[test]
    fn url_censor_respects_allowlist() {
        let allowed = default_allowed_domains();
        assert_eq!(find_disallowed_url("veja youtube.com/x", &allowed), None);
        assert_eq!(find_disallowed_url("entre discord.gg/abc", &allowed), None);
        assert_eq!(
            find_disallowed_url("visite ev1l.site agora", &allowed),
            Some("ev1l.site".to_string())
        );
        assert_eq!(find_disallowed_url("sem links aqui", &allowed), None);
        assert!(find_disallowed_url("meu server.net", &allowed).is_some());
    }

    #[test]
    fn blocked_content_matches_through_punctuation() {
        assert!(contains_blocked("o ip: do server", &["ip:".to_string()]));
        assert!(contains_blocked("i,p: 123", &["ip:".to_string()]));
        assert!(!contains_blocked("hipotenusa", &["ip:".to_string()]));
    }

    #[test]
    fn pipeline_blocks_before_rewriting() {
        let outcome = apply_filters("visite ev1l.com", &filters());
        assert_eq!(outcome, FilterOutcome::Block);
    }

    #[test]
    fn pipeline_rewrites_in_order() {
        let outcome = apply_filters("vc viu o VIDEO novo", &filters());
        assert_eq!(
            outcome,
            FilterOutcome::Send("Você viu o video novo.".to_string())
        );
    }
}
