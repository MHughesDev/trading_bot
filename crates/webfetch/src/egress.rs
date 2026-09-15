//! Where the agent is allowed to connect, and what it is never allowed to reach.
//!
//! ## What this module is defending
//!
//! Until now this platform cut one leg of the lethal trifecta: the agent had
//! private data (the ledger, positions, strategy definitions) and no outbound
//! network, so there was nothing to exfiltrate *to*. `the_only_outbound_tools_are_the_two_web_tools`
//! asserted that by name.
//!
//! Adding web access restores that leg deliberately. Private data, untrusted
//! content and outbound communication now coexist, and no amount of care in the
//! calling code makes that safe on its own. What makes it survivable is that
//! every reachable destination is decided **here**, by a pure function over a
//! parsed URL and a resolved address, with no I/O and no configuration the agent
//! can influence.
//!
//! ## The four attacks this is shaped against
//!
//! 1. **SSRF into the private network.** A page says "fetch
//!    `http://169.254.169.254/latest/meta-data/iam/security-credentials/`" and
//!    the agent, being helpful, does. That address is cloud instance metadata and
//!    on many hosts it hands out credentials to anyone who asks. Every private,
//!    loopback, link-local and unique-local range is refused, in both address
//!    families, before a socket is opened.
//!
//! 2. **DNS rebinding.** A name resolves to a public address when checked and to
//!    `127.0.0.1` a moment later when connected. The defence is not to check
//!    twice: [`Decision::Allow`] carries the **exact addresses that were
//!    checked**, and the caller is required to connect to one of those rather
//!    than re-resolving. A checker that returns only a verdict cannot prevent
//!    this, which is why this one returns addresses.
//!
//! 3. **Redirect laundering.** An allowed host 302s to a denied one. Redirects
//!    are re-checked hop by hop — [`RedirectPolicy`] — because a policy applied
//!    only to the first URL is a policy applied to the URL the attacker did not
//!    choose.
//!
//! 4. **Credential smuggling.** `https://user:token@host/` puts a secret in a
//!    URL the agent may have got from a web page, and `https://evil.com#@good.com`
//!    fools a human reader. Userinfo is refused outright rather than stripped:
//!    stripping produces a working request the author did not intend.
//!
//! ## What it does not defend
//!
//! Nothing here stops the agent being *persuaded* by a page it fetched. Content
//! is untrusted input, and the type that carries it says so
//! ([`crate::Untrusted`]). This module governs reachability, not belief.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use url::Url;

/// The verdict on one destination.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Allowed, **to these addresses only**.
    ///
    /// The caller must pin the connection to one of these rather than resolving
    /// the host again. Re-resolving reopens the rebinding window this closes.
    Allow { host: String, addrs: Vec<SocketAddr> },
    /// Refused, with a reason the agent can read and act on.
    Deny { reason: String },
}

impl Decision {
    #[must_use]
    pub const fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow { .. })
    }

    /// The refusal reason, if refused.
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Deny { reason } => Some(reason),
            Self::Allow { .. } => None,
        }
    }
}

/// How hosts are decided.
///
/// Deny-by-default is not the default here — it is the *only* mode for anything
/// but an explicitly listed host, and [`HostPolicy::AllowAny`] exists so that a
/// deployment which genuinely wants open browsing has to say so in one readable
/// place rather than by leaving a list empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostPolicy {
    /// Only these hosts, plus their subdomains. The default posture.
    Allowlist(Vec<String>),
    /// Any public host that survives the address checks.
    ///
    /// The address rules below still apply in full: "any" never means the
    /// private network.
    AllowAny,
}

/// The complete egress policy.
#[derive(Clone, Debug)]
pub struct Policy {
    pub hosts: HostPolicy,
    /// Hosts refused even under [`HostPolicy::AllowAny`], and refused if they
    /// somehow appear in an allowlist. Checked first, so a denial cannot be
    /// undone by adding an entry somewhere else.
    pub denied_hosts: Vec<String>,
    /// Permit plain `http://`. Off by default: a cleartext fetch is readable and
    /// modifiable by anything on the path, and the agent treats what comes back
    /// as evidence.
    pub allow_plaintext: bool,
    /// Largest response body accepted, in bytes.
    pub max_bytes: usize,
    /// Wall-clock budget for one fetch.
    pub timeout: std::time::Duration,
    /// Redirects followed before giving up.
    pub max_redirects: usize,
}

