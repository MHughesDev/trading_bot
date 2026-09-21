//! Which feature families exist, and how a call becomes a feature name.
//!
//! # The naming rule is the whole of the mapping
//!
//! `crates/features/src/registry.rs` states it: a feature is `family` for a
//! nullary family (`close`, `obv`, `hour_sin`) or `family_p1[_p2…]` with
//! **integer** parameters in declaration order (`ema_7`, `macd_12_26`,
//! `bb_upper_20_2`). SLv2's `ema(close, 7)` is sugar for the name `ema_7`, and
//! that is the only thing the compiler does to a feature call. Everything a
//! model can say about features it can say by naming one, which is what makes
//! the language grammar-constrainable (AGENT-011 P7).
//!
//! # There is one list of families, and it is not here
//!
//! The authority on which families exist is `features::registry`, and
//! [`Catalogue::engine`] reads it. An earlier version of this file carried its
//! own transcription of that registry — sixty-one names with their parameter
//! ranges — and it was removed as a defect: a language that disagreed with the
//! engine about what a feature is called would refuse valid strategies and
//! accept invalid ones, and nothing would compare the two lists until a model
//! hit the difference. Do not reintroduce one.
//! `the_catalogue_is_the_engine_s_registry_and_holds_nothing_else` compares
//! the two sets name by name and parameter by parameter.
//!
//! The catalogue is still **injectable** — [`Catalogue::from_families`] takes
//! any source and the compiler takes a `&Catalogue` — so a test can compile
//! against a smaller world without a second registry existing in the build.
//!
//! What this type deliberately does **not** carry is lookback, PIT lag, cost
//! class or `use_when`. Those are the parts `list_features` serves; a second
//! copy here would be a second answer to a question the registry already
//! answers. Names and arities are what the *compiler* needs to refuse `rsi14`
//! and `ema(close, 7, 9)`, and nothing more.

/// One integer parameter of a family, with the range outside which the name
/// does not resolve.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FamilyParam {
    pub name: String,
    pub min: i64,
    pub max: i64,
}

/// One feature family.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Family {
    pub name: String,
    pub params: Vec<FamilyParam>,
}

impl Family {
    /// The name this family takes for `params`, e.g. `("ema", [7]) -> "ema_7"`.
    #[must_use]
    pub fn name_for(&self, params: &[String]) -> String {
        if params.is_empty() {
            return self.name.clone();
        }
        let mut s = self.name.clone();
        for p in params {
            s.push('_');
            s.push_str(p);
        }
        s
    }
}

/// The families a compiler will accept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Catalogue {
    families: Vec<Family>,
}

impl Catalogue {
    /// Build from any source — including `features::registry::families()`,
    /// which is where this should come from.
    #[must_use]
    pub fn from_families(families: Vec<Family>) -> Self {
        let mut families = families;
        // Longest name first, so `resolve` can take the first prefix that
        // matches and be right: `macd_signal_12_26_9` must not resolve as
        // `macd` with a parameter called `signal`.
        families.sort_by(|a, b| b.name.len().cmp(&a.name.len()).then(a.name.cmp(&b.name)));
        Self { families }
    }

    /// The catalogue the engine actually serves.
    ///
    /// Read from `features::registry::families()` — the one registry the
    /// simulator resolves through (INV-14) — rather than transcribed. An earlier
    /// version of this file carried its own list of sixty-one families with
    /// their parameter names and ranges, which is the two-implementations shape
    /// this repository keeps finding: a language that disagreed with the engine
    /// about what a feature is called would refuse valid strategies and accept
    /// invalid ones, and nothing would compare the two lists until a model hit
    /// the difference.
    ///
    /// Aliases are dropped. `features::registry::resolve` rewrites an alias to
    /// its canonical family before anything is constructed, so admitting both
    /// here would let two spellings compile to the same feature and the
    /// decompiler would have to guess which the author wrote.
    #[must_use]
    pub fn engine() -> Self {
        Self::from_families(
            features::registry::families()
                .iter()
                .filter(|f| f.alias_of.is_none())
                .map(|f| Family {
                    name: f.name.to_string(),
                    params: f
                        .params
                        .iter()
                        .map(|p| FamilyParam {
                            name: p.name.to_string(),
                            min: i64::try_from(p.min).unwrap_or(i64::MAX),
                            max: i64::try_from(p.max).unwrap_or(i64::MAX),
                        })
                        .collect(),
                })
                .collect(),
        )
    }

