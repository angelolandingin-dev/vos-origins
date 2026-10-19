#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonKind {
    Identical,
    Compatible,
    Incompatible,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct RangeComparison {
    pub kind: ComparisonKind,
    pub reason: Option<String>,
}

type Version = [u32; 3];

#[derive(Debug, Clone, Copy)]
struct Interval {
    lo: Option<Version>,
    lo_inclusive: bool,
    hi: Option<Version>,
    hi_inclusive: bool,
}

fn unbounded() -> Interval {
    Interval {
        lo: None,
        lo_inclusive: false,
        hi: None,
        hi_inclusive: false,
    }
}

fn parse_version(part: &str) -> Option<(Vec<u32>, Option<usize>)> {
    let stripped = part.trim_start_matches(['v', '=']);
    let core_raw = stripped.split('+').next().unwrap_or("");
    let core = core_raw.split('-').next().unwrap_or("");
    let bits: Vec<&str> = core.split('.').collect();
    let mut nums = Vec::new();
    for i in 0..3 {
        match bits.get(i) {
            None => return Some((nums, Some(i))),
            Some(token) if is_wildcard(token) => return Some((nums, Some(i))),
            Some(token) => {
                if token.is_empty() || !token.chars().all(|c| c.is_ascii_digit()) {
                    return None;
                }
                nums.push(token.parse::<u32>().unwrap_or(0));
            }
        }
    }
    Some((nums, None))
}

fn is_wildcard(token: &str) -> bool {
    matches!(token, "x" | "X" | "*")
}

fn pad(nums: &[u32]) -> Version {
    [
        nums.first().copied().unwrap_or(0),
        nums.get(1).copied().unwrap_or(0),
        nums.get(2).copied().unwrap_or(0),
    ]
}

fn cmp_version(a: &Version, b: &Version) -> std::cmp::Ordering {
    a.cmp(b)
}

fn intersect(a: Interval, b: Interval) -> Option<Interval> {
    let (lo, lo_inclusive) = match (a.lo, b.lo) {
        (None, lo) => (lo, b.lo_inclusive),
        (lo, None) => (lo, a.lo_inclusive),
        (Some(al), Some(bl)) => match cmp_version(&al, &bl) {
            std::cmp::Ordering::Greater => (Some(al), a.lo_inclusive),
            std::cmp::Ordering::Less => (Some(bl), b.lo_inclusive),
            std::cmp::Ordering::Equal => (Some(al), a.lo_inclusive && b.lo_inclusive),
        },
    };
    let (hi, hi_inclusive) = match (a.hi, b.hi) {
        (None, hi) => (hi, b.hi_inclusive),
        (hi, None) => (hi, a.hi_inclusive),
        (Some(ah), Some(bh)) => match cmp_version(&ah, &bh) {
            std::cmp::Ordering::Less => (Some(ah), a.hi_inclusive),
            std::cmp::Ordering::Greater => (Some(bh), b.hi_inclusive),
            std::cmp::Ordering::Equal => (Some(ah), a.hi_inclusive && b.hi_inclusive),
        },
    };
    if let (Some(lo), Some(hi)) = (lo, hi) {
        match cmp_version(&lo, &hi) {
            std::cmp::Ordering::Greater => return None,
            std::cmp::Ordering::Equal if !(lo_inclusive && hi_inclusive) => return None,
            _ => {}
        }
    }
    Some(Interval {
        lo,
        lo_inclusive,
        hi,
        hi_inclusive,
    })
}

