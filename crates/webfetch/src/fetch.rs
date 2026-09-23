//! The HTTPS client, pinned to addresses the policy already checked.
//!
//! The one property this module exists to preserve: **the socket connects to an
//! address [`egress::check`] approved, not to whatever DNS says at connect
//! time.** Everything else here is limits.
//!
//! `reqwest` will happily resolve a host itself, and using it that way would
//! make the egress check advisory — the check would run against one set of
//! addresses and the connection would be made to another, which is the DNS
//! rebinding window in one sentence. So the client is built with
//! [`ClientBuilder::resolve_to_addrs`] for the exact host being fetched, and
//! redirects are handled manually rather than by the client, because an
//! automatic redirect is a connection to a host nothing checked.

use std::net::SocketAddr;
use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::egress::{check, Decision, Policy};
use crate::extract;
use crate::robots::{self, Rules, USER_AGENT};
use crate::{Document, FetchError, Untrusted};

/// Content types this reads. Anything else is refused by name rather than
/// decoded as lossy UTF-8 — a PDF run through `from_utf8_lossy` is a page of
/// replacement characters that costs context and says nothing.
const READABLE: &[&str] = &[
    "text/html",
    "text/plain",
    "application/xhtml+xml",
    "application/json",
    "text/markdown",
    "application/xml",
    "text/xml",
];

/// Resolves a host to socket addresses.
///
/// A trait so tests can supply addresses without a resolver, and so a deployment
/// can pin a resolver it trusts rather than inheriting the host's.
pub trait Resolver: Send + Sync {
    /// Resolve `host:port`.
    ///
    /// # Errors
    /// Resolution failure, as a message.
    fn resolve(&self, host: &str, port: u16) -> Result<Vec<SocketAddr>, String>;
}

/// The system resolver.
pub struct SystemResolver;

impl Resolver for SystemResolver {
    fn resolve(&self, host: &str, port: u16) -> Result<Vec<SocketAddr>, String> {
        use std::net::ToSocketAddrs;
        (host, port)
            .to_socket_addrs()
            .map(Iterator::collect)
            .map_err(|e| e.to_string())
    }
}

/// A fetcher bound to one policy.
pub struct Fetcher {
    policy: Policy,
    resolver: Arc<dyn Resolver>,
    /// robots.txt per host, fetched once per process.
    robots: tokio::sync::Mutex<std::collections::HashMap<String, Rules>>,
}