    /// Look a family up by name.
    #[must_use]
    pub fn family(&self, name: &str) -> Option<&Family> {
        self.families.iter().find(|f| f.name == name)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.families.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.families.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Family> {
        self.families.iter()
    }

    /// Split a feature name into its family and its parameters.
    ///
    /// Returns `None` when no family's name is a prefix, or when what follows
    /// the prefix is not the right number of integers. That second condition is
    /// what makes `ema_7_9` unresolvable rather than silently an `ema`.
    #[must_use]
    pub fn resolve(&self, name: &str) -> Option<(&Family, Vec<String>)> {
        for family in &self.families {
            if name == family.name && family.params.is_empty() {
                return Some((family, Vec::new()));
            }
            let Some(rest) = name.strip_prefix(&family.name) else {
                continue;
            };
            let Some(rest) = rest.strip_prefix('_') else {
                continue;
            };
            if rest.is_empty() {
                continue;
            }
            let Some(parts) = split_params(rest) else {
                continue;
            };
            if parts.len() != family.params.len() {
                continue;
            }
            return Some((family, parts));
        }
        None
    }

    /// The closest family name to `name`, for a "did you mean" fix.
    ///
    /// Compared against family names rather than example feature names, for the
    /// reason S-5.3 recorded: `rsi`'s advertised example is `rsi_20`, so an
    /// edit-distance search over examples offers nothing for `rsi14` while a
    /// search over family names offers `rsi`.
    #[must_use]
    pub fn nearest(&self, name: &str) -> Option<&str> {
        let stem: String = name
            .chars()
            .take_while(|c| c.is_ascii_alphabetic() || *c == '_')
            .collect();
        let probe = if stem.is_empty() { name } else { &stem };
        self.families
            .iter()
            .map(|f| (edit_distance(probe, &f.name), f.name.as_str()))
            .filter(|(d, n)| *d <= 2 || n.starts_with(probe) && !probe.is_empty())
            .min_by_key(|(d, n)| (*d, n.len()))
            .map(|(_, n)| n)
    }
}

/// Split the parameter part of a feature name.
///
/// A part is either an integer or a v1.2 template hole (`{{fast}}`). The hole is
/// taken atomically rather than split on `_`, so a parameter called `my_param`
/// survives: `ema_{{my_param}}` is one parameter, not two.
fn split_params(rest: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let bytes = rest.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if rest[i..].starts_with("{{") {
            let end = rest[i..].find("}}")? + i + 2;
            let name = &rest[i + 2..end - 2];
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return None;
            }
            out.push(rest[i..end].to_string());
            i = end;
        } else {
            let end = rest[i..].find('_').map_or(rest.len(), |o| o + i);
            let part = &rest[i..end];
            if part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            out.push(part.to_string());
            i = end;
        }
        if i < bytes.len() {
            if bytes[i] != b'_' {
                return None;
            }
            i += 1;
            if i >= bytes.len() {
                return None;
            }
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Levenshtein distance, two rows. Small inputs; no allocation worth avoiding.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    /// The catalogue is the engine's, not a copy of it.
    ///
    /// The check that would have caught the transcription this function
    /// replaced: every non-alias family the engine serves resolves here, and
    /// this one holds nothing the engine does not. A list that drifted by one
    /// name would refuse a valid strategy or accept an invalid one, and nothing
    /// else compares them.
    #[test]
    fn the_catalogue_is_the_engine_s_registry_and_holds_nothing_else() {
        let c = super::Catalogue::engine();
        let engine: std::collections::BTreeSet<&str> = features::registry::families()
            .iter()
            .filter(|f| f.alias_of.is_none())
            .map(|f| f.name)
            .collect();
        assert!(
            engine.len() > 40,
            "the engine registry came back with {} families, which is not the registry — this              comparison has stopped comparing anything",
            engine.len()
        );
        let mine: std::collections::BTreeSet<&str> = c.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            mine, engine,
            "the language and the engine disagree about which feature families exist"
        );
        // And the parameter names travel with them, because a family whose
        // parameters are wrong compiles to a feature name the engine cannot
        // resolve.
        for f in features::registry::families()
            .iter()
            .filter(|f| f.alias_of.is_none())
        {
            let here = c.family(f.name).expect("every engine family resolves");
            let theirs: Vec<&str> = f.params.iter().map(|p| p.name).collect();
            let ours: Vec<&str> = here.params.iter().map(|p| p.name.as_str()).collect();
            assert_eq!(ours, theirs, "parameter names differ for `{}`", f.name);
        }
    }

