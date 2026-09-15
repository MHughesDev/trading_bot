//! robots.txt, honoured.
//!
//! Not a security control — a site cannot stop us with a text file, and nothing
//! here defends the platform. It is honoured because a research agent that
//! ignores robots.txt gets the platform's IP blocked by the exact sources it
//! most needs, and because a crawler that takes what it is asked not to take is
//! a bad neighbour at scale.
//!
//! The parse is the subset that is actually used: `User-agent`, `Disallow`,
//! `Allow`, `Crawl-delay`, with longest-match-wins between Allow and Disallow as
//! every major crawler implements it. Sitemap and wildcards beyond `*` and `$`
//! are ignored.
//!
//! **A file that cannot be fetched means allowed.** That is the standard
//! reading, and the alternative — treating an unreachable robots.txt as a
//! blanket refusal — makes the tool fail closed on transient errors in a way
//! nobody would debug.

use std::time::Duration;

/// The rules that apply to one user-agent.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rules {
    /// `(path_pattern, allow)`, in file order.
    rules: Vec<(String, bool)>,
    crawl_delay: Option<Duration>,
}

impl Rules {
    /// Nothing disallowed. What an absent or unfetchable robots.txt means.
    #[must_use]
    pub fn permissive() -> Self {
        Self::default()
    }

    /// Whether `path` may be fetched.
    ///
    /// Longest matching pattern wins; a tie goes to `Allow`, which is what
    /// Google and the RFC both specify and is the more permissive reading of an
    /// ambiguous file.
    #[must_use]
    pub fn allows(&self, path: &str) -> bool {
        let mut best: Option<(usize, bool)> = None;
        for (pattern, allow) in &self.rules {
            if let Some(len) = match_len(pattern, path) {
                best = match best {
                    Some((best_len, best_allow)) if best_len > len => Some((best_len, best_allow)),
                    Some((best_len, _)) if best_len == len && *allow => Some((len, true)),
                    Some((best_len, best_allow)) if best_len == len => Some((best_len, best_allow)),
                    _ => Some((len, *allow)),
                };
            }
        }
        best.is_none_or(|(_, allow)| allow)
    }

    /// The requested delay between fetches, if the file asked for one.
    #[must_use]
    pub const fn crawl_delay(&self) -> Option<Duration> {
        self.crawl_delay
    }
}

/// Parse a robots.txt for `agent`.
///
/// Groups for `*` and for `agent` are both collected, with the specific group
/// taking precedence when one exists — which is the behaviour a site author
/// expects when they write a rule naming us.
#[must_use]
pub fn parse(body: &str, agent: &str) -> Rules {
    let agent = agent.to_ascii_lowercase();
    let mut star = Rules::default();
    let mut specific = Rules::default();
    let mut found_specific = false;

    // Which groups the current block applies to. A run of consecutive
    // `User-agent` lines shares one block, which is what the format means and is
    // easy to get wrong by resetting on every line.
    let (mut in_star, mut in_specific, mut last_was_agent) = (false, false, false);

    for raw in body.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();

        match key.as_str() {
            "user-agent" => {
                if !last_was_agent {
                    in_star = false;
                    in_specific = false;
                }
                let ua = value.to_ascii_lowercase();
                if ua == "*" {
                    in_star = true;
                } else if agent.contains(&ua) || ua.contains(&agent) {
                    in_specific = true;
                    found_specific = true;
                }
                last_was_agent = true;
            }
            "disallow" | "allow" => {
                last_was_agent = false;
                let allow = key == "allow";
                // An empty `Disallow:` means "nothing is disallowed" and must
                // not be recorded as a rule matching every path.
                if value.is_empty() && !allow {
                    continue;
                }
                if in_star {
                    star.rules.push((value.to_string(), allow));
                }
                if in_specific {
                    specific.rules.push((value.to_string(), allow));
                }
            }
            "crawl-delay" => {
                last_was_agent = false;
                if let Ok(secs) = value.parse::<f64>() {
                    if secs.is_finite() && secs >= 0.0 {
                        let d = Duration::from_secs_f64(secs.min(60.0));
                        if in_star {
                            star.crawl_delay = Some(d);
                        }
                        if in_specific {
                            specific.crawl_delay = Some(d);
                        }
                    }
                }
            }
            _ => last_was_agent = false,
        }
    }

    if found_specific {
        specific
    } else {
        star
    }
}

/// Length of `pattern`'s match against `path`, or `None`.
///
/// Supports `*` (any run) and a trailing `$` (end anchor), which between them
/// cover essentially every rule in the wild.
fn match_len(pattern: &str, path: &str) -> Option<usize> {
    let (body, anchored) = pattern
        .strip_suffix('$')
        .map_or((pattern, false), |b| (b, true));

    if !body.contains('*') {
        let hit = if anchored { path == body } else { path.starts_with(body) };
        return hit.then_some(body.len());
    }

    let mut pos = 0usize;
    let mut first = true;
    for part in body.split('*') {
        if part.is_empty() {
            first = false;
            continue;
        }
        let hay = &path[pos..];
        let found = if first { hay.starts_with(part).then_some(0) } else { hay.find(part) }?;
        pos += found + part.len();
        first = false;
    }
    if anchored && pos != path.len() {
        return None;
    }
    Some(body.len())
}