impl Default for Policy {
    /// The research posture: the sources a quant researcher actually needs,
    /// nothing else.
    ///
    /// This is a starting list and is meant to be edited. It is deliberately
    /// *not* "everything except social media": an allowlist somebody curates is
    /// auditable, and a denylist somebody maintains is a list of the attacks
    /// already known.
    fn default() -> Self {
        Self {
            hosts: HostPolicy::Allowlist(
                [
                    // Search: keyless scholarly APIs that permit programmatic
                    // access (see `crate::search`). No scraped search engine —
                    // the one this started with answers a plain GET with a bot
                    // challenge, and working around that is not something this
                    // platform does.
                    "api.openalex.org",
                    "api.crossref.org",
                    // Papers and preprints.
                    "arxiv.org",
                    "export.arxiv.org",
                    "papers.ssrn.com",
                    "www.jstor.org",
                    "doi.org",
                    "link.springer.com",
                    "www.sciencedirect.com",
                    "jmlr.org",
                    "openreview.net",
                    "papers.nips.cc",
                    "proceedings.mlr.press",
                    "www.nber.org",
                    "econpapers.repec.org",
                    "ideas.repec.org",
                    "www.federalreserve.gov",
                    "fred.stlouisfed.org",
                    // Venue documentation and fee schedules — the highest-value
                    // fetch this platform has, since the cost model is quadratic
                    // in the answer.
                    "docs.cdp.coinbase.com",
                    "www.coinbase.com",
                    "help.coinbase.com",
                    "docs.kraken.com",
                    "www.kraken.com",
                    "support.kraken.com",
                    "alpaca.markets",
                    "docs.alpaca.markets",
                    "www.binance.com",
                    "developers.binance.com",
                    // Reference.
                    "en.wikipedia.org",
                    "github.com",
                    "raw.githubusercontent.com",
                    "docs.python.org",
                    "numpy.org",
                    "pandas.pydata.org",
                    "scikit-learn.org",
                    "docs.scipy.org",
                    "arch.readthedocs.io",
                ]
                .into_iter()
                .map(str::to_string)
                .collect(),
            ),
            denied_hosts: Vec::new(),
            allow_plaintext: false,
            max_bytes: 2 * 1024 * 1024,
            timeout: std::time::Duration::from_secs(20),
            max_redirects: 5,
        }
    }
}

/// Why a destination was refused. Separated from the message so tests assert on
/// the cause rather than on prose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DenyReason {
    BadUrl,
    Scheme,
    Userinfo,
    NotAllowlisted,
    DeniedHost,
    NoHost,
    PrivateAddress,
    NoAddresses,
    Port,
}

