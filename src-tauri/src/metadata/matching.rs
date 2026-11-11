//! Title scoring for unattended auto-match (adapted from KoiPage metadata/matching.rs — ideas only).

use std::collections::HashMap;

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

fn fold_latin(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ä' | 'ā' | 'ã' => 'a',
        'è' | 'é' | 'ê' | 'ë' | 'ē' => 'e',
        'ì' | 'í' | 'î' | 'ï' | 'ī' => 'i',
        'ò' | 'ó' | 'ô' | 'ö' | 'ō' | 'õ' => 'o',
        'ù' | 'ú' | 'û' | 'ü' | 'ū' => 'u',
        'ý' | 'ÿ' => 'y',
        'ñ' => 'n',
        'ç' => 'c',
        other => other,
    }
}

fn fold_han(c: char) -> char {
    match c {
        '國' => '国',
        '語' => '语',
        '門' => '门',
        '書' => '书',
        '長' => '长',
        '東' => '东',
        '車' => '车',
        '風' => '风',
        '龍' => '龙',
        '馬' => '马',
        '鳥' => '鸟',
        '魚' => '鱼',
        '頁' => '页',
        '見' => '见',
        '貝' => '贝',
        '紅' => '红',
        '綠' => '绿',
        '藍' => '蓝',
        '黃' => '黄',
        '學' => '学',
        '會' => '会',
        '體' => '体',
        '點' => '点',
        '對' => '对',
        '發' => '发',
        '經' => '经',
        '線' => '线',
        '網' => '网',
        '電' => '电',
        '話' => '话',
        '說' => '说',
        '記' => '记',
        '愛' => '爱',
        '戰' => '战',
        '義' => '义',
        '氣' => '气',
        '雲' => '云',
        '靈' => '灵',
        '劍' => '剑',
        '俠' => '侠',
        '傳' => '传',
        '錄' => '录',
        '畫' => '画',
        '圖' => '图',
        '館' => '馆',
        '場' => '场',
        '後' => '后',
        '從' => '从',
        '來' => '来',
        '個' => '个',
        '們' => '们',
        '這' => '这',
        '還' => '还',
        '過' => '过',
        '進' => '进',
        '遠' => '远',
        '連' => '连',
        '運' => '运',
        '邊' => '边',
        '開' => '开',
        '關' => '关',
        '問' => '问',
        '題' => '题',
        '無' => '无',
        '與' => '与',
        '為' => '为',
        '時' => '时',
        '間' => '间',
        '現' => '现',
        '實' => '实',
        '際' => '际',
        '機' => '机',
        '極' => '极',
        '強' => '强',
        '難' => '难',
        '雜' => '杂',
        '雙' => '双',
        '隻' => '只',
        '條' => '条',
        '萬' => '万',
        '億' => '亿',
        '歲' => '岁',
        '歷' => '历',
        '聖' => '圣',
        '師' => '师',
        '眾' => '众',
        '種' => '种',
        '裡' => '里',
        '麵' => '面',
        '擊' => '击',
        '鬥' => '斗',
        '廣' => '广',
        '慶' => '庆',
        '應' => '应',
        '樂' => '乐',
        '醫' => '医',
        '藥' => '药',
        '術' => '术',
        '號' => '号',
        '觀' => '观',
        '覺' => '觉',
        '讓' => '让',
        '認' => '认',
        '識' => '识',
        '議' => '议',
        '護' => '护',
        '變' => '变',
        '負' => '负',
        '質' => '质',
        '軍' => '军',
        '達' => '达',
        '選' => '选',
        '鐵' => '铁',
        '隊' => '队',
        '險' => '险',
        '隱' => '隐',
        '願' => '愿',
        '類' => '类',
        '飛' => '飞',
        '龜' => '龟',
        '髮' => '发',
        '節' => '节',
        '幹' => '干',
        other => other,
    }
}

fn fold_text(s: &str) -> String {
    s.chars().map(fold_han).collect()
}

pub fn normalise(title: &str) -> String {
    let lowered = title.to_lowercase();
    let kept: String = lowered
        .chars()
        .map(fold_latin)
        .filter(|ch| ch.is_alphanumeric())
        .collect();
    let mut out = strip_version(&kept).to_string();
    for noise in NOISE_SUFFIXES {
        if let Some(stripped) = out.strip_suffix(&noise.to_lowercase()) {
            out = stripped.to_string();
        }
    }
    fold_text(&out)
}