fn comparator_interval(op: &str, version: &str) -> Option<Interval> {
    let (nums, wildcard_from) = parse_version(version)?;

    if matches!(op, ">=" | ">" | "<=" | "<") {
        if wildcard_from.is_some() && nums.is_empty() {
            return Some(unbounded());
        }
        let v = pad(&nums);
        return Some(match op {
            ">=" => Interval {
                lo: Some(v),
                lo_inclusive: true,
                hi: None,
                hi_inclusive: false,
            },
            ">" => Interval {
                lo: Some(v),
                lo_inclusive: false,
                hi: None,
                hi_inclusive: false,
            },
            "<=" => Interval {
                lo: None,
                lo_inclusive: false,
                hi: Some(v),
                hi_inclusive: true,
            },
            _ => Interval {
                lo: None,
                lo_inclusive: false,
                hi: Some(v),
                hi_inclusive: false,
            },
        });
    }

    if let Some(wf) = wildcard_from {
        let major = nums.first().copied().unwrap_or(0);
        if wf == 0 {
            return Some(unbounded());
        }
        if wf == 1 {
            return Some(Interval {
                lo: Some([major, 0, 0]),
                lo_inclusive: true,
                hi: Some([major + 1, 0, 0]),
                hi_inclusive: false,
            });
        }
        let minor = nums.get(1).copied().unwrap_or(0);
        return Some(Interval {
            lo: Some([major, minor, 0]),
            lo_inclusive: true,
            hi: Some([major, minor + 1, 0]),
            hi_inclusive: false,
        });
    }

    let v = pad(&nums);
    match op {
        "^" => {
            let hi = if v[0] != 0 {
                [v[0] + 1, 0, 0]
            } else if v[1] != 0 {
                [0, v[1] + 1, 0]
            } else {
                [0, 0, v[2] + 1]
            };
            Some(Interval {
                lo: Some(v),
                lo_inclusive: true,
                hi: Some(hi),
                hi_inclusive: false,
            })
        }
        "~" => {
            let hi = if nums.len() >= 2 {
                [v[0], v[1] + 1, 0]
            } else {
                [v[0] + 1, 0, 0]
            };
            Some(Interval {
                lo: Some(v),
                lo_inclusive: true,
                hi: Some(hi),
                hi_inclusive: false,
            })
        }
        _ => {
            // Bare version / "=": partial versions act as ranges.
            match nums.len() {
                3 => Some(Interval {
                    lo: Some(v),
                    lo_inclusive: true,
                    hi: Some(v),
                    hi_inclusive: true,
                }),
                2 => Some(Interval {
                    lo: Some(v),
                    lo_inclusive: true,
                    hi: Some([v[0], v[1] + 1, 0]),
                    hi_inclusive: false,
                }),
                _ => Some(Interval {
                    lo: Some(v),
                    lo_inclusive: true,
                    hi: Some([v[0] + 1, 0, 0]),
                    hi_inclusive: false,
                }),
            }
        }
    }
}

enum ComparatorSet {
    Ok(Interval),
    Empty,
    Bad,
}

fn parse_comparator_set(set: &str) -> ComparatorSet {
    let tokens: Vec<&str> = set.split_whitespace().filter(|t| !t.is_empty()).collect();
    if tokens.is_empty() {
        return ComparatorSet::Ok(unbounded());
    }
    let mut acc = unbounded();
    for token in tokens {
        let (op, version) = split_operator(token);
        if is_wildcard(version) {
            continue;
        }
        let Some(iv) = comparator_interval(op, version) else {
            return ComparatorSet::Bad;
        };
        match intersect(acc, iv) {
            Some(next) => acc = next,
            None => return ComparatorSet::Empty,
        }
    }
    ComparatorSet::Ok(acc)
}

fn split_operator(token: &str) -> (&str, &str) {
    for op in [">=", "<=", ">", "<", "^", "~", "="] {
        if let Some(rest) = token.strip_prefix(op) {
            return (op, rest);
        }
    }
    ("", token)
}

enum Norm {
    Any,
    Unparseable,
    Empty,
    NonSemver(String),
    Alias(String),
    Tag(String),
    Range(Vec<Interval>),
}