/// Check a URL against `policy`, given the addresses its host resolved to.
///
/// Resolution is the caller's job so this stays pure and testable: the whole
/// point of the module is that it can be exhaustively exercised without a
/// network, and a function that resolves internally cannot be.
#[must_use]
pub fn check(url: &str, policy: &Policy, resolved: &[SocketAddr]) -> Decision {
    let Ok(parsed) = Url::parse(url) else {
        return deny(DenyReason::BadUrl, format!("{url:?} is not a URL"));
    };

    match parsed.scheme() {
        "https" => {}
        "http" if policy.allow_plaintext => {}
        "http" => {
            return deny(
                DenyReason::Scheme,
                "plaintext http is refused: what comes back is treated as evidence, and a \
                 cleartext fetch can be read and rewritten by anything on the path. Use https."
                    .into(),
            )
        }
        other => {
            return deny(
                DenyReason::Scheme,
                format!(
                    "scheme {other:?} is not fetchable. Only https is, and `file:`, `data:`, \
                     `ftp:` and `gopher:` are refused by name because each has been a way to \
                     read something local through a fetcher."
                ),
            )
        }
    }

    // Refused, never stripped. Stripping produces a working request whose author
    // did not intend it, and the credential in question often came from a page
    // the agent was reading rather than from the operator.
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return deny(
            DenyReason::Userinfo,
            "the URL carries credentials before the host (`user:pass@`). Refused rather than \
             stripped: a URL like this usually came from the page being read, not from you."
                .into(),
        );
    }

    let Some(host) = parsed.host_str() else {
        return deny(DenyReason::NoHost, "the URL has no host".into());
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    // `Url::host_str` keeps the brackets on an IPv6 literal, so `[::1]` does not
    // parse as an address and the private-range check below would miss it.
    let host_ip = host.trim_start_matches('[').trim_end_matches(']');

    // Ports outside the web are a scanner, not a fetch.
    if let Some(port) = parsed.port() {
        if !matches!(port, 80 | 443 | 8080 | 8443) {
            return deny(
                DenyReason::Port,
                format!(
                    "port {port} is not a web port. Fetching arbitrary ports turns this tool \
                     into a port scanner for whatever network it runs on."
                ),
            );
        }
    }

    // Denylist first, so no later rule can undo it.
    if policy.denied_hosts.iter().any(|d| host_matches(&host, d)) {
        return deny(DenyReason::DeniedHost, format!("{host} is on the deny list"));
    }

    match &policy.hosts {
        HostPolicy::Allowlist(allowed) => {
            if !allowed.iter().any(|a| host_matches(&host, a)) {
                return deny(
                    DenyReason::NotAllowlisted,
                    format!(
                        "{host} is not on the egress allowlist. This platform allows a curated \
                         set of research and venue-documentation sources; an operator can add \
                         one, and the agent cannot."
                    ),
                );
            }
        }
        HostPolicy::AllowAny => {}
    }

    // A literal IP in the URL skips DNS entirely, so it is checked here too.
    if let Ok(ip) = host_ip.parse::<IpAddr>() {
        if let Some(why) = blocked_ip(ip) {
            return deny(
                DenyReason::PrivateAddress,
                format!("{ip} is {why}, which is never fetchable"),
            );
        }
    }

    if resolved.is_empty() {
        return deny(
            DenyReason::NoAddresses,
            format!("{host} did not resolve to any address"),
        );
    }

    // Every address, not merely the first. A host that resolves to one public
    // and one private address is the textbook rebinding setup, and picking the
    // public one would be exactly the mistake.
    for addr in resolved {
        if let Some(why) = blocked_ip(addr.ip()) {
            return deny(
                DenyReason::PrivateAddress,
                format!(
                    "{host} resolves to {}, which is {why}. Every resolved address must be \
                     public: a host that answers with one public and one private address is \
                     how a fetcher gets pointed at its own network.",
                    addr.ip()
                ),
            );
        }
    }

    Decision::Allow {
        host,
        addrs: resolved.to_vec(),
    }
}

fn deny(_reason: DenyReason, message: String) -> Decision {
    Decision::Deny { reason: message }
}