pub fn score(query: &str, candidate: &MetadataCandidate) -> Match {
    score_parts(query, candidate).0
}

/// Match kind plus a 0–100 confidence. Exact/strong stay confident.
pub fn score_parts(query: &str, candidate: &MetadataCandidate) -> (Match, u8) {
    let needle = normalise(query);
    if needle.is_empty() {
        return (Match::None, 0);
    }
    let mut best = (Match::None, 0u8);
    for haystack in [Some(candidate.title.as_str()), candidate.original_title.as_deref()]
        .into_iter()
        .chain(candidate.aliases.iter().map(|a| Some(a.as_str())))
        .flatten()
    {
        let cur = compare(&needle, &normalise(haystack));
        if cur.0 > best.0 || (cur.0 == best.0 && cur.1 > best.1) {
            best = cur;
        }
    }
    best
}

fn dice(a: &str, b: &str) -> f64 {
    let av: Vec<char> = a.chars().collect();
    let bv: Vec<char> = b.chars().collect();
    if av.len() < 2 || bv.len() < 2 {
        return 0.0;
    }
    let mut left: HashMap<(char, char), u16> = HashMap::new();
    for w in av.windows(2) {
        *left.entry((w[0], w[1])).or_default() += 1;
    }
    let mut right: HashMap<(char, char), u16> = HashMap::new();
    for w in bv.windows(2) {
        *right.entry((w[0], w[1])).or_default() += 1;
    }
    let mut inter = 0u32;
    for (k, v) in &left {
        if let Some(w) = right.get(k) {
            inter += u32::from((*v).min(*w));
        }
    }
    (2.0 * inter as f64) / ((av.len() - 1 + bv.len() - 1) as f64)
}

fn compare(needle: &str, haystack: &str) -> (Match, u8) {
    if haystack.is_empty() {
        return (Match::None, 0);
    }
    let n = fold_text(needle);
    let h = fold_text(haystack);
    if needle == haystack || (!n.is_empty() && n == h) {
        let conf = if needle == haystack { 100 } else { 96 };
        return (Match::Exact, conf);
    }
    let (longer, shorter) = if n.len() >= h.len() { (n.as_str(), h.as_str()) } else { (h.as_str(), n.as_str()) };
    if !shorter.is_empty() && longer.starts_with(shorter) {
        let slack = longer.chars().count() - shorter.chars().count();
        if slack <= MAX_CONTAINMENT_SLACK && shorter.chars().count() >= MIN_STRONG_LEN {
            return (Match::Strong, 88);
        }
        return (Match::Weak, 62);
    }
    if shorter.len() >= MIN_STRONG_LEN && longer.contains(shorter) {
        return (Match::Weak, 58);
    }
    let d = dice(&n, &h);
    if d >= 0.9 && n.chars().count() >= 4 {
        return (Match::Strong, (78.0 + d * 18.0).round() as u8);
    }
    if d >= 0.72 && n.chars().count() >= 4 {
        return (Match::Weak, (48.0 + d * 30.0).round() as u8);
    }
    (Match::None, (d * 40.0).round() as u8)
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
            confidence: None,
        }
    }

    #[test]
    fn strips_localisation_noise() {
        assert!(score("进击的巨人", &cand("进击的巨人", &[])).is_confident());
        assert!(score(" solo leveling 汉化 ", &cand("Solo Leveling", &["나 혼자만 레벨업"])).is_confident());
        let (kind, conf) = score_parts("進擊的巨人", &cand("进击的巨人", &[]));
        assert!(kind.is_confident(), "{kind:?}");
        assert!(conf >= 90);
        let (kind, conf) = score_parts("Shingeki no Kyojin", &cand("Shingeki", &["進撃の巨人"]));
        assert!(kind.is_confident() || conf >= 50, "{kind:?} {conf}");
        let (romaji, c2) = score_parts("Ōkami", &cand("Okami", &[]));
        assert!(romaji.is_confident(), "{c2}");
    }
}