/// The user-agent this platform presents.
///
/// Identifies the software and points at nothing. A crawler that lies about who
/// it is has no standing to complain when it is blocked, and one that presents a
/// browser string is pretending to be a person.
pub const USER_AGENT: &str = "TradingBotResearchAgent/1.0 (+self-hosted; research; respects robots.txt)";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_file_allows_everything() {
        let r = Rules::permissive();
        assert!(r.allows("/anything"));
        assert!(r.allows("/"));
        assert!(r.crawl_delay().is_none());
    }

    #[test]
    fn a_star_group_applies_when_no_specific_group_exists() {
        let r = parse("User-agent: *\nDisallow: /private\n", USER_AGENT);
        assert!(!r.allows("/private/x"));
        assert!(r.allows("/public"));
    }

    /// A rule naming us takes precedence over the wildcard group, which is what
    /// a site author expects when they write one.
    #[test]
    fn a_specific_group_overrides_the_star_group() {
        let body = "User-agent: *\nDisallow: /\n\nUser-agent: TradingBotResearchAgent\nDisallow: /admin\n";
        let r = parse(body, USER_AGENT);
        assert!(r.allows("/papers"), "the star group's blanket deny must not apply");
        assert!(!r.allows("/admin/x"));
    }

    /// A run of consecutive `User-agent` lines is one block. Resetting on every
    /// line is the easy mistake and silently drops the rules.
    #[test]
    fn consecutive_user_agent_lines_share_one_block() {
        let r = parse("User-agent: foo\nUser-agent: *\nDisallow: /x\n", USER_AGENT);
        assert!(!r.allows("/x/y"));
    }

    /// `Disallow:` with nothing after it means "nothing is disallowed", not "the
    /// empty prefix, which matches every path".
    #[test]
    fn an_empty_disallow_permits_everything() {
        let r = parse("User-agent: *\nDisallow:\n", USER_AGENT);
        assert!(r.allows("/anything"));
    }

    #[test]
    fn longest_match_wins_between_allow_and_disallow() {
        let r = parse("User-agent: *\nDisallow: /docs\nAllow: /docs/public\n", USER_AGENT);
        assert!(!r.allows("/docs/secret"));
        assert!(r.allows("/docs/public/a"), "the longer Allow wins");
    }

    /// An ambiguous file should read the permissive way, as every major crawler
    /// implements it.
    #[test]
    fn an_exact_tie_goes_to_allow() {
        let r = parse("User-agent: *\nDisallow: /x\nAllow: /x\n", USER_AGENT);
        assert!(r.allows("/x"));
    }

    #[test]
    fn wildcards_and_end_anchors_work() {
        let r = parse("User-agent: *\nDisallow: /*.pdf$\nDisallow: /a/*/secret\n", USER_AGENT);
        assert!(!r.allows("/papers/x.pdf"));
        assert!(r.allows("/papers/x.pdf.html"), "$ anchors the end");
        assert!(!r.allows("/a/b/secret"));
        assert!(r.allows("/a/b/public"));
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let r = parse("# comment\n\nUser-agent: *   # trailing\nDisallow: /p  # why\n", USER_AGENT);
        assert!(!r.allows("/p/x"));
    }

    #[test]
    fn crawl_delay_is_read_and_capped() {
        assert_eq!(
            parse("User-agent: *\nCrawl-delay: 2.5\n", USER_AGENT).crawl_delay(),
            Some(Duration::from_secs_f64(2.5))
        );
        assert_eq!(
            parse("User-agent: *\nCrawl-delay: 9999\n", USER_AGENT).crawl_delay(),
            Some(Duration::from_secs(60)),
            "a hostile delay is capped rather than obeyed"
        );
        assert_eq!(parse("User-agent: *\nCrawl-delay: soon\n", USER_AGENT).crawl_delay(), None);
    }

    #[test]
    fn a_file_of_junk_parses_to_something_permissive_rather_than_panicking() {
        let r = parse("\u{0}\u{1}garbage\nno colons here\n:::\n", USER_AGENT);
        assert!(r.allows("/"));
    }

    /// A crawler that presents a browser string is pretending to be a person.
    #[test]
    fn the_user_agent_identifies_the_software_and_claims_nothing_else() {
        assert!(USER_AGENT.contains("TradingBot"));
        assert!(USER_AGENT.contains("robots.txt"));
        for browser in ["Mozilla", "Chrome", "Safari", "Gecko"] {
            assert!(!USER_AGENT.contains(browser), "must not impersonate a browser");
        }
    }
}