    use super::*;

    /// A check over the catalogue that would pass on an empty one is a check
    /// that passes when the registry stops being read.
    #[test]
    fn the_catalogue_is_not_empty_and_covers_the_skeletons() {
        let c = Catalogue::engine();
        assert!(c.len() >= 55, "the scan found {} families", c.len());
        for needed in [
            "ema",
            "rsi",
            "atr",
            "zscore",
            "rolling_std",
            "donchian_upper",
            "rv",
            "macd",
            "pct_rank",
            "obv",
            "hour_sin",
        ] {
            assert!(
                c.family(needed).is_some(),
                "{needed} is used by a hypothesis skeleton and is not in the catalogue"
            );
        }
    }

    /// The longest-prefix rule. Without it `macd_signal_12_26_9` resolves as
    /// `macd` with three parameters and the arity check passes on a name that
    /// means something else.
    #[test]
    fn resolve_takes_the_longest_family_prefix() {
        let c = Catalogue::engine();
        let (f, p) = c.resolve("macd_signal_12_26_9").expect("resolves");
        assert_eq!(f.name, "macd_signal");
        assert_eq!(p, vec!["12", "26", "9"]);

        let (f, p) = c.resolve("macd_12_26").expect("resolves");
        assert_eq!(f.name, "macd");
        assert_eq!(p, vec!["12", "26"]);
    }

    #[test]
    fn a_wrong_parameter_count_does_not_resolve() {
        let c = Catalogue::engine();
        assert!(c.resolve("ema_7").is_some());
        assert!(c.resolve("ema_7_9").is_none());
        assert!(c.resolve("ema").is_none(), "ema takes one parameter");
        assert!(c.resolve("obv").is_some(), "obv takes none");
        assert!(c.resolve("obv_7").is_none());
        assert!(
            c.resolve("rsi14").is_none(),
            "the separator is not optional"
        );
    }

    #[test]
    fn nearest_suggests_the_family_behind_a_mistyped_name() {
        let c = Catalogue::engine();
        assert_eq!(c.nearest("rsi14"), Some("rsi"));
        assert_eq!(c.nearest("emma"), Some("ema"));
        assert_eq!(c.nearest("zscore"), Some("zscore"));
        assert_eq!(c.nearest("qqqqqqqqqqqqqq"), None);
    }

    /// v1.2 interpolation survives resolution, so a decompiler can turn
    /// `ema_{{fast}}` back into `ema(close, fast)` rather than giving up.
    #[test]
    fn a_template_hole_is_one_parameter_however_many_underscores_it_has() {
        let c = Catalogue::engine();
        let (f, p) = c.resolve("ema_{{fast}}").expect("resolves");
        assert_eq!(f.name, "ema");
        assert_eq!(p, vec!["{{fast}}"]);

        let (f, p) = c.resolve("bb_upper_{{my_window}}_2").expect("resolves");
        assert_eq!(f.name, "bb_upper");
        assert_eq!(p, vec!["{{my_window}}", "2"]);

        assert!(c.resolve("ema_{{}}").is_none());
        assert!(c.resolve("ema_{{fast").is_none());
    }

    #[test]
    fn a_name_is_built_back_from_its_parts() {
        let c = Catalogue::engine();
        let f = c.family("bb_upper").expect("registered");
        assert_eq!(f.name_for(&["20".into(), "2".into()]), "bb_upper_20_2");
        let f = c.family("obv").expect("registered");
        assert_eq!(f.name_for(&[]), "obv");
    }

    /// Every family in the table can rebuild a name that resolves back to it,
    /// which is the property `compile` and `decompile` both rely on.
    #[test]
    fn every_family_round_trips_through_its_own_name() {
        let c = Catalogue::engine();
        let mut checked = 0;
        for f in c.iter() {
            let params: Vec<String> = f
                .params
                .iter()
                .map(|p| p.min.max(2).min(p.max).to_string())
                .collect();
            let name = f.name_for(&params);
            let (back, got) = c
                .resolve(&name)
                .unwrap_or_else(|| panic!("{name} does not resolve"));
            assert_eq!(back.name, f.name, "{name} resolved to the wrong family");
            assert_eq!(got, params);
            checked += 1;
        }
        assert_eq!(checked, c.len());
        assert!(checked >= 55, "the scan found {checked} families");
    }
}
