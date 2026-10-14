use std::collections::HashSet;

/// Accept seeds, whitespace tables, or CSV rows whose final column is a seed.
pub fn parse_seed_list(text: &str) -> Result<Vec<i32>, String> {
    let mut seeds = Vec::new();
    let mut unique = HashSet::new();
    for (index, line) in text.lines().enumerate() {
        if let Some(seed) = parse_seed_line(line, index + 1, seeds.is_empty())? {
            if unique.insert(seed) {
                seeds.push(seed);
            }
        }
    }
    if seeds.is_empty() {
        return Err("文件不包含有效种子".into());
    }
    Ok(seeds)
}

fn parse_seed_line(
    line: &str,
    number: usize,
    before_first_seed: bool,
) -> Result<Option<i32>, String> {
    let line = line.trim().trim_start_matches('\u{feff}');
    if line.is_empty() || line.starts_with('#') {
        return Ok(None);
    }
    let field = seed_field(line);
    if field.is_empty() {
        return Err(format!("第 {number} 行没有种子"));
    }
    if before_first_seed && is_seed_header(field) {
        return Ok(None);
    }
    field
        .parse::<i32>()
        .map(Some)
        .map_err(|error| format!("第 {number} 行种子无效 ({field}): {error}"))
}

fn seed_field(line: &str) -> &str {
    let field = if line.contains([',', ';']) {
        line.rsplit([',', ';']).next().unwrap_or_default()
    } else {
        line.split_whitespace().next_back().unwrap_or_default()
    };
    field.trim().trim_matches('"')
}

fn is_seed_header(field: &str) -> bool {
    matches!(field.to_lowercase().as_str(), "seed" | "数字种子" | "种子")
}

pub fn inclusive_seed_count(start: i32, end: i32) -> Result<usize, String> {
    if start > end {
        return Err("起始种子不能大于结束种子".into());
    }
    usize::try_from(i64::from(end) - i64::from(start) + 1)
        .map_err(|_| "种子区间超出当前平台的计数范围".into())
}
