//! Title scoring for unattended auto-match (adapted from KoiPage metadata/matching.rs — ideas only).

use super::MetadataCandidate;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Match {
    None,
    Weak,
    Strong,
    Exact,
}

impl Match {
    pub fn is_confident(self) -> bool {
        matches!(self, Match::Exact | Match::Strong)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Match::None => "none",
            Match::Weak => "weak",
            Match::Strong => "strong",
            Match::Exact => "exact",
        }
    }
}

const NOISE_SUFFIXES: &[&str] = &[
    "全年龄",
    "汉化",
    "漢化",
    "韩漫",
    "韩文",
    "正版",
    "中文",
    "简体",
    "繁体",
    "官中",
    "汉化版",
    "汉化组",
    "嵌字",
    "扫图",
    "裁剪",
    "无水",
    "无水印",
    "完结",
    "连载中",
    "完结版",
    "单行本",
    "汉化补丁",
];

const MAX_CONTAINMENT_SLACK: usize = 8;
const MIN_STRONG_LEN: usize = 3;

fn strip_version(text: &str) -> &str {
    let bytes = text.as_bytes();
    let mut end = bytes.len();
    while end > 0 && bytes[end - 1].is_ascii_digit() {
        end -= 1;
    }
    if end > 0 && end < bytes.len() && bytes[end - 1] == b'v' {
        return &text[..end - 1];
    }
    text
}

pub fn normalise(title: &str) -> String {
    let lowered = title.to_lowercase();
    let kept: String = lowered.chars().filter(|ch| ch.is_alphanumeric()).collect();
    let mut out = strip_version(&kept).to_string();
    for noise in NOISE_SUFFIXES {
        if let Some(stripped) = out.strip_suffix(&noise.to_lowercase()) {
            out = stripped.to_string();
        }
    }
    out
}

pub fn score(query: &str, candidate: &MetadataCandidate) -> Match {
    let needle = normalise(query);
    if needle.is_empty() {
        return Match::None;
    }
    let mut best = Match::None;
    for haystack in [Some(candidate.title.as_str()), candidate.original_title.as_deref()]
        .into_iter()
        .chain(candidate.aliases.iter().map(|a| Some(a.as_str())))
        .flatten()
    {
        best = best.max(compare(&needle, &normalise(haystack)));
    }
    best
}

fn compare(needle: &str, haystack: &str) -> Match {
    if haystack.is_empty() {
        return Match::None;
    }
    if needle == haystack {
        return Match::Exact;
    }
    let (longer, shorter) = if needle.len() >= haystack.len() {
        (needle, haystack)
    } else {
        (haystack, needle)
    };
    if !shorter.is_empty() && longer.starts_with(shorter) {
        let slack = longer.chars().count() - shorter.chars().count();
        if slack <= MAX_CONTAINMENT_SLACK && shorter.chars().count() >= MIN_STRONG_LEN {
            return Match::Strong;
        }
        return Match::Weak;
    }
    if shorter.len() >= MIN_STRONG_LEN && longer.contains(shorter) {
        return Match::Weak;
    }
    Match::None
}

pub fn best_for_query<'a>(query: &str, candidates: &'a [MetadataCandidate]) -> Option<(&'a MetadataCandidate, Match)> {
    candidates
        .iter()
        .map(|c| (c, score(query, c)))
        .filter(|(_, m)| *m != Match::None)
        .max_by_key(|(_, m)| *m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::Provider;

    fn cand(title: &str, aliases: &[&str]) -> MetadataCandidate {
        MetadataCandidate {
            provider: Provider::Bangumi,
            external_id: "1".into(),
            title: title.into(),
            original_title: None,
            aliases: aliases.iter().map(|s| s.to_string()).collect(),
            release_date: None,
            cover_url: None,
            rating: None,
            source_label: "Bangumi".into(),
            match_kind: None,
            page_url: None,
        }
    }

    #[test]
    fn strips_localisation_noise() {
        assert!(score("进击的巨人", &cand("进击的巨人", &[])).is_confident());
        assert_eq!(
            score(" solo leveling 汉化 ", &cand("Solo Leveling", &["나 혼자만 레벨업"])),
            Match::Strong
        );
    }
}