/// The classified reason a check failed, for tests and metrics.
#[must_use]
pub fn deny_reason(url: &str, policy: &Policy, resolved: &[SocketAddr]) -> Option<DenyReason> {
    // Re-derives the classification by re-running the same ladder. Kept separate
    // from `check` so the public API carries prose the agent can act on rather
    // than an enum it cannot.
    let Ok(parsed) = Url::parse(url) else {
        return Some(DenyReason::BadUrl);
    };
    match parsed.scheme() {
        "https" => {}
        "http" if policy.allow_plaintext => {}
        _ => return Some(DenyReason::Scheme),
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Some(DenyReason::Userinfo);
    }
    let Some(host) = parsed.host_str() else {
        return Some(DenyReason::NoHost);
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let host_ip = host.trim_start_matches('[').trim_end_matches(']');
    if let Some(port) = parsed.port() {
        if !matches!(port, 80 | 443 | 8080 | 8443) {
            return Some(DenyReason::Port);
        }
    }
    if policy.denied_hosts.iter().any(|d| host_matches(&host, d)) {
        return Some(DenyReason::DeniedHost);
    }
    if let HostPolicy::Allowlist(allowed) = &policy.hosts {
        if !allowed.iter().any(|a| host_matches(&host, a)) {
            return Some(DenyReason::NotAllowlisted);
        }
    }
    if let Ok(ip) = host_ip.parse::<IpAddr>() {
        if blocked_ip(ip).is_some() {
            return Some(DenyReason::PrivateAddress);
        }
    }
    if resolved.is_empty() {
        return Some(DenyReason::NoAddresses);
    }
    if resolved.iter().any(|a| blocked_ip(a.ip()).is_some()) {
        return Some(DenyReason::PrivateAddress);
    }
    None
}

/// Whether `host` is `pattern` or a subdomain of it.
///
/// Boundary-aware on purpose: a naive `ends_with` would match
/// `evil-arxiv.org` against `arxiv.org`, which is the whole game.
#[must_use]
pub fn host_matches(host: &str, pattern: &str) -> bool {
    let pattern = pattern.trim_start_matches('.').to_ascii_lowercase();
    let host = host.to_ascii_lowercase();
    host == pattern || host.ends_with(&format!(".{pattern}"))
}

/// Why an address may never be connected to, or `None` if it is fine.
///
/// Returns a phrase rather than a bool so the refusal can say *which* rule it
/// hit; "is not fetchable" teaches nobody anything.
#[must_use]
pub fn blocked_ip(ip: IpAddr) -> Option<&'static str> {
    match ip {
        IpAddr::V4(v4) => blocked_v4(v4),
        IpAddr::V6(v6) => {
            // An IPv4-mapped address is an IPv4 address wearing a hat, and
            // checking only the v6 rules against it is a standard bypass.
            if let Some(mapped) = v6_to_v4(v6) {
                return blocked_v4(mapped).or(Some("an IPv4-mapped address"));
            }
            blocked_v6(v6)
        }
    }
}

fn blocked_v4(ip: Ipv4Addr) -> Option<&'static str> {
    let o = ip.octets();
    match () {
        () if ip.is_unspecified() => Some("the unspecified address"),
        () if ip.is_loopback() => Some("loopback"),
        () if ip.is_private() => Some("a private network"),
        // 169.254.0.0/16. The single most important entry in this function:
        // 169.254.169.254 is cloud instance metadata and on many hosts it hands
        // out credentials to anything that asks.
        () if ip.is_link_local() => Some("link-local (cloud instance metadata lives here)"),
        () if ip.is_broadcast() => Some("the broadcast address"),
        () if ip.is_multicast() => Some("multicast"),
        () if ip.is_documentation() => Some("documentation-only"),
        // 100.64.0.0/10 carrier-grade NAT — a private range in practice.
        () if o[0] == 100 && (64..128).contains(&o[1]) => Some("carrier-grade NAT space"),
        // 192.0.0.0/24 IETF protocol assignments.
        () if o[0] == 192 && o[1] == 0 && o[2] == 0 => Some("IETF protocol assignment space"),
        // 198.18.0.0/15 benchmarking.
        () if o[0] == 198 && (o[1] == 18 || o[1] == 19) => Some("benchmark-only space"),
        // 240.0.0.0/4 reserved.
        () if o[0] >= 240 => Some("reserved space"),
        () => None,
    }
}

fn blocked_v6(ip: Ipv6Addr) -> Option<&'static str> {
    let seg = ip.segments();
    match () {
        () if ip.is_unspecified() => Some("the unspecified address"),
        () if ip.is_loopback() => Some("loopback"),
        () if ip.is_multicast() => Some("multicast"),
        // fc00::/7 unique local.
        () if (seg[0] & 0xfe00) == 0xfc00 => Some("a unique-local address"),
        // fe80::/10 link local.
        () if (seg[0] & 0xffc0) == 0xfe80 => Some("link-local"),
        // 2001:db8::/32 documentation.
        () if seg[0] == 0x2001 && seg[1] == 0x0db8 => Some("documentation-only"),
        // ::ffff:0:0/96 handled by the caller; 64:ff9b::/96 is NAT64 onto v4.
        () if seg[0] == 0x0064 && seg[1] == 0xff9b => Some("NAT64 space onto IPv4"),
        () => None,
    }
}