fn is_tag(spec: &str) -> bool {
    let mut chars = spec.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn normalize_spec(raw: &str) -> Norm {
    let spec = raw.trim();
    if spec.is_empty() {
        return Norm::Any;
    }
    if let Some(rest) = spec.strip_prefix("npm:") {
        if let Some(at) = spec.rfind('@') {
            if at > 4 {
                return normalize_spec(&spec[at + 1..]);
            }
        }
        let _ = rest;
        return Norm::Alias(spec.to_string());
    }
    if spec.starts_with("link:")
        || spec.starts_with("file:")
        || spec.starts_with("portal:")
        || spec.starts_with("workspace:")
        || spec.starts_with("http:")
        || spec.starts_with("https:")
        || spec.starts_with("git+")
        || spec.starts_with("git:")
        || spec.starts_with("github:")
    {
        return Norm::NonSemver(spec.to_string());
    }
    if is_tag(spec) && !is_wildcard(spec) {
        return Norm::Tag(spec.to_string());
    }
    let mut intervals = Vec::new();
    for set in spec.split("||") {
        match parse_comparator_set(set) {
            ComparatorSet::Bad => return Norm::Unparseable,
            ComparatorSet::Empty => continue,
            ComparatorSet::Ok(iv) => intervals.push(iv),
        }
    }
    if intervals.is_empty() {
        return Norm::Empty;
    }
    Norm::Range(intervals)
}

pub fn compare_ranges(a: &str, b: &str) -> RangeComparison {
    let mk = |kind: ComparisonKind, reason: Option<&str>| RangeComparison {
        kind,
        reason: reason.map(str::to_string),
    };
    if a == b {
        return mk(ComparisonKind::Identical, None);
    }
    let na = normalize_spec(a);
    let nb = normalize_spec(b);
    let raw_of = |n: &Norm| match n {
        Norm::NonSemver(r) | Norm::Alias(r) | Norm::Tag(r) => Some(r.clone()),
        _ => None,
    };
    if matches!(na, Norm::Any) || matches!(nb, Norm::Any) {
        return mk(ComparisonKind::Compatible, Some("wildcard"));
    }
    if matches!(na, Norm::Unparseable) || matches!(nb, Norm::Unparseable) {
        return mk(ComparisonKind::Unknown, Some("unparseable range"));
    }
    let nonsemver = |n: &Norm| matches!(n, Norm::NonSemver(_) | Norm::Alias(_));
    if nonsemver(&na) || nonsemver(&nb) {
        return if raw_of(&na) == raw_of(&nb) {
            mk(ComparisonKind::Identical, None)
        } else {
            mk(ComparisonKind::Unknown, Some("non-semver spec differs"))
        };
    }
    if matches!(na, Norm::Tag(_)) || matches!(nb, Norm::Tag(_)) {
        return if raw_of(&na) == raw_of(&nb) {
            mk(ComparisonKind::Identical, None)
        } else {
            mk(ComparisonKind::Unknown, Some("dist-tag differs"))
        };
    }
    let (Norm::Range(ia), Norm::Range(ib)) = (&na, &nb) else {
        return mk(ComparisonKind::Incompatible, Some("empty range"));
    };
    for x in ia {
        for y in ib {
            if intersect(*x, *y).is_some() {
                return mk(ComparisonKind::Compatible, None);
            }
        }
    }
    mk(ComparisonKind::Incompatible, None)
}

pub fn is_compatible(a: &str, b: &str) -> bool {
    matches!(
        compare_ranges(a, b).kind,
        ComparisonKind::Identical | ComparisonKind::Compatible
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compatibility_decisions() {
        assert_eq!(
            compare_ranges("^1.0.0", "^1.0.0").kind,
            ComparisonKind::Identical
        );
        assert_eq!(
            compare_ranges("^1.0.0", "^1.5.0").kind,
            ComparisonKind::Compatible
        );
        assert_eq!(
            compare_ranges("^1.0.0", "^2.0.0").kind,
            ComparisonKind::Incompatible
        );
        assert_eq!(
            compare_ranges("^0.2.0", "^0.2.5").kind,
            ComparisonKind::Compatible
        );
        assert_eq!(
            compare_ranges("^0.2.0", "^0.3.0").kind,
            ComparisonKind::Incompatible
        );
        assert_eq!(
            compare_ranges(">=2 <3", "^2.5.0").kind,
            ComparisonKind::Compatible
        );
        assert_eq!(
            compare_ranges("workspace:*", "^1.0.0").kind,
            ComparisonKind::Unknown
        );
        assert_eq!(
            compare_ranges("latest", "next").kind,
            ComparisonKind::Unknown
        );
        assert_eq!(
            compare_ranges("1.2.3", "1.2.3").kind,
            ComparisonKind::Identical
        );
        assert_eq!(
            compare_ranges("*", "^9.9.9").kind,
            ComparisonKind::Compatible
        );
    }
}