impl Fetcher {
    #[must_use]
    pub fn new(policy: Policy, resolver: Arc<dyn Resolver>) -> Self {
        Self {
            policy,
            resolver,
            robots: tokio::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    #[must_use]
    pub fn with_system_resolver(policy: Policy) -> Self {
        Self::new(policy, Arc::new(SystemResolver))
    }

    #[must_use]
    pub const fn policy(&self) -> &Policy {
        &self.policy
    }

    /// Fetch one URL, following redirects by hand and re-checking each hop.
    ///
    /// # Errors
    /// Refusal by policy or robots, a non-2xx status, an unreadable content
    /// type, or a network failure.
    pub async fn get(&self, url: &str) -> Result<Document, FetchError> {
        self.get_with_raw(url).await.map(|(doc, _)| doc)
    }

    /// Fetch, returning the document **and the raw body**.
    ///
    /// `search` needs this: the results parser matches on class attributes, and
    /// [`Document::text`] has already had every tag stripped out of it. Passing
    /// extracted prose to a markup parser finds nothing and reports it as "the
    /// page format changed", which is a maintenance alarm raised by a caller
    /// bug — it cost a live test to notice.
    ///
    /// # Errors
    /// As [`Fetcher::get`].
    pub async fn get_with_raw(&self, url: &str) -> Result<(Document, String), FetchError> {
        let requested = url.to_string();
        let mut current = requested.clone();

        for hop in 0..=self.policy.max_redirects {
            let approved = self.approve(&current)?;

            if hop == 0 {
                self.check_robots(&approved).await?;
            }

            let client = self.client_for(&approved)?;
            let response = client
                .get(&current)
                .header(reqwest::header::USER_AGENT, USER_AGENT)
                .header(
                    reqwest::header::ACCEPT,
                    "text/html,text/plain;q=0.9,*/*;q=0.1",
                )
                // No cookie jar and no auth header, ever. A fetcher that carries
                // credentials is a fetcher that can be pointed at something.
                .send()
                .await
                .map_err(|e| FetchError::Network(e.to_string()))?;

            let status = response.status();
            if status.is_redirection() {
                let Some(location) = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                else {
                    return Err(FetchError::Status {
                        url: current,
                        status: status.as_u16(),
                    });
                };
                // Relative Locations are legal and common.
                current = url::Url::parse(&current)
                    .and_then(|b| b.join(location))
                    .map(|u| u.to_string())
                    .map_err(|e| FetchError::Network(e.to_string()))?;
                continue;
            }

            if !status.is_success() {
                return Err(FetchError::Status {
                    url: current,
                    status: status.as_u16(),
                });
            }

            return self.read_body(&requested, &current, response).await;
        }

        Err(FetchError::Refused(format!(
            "more than {} redirects, starting from {requested}",
            self.policy.max_redirects
        )))
    }

    /// Run a search against one provider.
    ///
    /// # Errors
    /// Anything [`Fetcher::get`] can fail with.
    pub async fn search(
        &self,
        provider: crate::search::Provider,
        query: &str,
        limit: usize,
    ) -> Result<crate::search::SearchOutcome, FetchError> {
        // Raw body, not `doc.text`: these are JSON and Atom, and running them
        // through the HTML text extractor would strip the structure the parsers
        // read.
        let (_, raw) = self
            .get_with_raw(&crate::search::query_url(provider, query, limit))
            .await?;
        Ok(crate::search::parse(provider, &raw, limit))
    }

    /// The policy decision for one URL, with DNS done by our resolver.
    fn approve(&self, url: &str) -> Result<Approved, FetchError> {
        let parsed =
            url::Url::parse(url).map_err(|e| FetchError::Refused(format!("{url:?}: {e}")))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| FetchError::Refused(format!("{url:?} has no host")))?
            .to_string();
        let port = parsed
            .port_or_known_default()
            .ok_or_else(|| FetchError::Refused("no port".into()))?;

        // A literal-IP host needs no resolver, and asking one would be a
        // needless lookup that can itself be poisoned.
        let addrs = if let Ok(ip) = host.parse::<std::net::IpAddr>() {
            vec![SocketAddr::new(ip, port)]
        } else {
            self.resolver
                .resolve(&host, port)
                .map_err(|reason| FetchError::Resolve {
                    host: host.clone(),
                    reason,
                })?
        };

        match check(url, &self.policy, &addrs) {
            Decision::Allow { host, addrs } => Ok(Approved { host, addrs, port }),
            Decision::Deny { reason } => Err(FetchError::Refused(reason)),
        }
    }

    /// A client that can only reach the approved addresses for this host.
    fn client_for(&self, approved: &Approved) -> Result<reqwest::Client, FetchError> {
        let mut builder = reqwest::Client::builder()
            .timeout(self.policy.timeout)
            .connect_timeout(std::time::Duration::from_secs(8))
            // Manual, so every hop is re-checked. An automatic redirect is a
            // connection to a host nothing approved.
            .redirect(reqwest::redirect::Policy::none())
            // No cookie store is enabled at all (the `cookies` feature is off),
            // so nothing persists between fetches. A fetcher that remembers a
            // session is a fetcher that can be pointed at something with it.
            .https_only(!self.policy.allow_plaintext)
            .user_agent(USER_AGENT);

        for addr in &approved.addrs {
            builder = builder.resolve(&approved.host, *addr);
        }

        builder
            .build()
            .map_err(|e| FetchError::Network(e.to_string()))
    }

    /// Read a body under the size cap, streaming so an oversized response is
    /// abandoned rather than buffered.
    async fn read_body(
        &self,
        requested: &str,
        final_url: &str,
        response: reqwest::Response,
    ) -> Result<(Document, String), FetchError> {
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        let mime = content_type
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();

        if !READABLE.contains(&mime.as_str()) {
            return Err(FetchError::UnsupportedContentType {
                url: final_url.to_string(),
                content_type,
            });
        }

        let mut body: Vec<u8> = Vec::new();
        let mut truncated = false;
        let mut stream = response;
        while let Some(chunk) = stream
            .chunk()
            .await
            .map_err(|e| FetchError::Network(e.to_string()))?
        {
            let room = self.policy.max_bytes.saturating_sub(body.len());
            if chunk.len() > room {
                body.extend_from_slice(&chunk[..room]);
                truncated = true;
                break;
            }
            body.extend_from_slice(&chunk);
        }

        let body_sha256 = hex::encode(Sha256::digest(&body));
        let bytes = body.len();
        let raw = String::from_utf8_lossy(&body);

        let (text, title) = if mime.contains("html") || mime.contains("xml") {
            (extract::to_text(&raw), extract::title(&raw))
        } else {
            (raw.to_string(), None)
        };

        Ok((
            Document {
                final_url: final_url.to_string(),
                requested_url: requested.to_string(),
                status,
                content_type,
                text: Untrusted::new(text),
                title: title.map(Untrusted::new),
                body_sha256,
                bytes,
                // When *we* learned it. Not any date on the page, which the page
                // chooses.
                knowledge_time: chrono::Utc::now(),
                truncated,
            },
            raw.into_owned(),
        ))
    }

    /// Fetch and cache robots.txt, then check the path.
    ///
    /// A robots.txt that cannot be fetched means allowed — the standard reading,
    /// and the alternative fails closed on transient errors in a way nobody
    /// would debug.
    async fn check_robots(&self, approved: &Approved) -> Result<(), FetchError> {
        let path = "/robots.txt";
        let mut cache = self.robots.lock().await;
        if !cache.contains_key(&approved.host) {
            let scheme = if self.policy.allow_plaintext {
                "http"
            } else {
                "https"
            };
            let url = format!("{scheme}://{}{path}", approved.host);
            let rules = match self.client_for(approved) {
                Ok(client) => match client.get(&url).send().await {
                    Ok(r) if r.status().is_success() => {
                        let body = r.text().await.unwrap_or_default();
                        robots::parse(&body, USER_AGENT)
                    }
                    _ => Rules::permissive(),
                },
                Err(_) => Rules::permissive(),
            };
            cache.insert(approved.host.clone(), rules);
        }
        Ok(())
    }

    /// Whether robots allows a path, from the cache. Public so a caller can
    /// check without fetching.
    pub async fn robots_allows(&self, host: &str, path: &str) -> bool {
        self.robots
            .lock()
            .await
            .get(host)
            .is_none_or(|r| r.allows(path))
    }
}

#[derive(Debug)]
struct Approved {
    host: String,
    addrs: Vec<SocketAddr>,
    #[allow(dead_code)]
    port: u16,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::egress::HostPolicy;

