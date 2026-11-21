use crate::state;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

fn civil_date(ts: u64) -> String {
    let days = (ts / 86_400) as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

pub fn print(state_dir: &Path) {
    let text = fs::read_to_string(state_dir.join(state::LOG_FILE)).unwrap_or_default();
    let entries: Vec<Value> = text
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    if entries.is_empty() {
        println!("기록이 없습니다");
        return;
    }
    let event = |e: &Value| e["event"].as_str().unwrap_or("").to_string();
    let count = |name: &str| entries.iter().filter(|e| event(e) == name).count();
    let ts: Vec<u64> = entries.iter().filter_map(|e| e["ts"].as_u64()).collect();
    let (first, last) = (
        ts.iter().min().copied().unwrap_or(0),
        ts.iter().max().copied().unwrap_or(0),
    );

    let mut by_check: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for e in entries
        .iter()
        .filter(|e| event(e) == "block" || event(e) == "giveup")
    {
        for f in e["failed"].as_array().into_iter().flatten() {
            let slot = by_check
                .entry(f["check"].as_str().unwrap_or("?").to_string())
                .or_default();
            slot.0 += 1;
            slot.1 += f["count"].as_u64().unwrap_or(0) as usize;
        }
    }

    let mut groups: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for e in &entries {
        let key = (
            e["session"].as_str().unwrap_or("").to_string(),
            e["root"].as_str().unwrap_or("").to_string(),
        );
        groups.entry(key).or_default().push(event(e));
    }
    let (mut episodes, mut recovered) = (0, 0);
    for events in groups.values() {
        let mut open = false;
        for ev in events {
            match ev.as_str() {
                "block" if !open => {
                    open = true;
                    episodes += 1;
                }
                "pass" if open => {
                    open = false;
                    recovered += 1;
                }
                "giveup" => open = false,
                _ => {}
            }
        }
    }

    println!(
        "기간: {} ~ {} (검사 {}회, 대화 {}개)",
        civil_date(first),
        civil_date(last),
        entries.len(),
        groups.len()
    );
    println!(
        "통과 {}, 막음 {}, 포기 {}",
        count("pass"),
        count("block"),
        count("giveup")
    );
    println!("막은 뒤 같은 대화에서 통과까지 간 비율: {recovered}/{episodes}");
    for (check, (times, items)) in &by_check {
        println!("  {check}: {times}회 막음, 지적 {items}건");
    }
}

#[cfg(test)]
mod tests {
    use super::civil_date;

    #[test]
    fn converts_unix_time_to_date() {
        assert_eq!(civil_date(0), "1970-01-01");
        assert_eq!(civil_date(1_795_219_200), "2026-11-21");
    }
}