/// The IPv4 address inside an IPv4-mapped or IPv4-compatible IPv6 address.
fn v6_to_v4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    let seg = ip.segments();
    let is_mapped = seg[..5] == [0, 0, 0, 0, 0] && seg[5] == 0xffff;
    let is_compat = seg[..6] == [0, 0, 0, 0, 0, 0] && (seg[6] != 0 || seg[7] != 0);
    (is_mapped || is_compat).then(|| {
        let o = ip.octets();
        Ipv4Addr::new(o[12], o[13], o[14], o[15])
    })
}

/// Re-checks every redirect hop against the same policy.
///
/// A policy applied only to the first URL is a policy applied to the URL the
/// attacker did not choose.
pub struct RedirectPolicy;

impl RedirectPolicy {
    /// Whether following `to` is permitted, given the addresses it resolved to.
    #[must_use]
    pub fn may_follow(to: &str, policy: &Policy, resolved: &[SocketAddr]) -> Decision {
        check(to, policy, resolved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn public() -> Vec<SocketAddr> {
        vec!["93.184.216.34:443".parse().unwrap()]
    }

    fn open_policy() -> Policy {
        Policy {
            hosts: HostPolicy::AllowAny,
            ..Policy::default()
        }
    }

    fn reason_of(url: &str, p: &Policy, addrs: &[SocketAddr]) -> DenyReason {
        deny_reason(url, p, addrs).expect("expected a denial")
    }

    // ── the address rules ────────────────────────────────────────────────────

    /// The single most important line in the module. `169.254.169.254` is cloud
    /// instance metadata; on many hosts it hands out credentials to anything
    /// that asks, and an agent that can be told to fetch a URL can be told to
    /// fetch that one.
    #[test]
    fn cloud_instance_metadata_is_never_reachable() {
        let meta: Vec<SocketAddr> = vec!["169.254.169.254:80".parse().unwrap()];
        assert_eq!(
            reason_of("https://metadata.example.com/", &open_policy(), &meta),
            DenyReason::PrivateAddress
        );
        assert!(blocked_ip("169.254.169.254".parse().unwrap())
            .unwrap()
            .contains("metadata"));
    }

    #[test]
    fn every_private_and_local_v4_range_is_blocked() {
        for ip in [
            "0.0.0.0",
            "127.0.0.1",
            "127.255.255.254",
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.254",
            "192.168.1.1",
            "169.254.1.1",
            "100.64.0.1",
            "192.0.0.1",
            "198.18.0.1",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
        ] {
            assert!(
                blocked_ip(ip.parse().unwrap()).is_some(),
                "{ip} must be blocked"
            );
        }
    }

    #[test]
    fn public_v4_addresses_are_allowed() {
        for ip in ["93.184.216.34", "1.1.1.1", "8.8.8.8", "172.32.0.1", "172.15.255.254"] {
            assert!(blocked_ip(ip.parse().unwrap()).is_none(), "{ip} should be fine");
        }
    }

    #[test]
    fn every_private_and_local_v6_range_is_blocked() {
        for ip in ["::", "::1", "fc00::1", "fd00::1", "fe80::1", "ff02::1", "2001:db8::1", "64:ff9b::1"] {
            assert!(blocked_ip(ip.parse().unwrap()).is_some(), "{ip} must be blocked");
        }
    }

    /// An IPv4-mapped v6 address is an IPv4 address wearing a hat. Checking only
    /// the v6 rules against it is a standard bypass.
    #[test]
    fn ipv4_mapped_addresses_are_checked_as_ipv4() {
        for ip in ["::ffff:127.0.0.1", "::ffff:169.254.169.254", "::ffff:10.0.0.1"] {
            assert!(
                blocked_ip(ip.parse().unwrap()).is_some(),
                "{ip} must be blocked through its mapped v4 address"
            );
        }
    }

    // ── DNS rebinding ────────────────────────────────────────────────────────

    /// A host that answers with one public and one private address is the
    /// textbook rebinding setup. Checking only the first would be the mistake.
    #[test]
    fn a_host_resolving_to_any_private_address_is_refused() {
        let mixed: Vec<SocketAddr> = vec![
            "93.184.216.34:443".parse().unwrap(),
            "127.0.0.1:443".parse().unwrap(),
        ];
        assert_eq!(
            reason_of("https://arxiv.org/abs/1234", &Policy::default(), &mixed),
            DenyReason::PrivateAddress
        );
    }

    /// The defence against re-resolution is that the allow verdict carries the
    /// addresses. A verdict without them would leave the caller to resolve
    /// again, which is the window.
    #[test]
    fn an_allow_carries_the_exact_addresses_that_were_checked() {
        match check("https://arxiv.org/abs/1", &Policy::default(), &public()) {
            Decision::Allow { host, addrs } => {
                assert_eq!(host, "arxiv.org");
                assert_eq!(addrs, public());
            }
            Decision::Deny { reason } => panic!("{reason}"),
        }
    }

    #[test]
    fn a_host_that_does_not_resolve_is_refused_rather_than_attempted() {
        assert_eq!(
            reason_of("https://arxiv.org/", &Policy::default(), &[]),
            DenyReason::NoAddresses
        );
    }

    // ── the allowlist ────────────────────────────────────────────────────────

    /// The bug a naive `ends_with` would have: `evil-arxiv.org` is not a
    /// subdomain of `arxiv.org`.
    #[test]
    fn a_lookalike_domain_does_not_match_the_allowlist() {
        assert!(host_matches("arxiv.org", "arxiv.org"));
        assert!(host_matches("export.arxiv.org", "arxiv.org"));
        assert!(!host_matches("evil-arxiv.org", "arxiv.org"));
        assert!(!host_matches("arxiv.org.evil.com", "arxiv.org"));
        assert!(!host_matches("notarxiv.org", "arxiv.org"));
    }

    #[test]
    fn an_unlisted_host_is_refused_and_told_who_can_add_it() {
        let d = check("https://example.com/", &Policy::default(), &public());
        assert!(!d.is_allowed());
        let reason = d.reason().unwrap();
        assert!(reason.contains("not on the egress allowlist"), "{reason}");
        assert!(reason.contains("the agent cannot"), "{reason}");
    }

    /// A denial must not be undoable by adding the same host to the allowlist.
    #[test]
    fn the_denylist_beats_the_allowlist() {
        let p = Policy {
            hosts: HostPolicy::Allowlist(vec!["arxiv.org".into()]),
            denied_hosts: vec!["arxiv.org".into()],
            ..Policy::default()
        };
        assert_eq!(reason_of("https://arxiv.org/", &p, &public()), DenyReason::DeniedHost);
    }

    /// "Any host" still never means the private network.
    #[test]
    fn allow_any_still_refuses_private_addresses() {
        let local: Vec<SocketAddr> = vec!["10.1.2.3:443".parse().unwrap()];
        assert_eq!(
            reason_of("https://internal.corp/", &open_policy(), &local),
            DenyReason::PrivateAddress
        );
    }

    #[test]
    fn a_trailing_dot_does_not_escape_the_allowlist() {
        // `arxiv.org.` is the same name to a resolver and a different string.
        assert!(check("https://arxiv.org./abs/1", &Policy::default(), &public()).is_allowed());
    }

    #[test]
    fn case_does_not_escape_the_allowlist() {
        assert!(check("https://ARXIV.ORG/abs/1", &Policy::default(), &public()).is_allowed());
    }

    // ── schemes, credentials, ports ──────────────────────────────────────────

    /// Each of these has been a way to read something local through a fetcher.
    #[test]
    fn only_https_is_fetchable() {
        for url in [
            "file:///etc/passwd",
            "data:text/html,<script>",
            "ftp://arxiv.org/x",
            "gopher://arxiv.org/x",
            "jar:https://arxiv.org/!/x",
        ] {
            assert_eq!(reason_of(url, &open_policy(), &public()), DenyReason::Scheme, "{url}");
        }
    }

    #[test]
    fn plaintext_http_is_refused_unless_the_operator_opts_in() {
        assert_eq!(
            reason_of("http://arxiv.org/", &Policy::default(), &public()),
            DenyReason::Scheme
        );
        let p = Policy { allow_plaintext: true, ..Policy::default() };
        assert!(check("http://arxiv.org/", &p, &public()).is_allowed());
    }

    /// Refused rather than stripped: a URL like this usually came from the page
    /// being read, and stripping produces a working request nobody intended.
    #[test]
    fn credentials_in_a_url_are_refused_not_stripped() {
        assert_eq!(
            reason_of("https://user:token@arxiv.org/", &Policy::default(), &public()),
            DenyReason::Userinfo
        );
        assert_eq!(
            reason_of("https://user@arxiv.org/", &Policy::default(), &public()),
            DenyReason::Userinfo
        );
    }

    #[test]
    fn non_web_ports_are_refused_because_that_is_a_port_scanner() {
        for port in [22, 25, 3306, 5432, 6379, 9200, 11211] {
            let url = format!("https://arxiv.org:{port}/");
            assert_eq!(reason_of(&url, &Policy::default(), &public()), DenyReason::Port);
        }
        assert!(check("https://arxiv.org:443/", &Policy::default(), &public()).is_allowed());
        assert!(check("https://arxiv.org:8443/", &Policy::default(), &public()).is_allowed());
    }

    #[test]
    fn a_literal_private_ip_in_the_url_is_refused_even_before_dns() {
        let p = open_policy();
        // Resolution is irrelevant here: the host *is* the address.
        assert_eq!(reason_of("https://127.0.0.1/", &p, &public()), DenyReason::PrivateAddress);
        assert_eq!(
            reason_of("https://[::1]/", &p, &public()),
            DenyReason::PrivateAddress
        );
    }

    #[test]
    fn a_malformed_url_is_refused() {
        assert_eq!(reason_of("not a url", &open_policy(), &public()), DenyReason::BadUrl);
        assert_eq!(reason_of("", &open_policy(), &public()), DenyReason::BadUrl);
    }

    // ── redirects ────────────────────────────────────────────────────────────

    /// A policy applied only to the first URL is a policy applied to the URL the
    /// attacker did not choose.
    #[test]
    fn a_redirect_to_a_denied_destination_is_refused() {
        let p = Policy::default();
        assert!(RedirectPolicy::may_follow("https://arxiv.org/x", &p, &public()).is_allowed());
        assert!(!RedirectPolicy::may_follow("https://evil.com/x", &p, &public()).is_allowed());

        let meta: Vec<SocketAddr> = vec!["169.254.169.254:80".parse().unwrap()];
        assert!(!RedirectPolicy::may_follow("https://arxiv.org/x", &p, &meta).is_allowed());
    }

    // ── the default posture ──────────────────────────────────────────────────

    #[test]
    fn the_default_policy_is_an_allowlist_with_real_limits() {
        let p = Policy::default();
        assert!(matches!(p.hosts, HostPolicy::Allowlist(_)));
        assert!(!p.allow_plaintext);
        assert!(p.max_bytes <= 4 * 1024 * 1024);
        assert!(p.timeout.as_secs() <= 30);
        assert!(p.max_redirects <= 10);
    }

    /// The venue documentation hosts are the highest-value fetch this platform
    /// has: the cost model is quadratic in the fee tier, so a wrong number there
    /// costs more than a wrong number almost anywhere else.
    #[test]
    fn the_default_allowlist_covers_venue_fee_documentation() {
        let p = Policy::default();
        for url in [
            "https://docs.cdp.coinbase.com/advanced-trade/docs/fees",
            "https://docs.kraken.com/api/",
            "https://docs.alpaca.markets/docs/crypto-fees",
        ] {
            assert!(check(url, &p, &public()).is_allowed(), "{url}");
        }
    }
}