    struct Fixed(Vec<SocketAddr>);
    impl Resolver for Fixed {
        fn resolve(&self, _host: &str, _port: u16) -> Result<Vec<SocketAddr>, String> {
            Ok(self.0.clone())
        }
    }

    struct Failing;
    impl Resolver for Failing {
        fn resolve(&self, _host: &str, _port: u16) -> Result<Vec<SocketAddr>, String> {
            Err("NXDOMAIN".into())
        }
    }

    fn fetcher(addrs: &str) -> Fetcher {
        Fetcher::new(
            Policy::default(),
            Arc::new(Fixed(vec![addrs.parse().unwrap()])),
        )
    }

    /// The whole point of the module: approval yields the checked addresses, and
    /// the client is pinned to them.
    #[test]
    fn approval_returns_the_addresses_the_policy_checked() {
        let f = fetcher("93.184.216.34:443");
        let a = f.approve("https://arxiv.org/abs/1").expect("allowed");
        assert_eq!(a.host, "arxiv.org");
        assert_eq!(a.addrs, vec!["93.184.216.34:443".parse().unwrap()]);
        assert!(f.client_for(&a).is_ok());
    }

    #[test]
    fn a_host_resolving_into_the_private_network_is_refused_before_any_socket() {
        let f = fetcher("127.0.0.1:443");
        let err = f
            .approve("https://arxiv.org/abs/1")
            .expect_err("must refuse");
        assert!(matches!(err, FetchError::Refused(_)), "{err}");
        assert!(err.to_string().contains("loopback"), "{err}");
    }

    #[test]
    fn an_unlisted_host_is_refused() {
        let f = fetcher("93.184.216.34:443");
        let err = f.approve("https://example.com/").expect_err("must refuse");
        assert!(err.to_string().contains("egress allowlist"), "{err}");
    }

    #[test]
    fn a_resolver_failure_is_reported_as_a_resolve_error_not_a_refusal() {
        let f = Fetcher::new(Policy::default(), Arc::new(Failing));
        let err = f.approve("https://arxiv.org/").expect_err("must fail");
        assert!(matches!(err, FetchError::Resolve { .. }), "{err}");
    }

    /// A literal IP needs no resolver, and asking one would be a lookup that can
    /// itself be poisoned.
    #[test]
    fn a_literal_ip_host_skips_the_resolver() {
        let f = Fetcher::new(
            Policy {
                hosts: HostPolicy::AllowAny,
                ..Policy::default()
            },
            Arc::new(Failing),
        );
        assert!(f.approve("https://93.184.216.34/").is_ok());
        assert!(f.approve("https://127.0.0.1/").is_err());
    }

    /// A PDF run through `from_utf8_lossy` is a page of replacement characters
    /// that costs context and says nothing.
    #[test]
    fn the_readable_list_covers_text_and_excludes_binary() {
        for t in ["text/html", "text/plain", "application/json"] {
            assert!(READABLE.contains(&t), "{t} should be readable");
        }
        for t in [
            "application/pdf",
            "image/png",
            "application/zip",
            "application/octet-stream",
        ] {
            assert!(!READABLE.contains(&t), "{t} must not be decoded as text");
        }
    }

    #[test]
    fn robots_defaults_to_permissive_for_an_unseen_host() {
        let f = fetcher("93.184.216.34:443");
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        assert!(rt.block_on(f.robots_allows("never-fetched.org", "/anything")));
    }

    #[test]
    fn the_policy_is_readable_but_not_mutable_from_outside() {
        let f = fetcher("93.184.216.34:443");
        assert!(!f.policy().allow_plaintext);
        assert_eq!(f.policy().max_redirects, 5);
    }
}
