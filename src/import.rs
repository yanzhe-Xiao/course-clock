use std::{
    collections::{BTreeMap, BTreeSet},
    io::Cursor,
};

use chrono::{
    DateTime, Datelike, Duration, FixedOffset, Local, NaiveDate, NaiveDateTime, NaiveTime,
    TimeZone, Utc, Weekday,
};
use ical::property::Property;
use regex::Regex;
use rrule::RRuleSet;
use uuid::Uuid;

use crate::model::{Entries, Entry, Kind, Result};

#[derive(Clone, Copy)]
pub enum Campus {
    Changan,
    YouyiSummer,
    YouyiWinter,
}

impl Campus {
    pub const OPTIONS: [(&str, Self); 3] = [
        ("长安校区", Self::Changan),
        ("友谊校区 · 夏季", Self::YouyiSummer),
        ("友谊校区 · 冬季", Self::YouyiWinter),
    ];

    fn slots(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Changan => &[
                ("08:30", "09:15"),
                ("09:25", "10:10"),
                ("10:30", "11:15"),
                ("11:25", "12:10"),
                ("12:20", "13:05"),
                ("13:05", "13:50"),
                ("14:00", "14:45"),
                ("14:55", "15:40"),
                ("16:00", "16:45"),
                ("16:55", "17:40"),
                ("19:00", "19:45"),
                ("19:55", "20:40"),
                ("20:40", "21:25"),
            ],
            Self::YouyiSummer => &[
                ("08:00", "08:50"),
                ("09:00", "09:50"),
                ("10:10", "11:00"),
                ("11:10", "12:00"),
                ("12:20", "13:05"),
                ("13:05", "13:50"),
                ("14:30", "15:20"),
                ("15:30", "16:20"),
                ("16:40", "17:30"),
                ("17:40", "18:30"),
                ("19:30", "20:20"),
                ("20:30", "21:20"),
            ],
            Self::YouyiWinter => &[
                ("08:00", "08:50"),
                ("09:00", "09:50"),
                ("10:10", "11:00"),
                ("11:10", "12:00"),
                ("12:20", "13:05"),
                ("13:05", "13:50"),
                ("14:00", "14:50"),
                ("15:00", "15:50"),
                ("16:10", "17:00"),
                ("17:10", "18:00"),
                ("19:00", "19:50"),
                ("20:00", "20:50"),
            ],
        }
    }
}

fn course(
    title: String,
    teacher: String,
    place: String,
    start: DateTime<Utc>,
    end: Option<DateTime<Utc>>,
) -> Entry {
    Entry {
        kind: Kind::Course,
        title,
        teacher,
        place,
        start: Some(start),
        end,
        done: false,
        lead: Some(10),
        repeat: None,
        conflict: false,
    }
}

pub fn csv(bytes: &[u8], monday: NaiveDate, campus: Campus) -> Result<Entries> {
    if monday.weekday() != Weekday::Mon {
        return Err("第一教学周的日期须为周一".into());
    }
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => std::borrow::Cow::Borrowed(text),
        Err(_) => encoding_rs::GBK
            .decode_without_bom_handling_and_without_replacement(bytes)
            .ok_or("无法识别 CSV 编码，请保存为 UTF-8 或 GBK")?,
    };
    let mut reader = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .flexible(true)
        .from_reader(text.as_bytes());
    let headers = reader.headers().map_err(|error| error.to_string())?.clone();
    let column = |names: &[&str]| {
        headers
            .iter()
            .position(|header| names.contains(&header))
            .ok_or_else(|| format!("CSV 缺少「{}」列", names[0]))
    };
    let title = column(&["课程名称", "课程名"])?;
    let day = column(&["星期", "星期几"])?;
    let first = column(&["开始节数", "开始节次"])?;
    let last = column(&["结束节数", "结束节次"])?;
    let place = column(&["地点", "教室", "上课地点"])?;
    let weeks = column(&["周数", "周次"])?;
    let periods = campus.slots();
    let china = FixedOffset::east_opt(8 * 3600).unwrap();
    let mut entries = Entries::new();
    for (index, row) in reader.records().enumerate() {
        let row = row.map_err(|error| error.to_string())?;
        let line = row.position().map_or(index as u64 + 2, |pos| pos.line());
        let invalid = |what: &str| format!("CSV 第 {line} 行 · {what}");
        if row.len() < headers.len() || (row.len() > headers.len() && weeks != headers.len() - 1) {
            return Err(invalid("列数与表头不符"));
        }
        let start = row[first]
            .parse::<usize>()
            .ok()
            .filter(|n| (1..=periods.len()).contains(n));
        let end = row[last]
            .parse::<usize>()
            .ok()
            .filter(|n| (1..=periods.len()).contains(n));
        let (first, last) = start
            .zip(end)
            .filter(|(a, b)| a <= b)
            .ok_or_else(|| invalid("节次超出所选校区范围"))?;
        let day = weekday(&row[day]).ok_or_else(|| invalid("星期须在 1–7 之间"))?;
        let weeks_text = if weeks == headers.len() - 1 {
            row.iter().skip(weeks).collect::<Vec<_>>().join(",")
        } else {
            row[weeks].to_string()
        };
        for week in week_numbers(&weeks_text).map_err(|error| invalid(&error))? {
            let date = monday
                .checked_add_signed(Duration::days(i64::from((week - 1) * 7 + day)))
                .ok_or_else(|| invalid("日期超出有效范围"))?;
            let at = |text| {
                let time = NaiveTime::parse_from_str(text, "%H:%M").expect("school timetable");
                china
                    .from_local_datetime(&date.and_time(time))
                    .single()
                    .unwrap()
                    .with_timezone(&Utc)
            };
            let start = at(periods[first - 1].0);
            let teacher = headers
                .iter()
                .position(|header| ["老师", "教师", "任课教师"].contains(&header))
                .and_then(|column| row.get(column))
                .unwrap_or_default()
                .to_string();
            let entry = course(
                row[title].to_string(),
                teacher,
                row[place].to_string(),
                start,
                Some(at(periods[last - 1].1)),
            );
            if !entry.validates() {
                return Err(invalid("课程名或地点超出有效范围"));
            }
            let key = format!("nwpu:{monday}:{week}:{day}:{first}:{}", entry.title);
            if entries
                .insert(
                    Uuid::new_v5(&Uuid::NAMESPACE_OID, key.as_bytes()),
                    entry.clone(),
                )
                .is_some_and(|previous| previous != entry)
            {
                return Err(invalid("同一课程在相同日期和节次有多条不同记录"));
            }
        }
    }
    if entries.is_empty() {
        return Err("CSV 中没有课程".into());
    }
    Ok(entries)
}

pub fn html(bytes: &[u8], monday: NaiveDate, campus: Campus) -> Result<Entries> {
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => std::borrow::Cow::Borrowed(text),
        Err(_) => encoding_rs::GBK
            .decode_without_bom_handling_and_without_replacement(bytes)
            .ok_or("无法识别 HTML 编码，请保存为 UTF-8 或 GBK")?,
    };
    if text.contains("lessonInfo") && text.contains("data-course=") {
        return nwpu_html(&text, monday, campus);
    }
    if !text.contains("courseTable") && !text.contains("course-name") {
        return Err("HTML 中没有找到西工大课程表结构".into());
    }
    let row_re = Regex::new(r"(?is)<tr\b[^>]*>(.*?)</tr>").map_err(|error| error.to_string())?;
    let cell_re = Regex::new(r"(?is)<td\b[^>]*>(.*?)</td>").map_err(|error| error.to_string())?;
    let title_re = Regex::new(r#"(?is)<(?:div|span)\b[^>]*class\s*=\s*["'][^"']*course-name[^"']*["'][^>]*>(.*?)</(?:div|span)>"#)
        .map_err(|error| error.to_string())?;
    let period_re = Regex::new(r"(?i)(\d{1,2})\s*(?:[-~至]\s*(\d{1,2}))?\s*节")
        .map_err(|error| error.to_string())?;
    let week_re = Regex::new(r"(?i)(\d{1,2})\s*(?:[-~至]\s*(\d{1,2}))?\s*周")
        .map_err(|error| error.to_string())?;
    let mut rows = Entries::new();
    for (row_index, row_match) in row_re.captures_iter(&text).enumerate() {
        let row = &row_match[1];
        for (column, cell_match) in cell_re.captures_iter(row).enumerate() {
            let cell = &cell_match[1];
            let titles: Vec<_> = title_re
                .captures_iter(cell)
                .map(|capture| html_text(&capture[1]))
                .filter(|title| !title.is_empty())
                .collect();
            if titles.is_empty() || column >= 7 {
                continue;
            }
            let plain = html_text(cell);
            let periods = period_re.captures(&plain);
            let (first, last) = periods
                .as_ref()
                .and_then(|capture| {
                    let first = capture.get(1)?.as_str().parse::<usize>().ok()?;
                    let last = capture
                        .get(2)
                        .and_then(|value| value.as_str().parse::<usize>().ok())
                        .unwrap_or(first);
                    Some((first, last))
                })
                .or_else(|| {
                    (row_index < campus.slots().len()).then_some((row_index + 1, row_index + 1))
                })
                .ok_or("HTML 课程表缺少节次信息，请改用 CSV 导入")?;
            let weeks = week_re
                .captures(&plain)
                .and_then(|capture| {
                    let first = capture.get(1)?.as_str().parse::<u32>().ok()?;
                    let last = capture
                        .get(2)
                        .and_then(|value| value.as_str().parse::<u32>().ok())
                        .unwrap_or(first);
                    Some(format!("{first}-{last}"))
                })
                .unwrap_or_else(|| "1-60".into());
            let teacher = extract_label(&plain, &["教师", "老师", "任课教师"]);
            let place = extract_label(&plain, &["地点", "教室", "上课地点"]);
            for title in titles {
                let mut writer = csv::Writer::from_writer(Vec::new());
                writer
                    .write_record([
                        "课程名称",
                        "星期",
                        "开始节数",
                        "结束节数",
                        "老师",
                        "地点",
                        "周数",
                    ])
                    .map_err(|error| error.to_string())?;
                writer
                    .write_record([
                        title.as_str(),
                        &(column + 1).to_string(),
                        &first.to_string(),
                        &last.to_string(),
                        &teacher,
                        &place,
                        &weeks,
                    ])
                    .map_err(|error| error.to_string())?;
                let bytes = writer.into_inner().map_err(|error| error.to_string())?;
                let parsed = csv(&bytes, monday, campus)?;
                rows.extend(parsed);
            }
        }
    }
    if rows.is_empty() {
        return Err("没有从 HTML 中解析出带周次和节次的课程".into());
    }
    Ok(rows)
}

fn nwpu_html(text: &str, monday: NaiveDate, campus: Campus) -> Result<Entries> {
    if monday.weekday() != Weekday::Mon {
        return Err("第一教学周的日期须为周一".into());
    }
    let row_re = Regex::new(
        r#"(?is)<tr\b[^>]*class\s*=\s*["'][^"']*\blessonInfo\b[^"']*["'][^>]*>(.*?)</tr>"#,
    )
    .map_err(|error| error.to_string())?;
    let cell_re = Regex::new(r"(?is)<td\b[^>]*>(.*?)</td>").map_err(|error| error.to_string())?;
    let course_re = Regex::new(r#"(?is)data-course\s*=\s*["']([^"']+)["']"#)
        .map_err(|error| error.to_string())?;
    let title_re = Regex::new(
        r#"(?is)<p\b[^>]*class\s*=\s*["'][^"']*\bshowSchedules\b[^"']*["'][^>]*>(.*?)</p>"#,
    )
    .map_err(|error| error.to_string())?;
    let teacher_re = Regex::new(r"授课教师：\s*([^<]+)").map_err(|error| error.to_string())?;
    let day_re = Regex::new(r"周([一二三四五六日天])").map_err(|error| error.to_string())?;
    let period_re = Regex::new(
        r"第([一二三四五六七八九十百]+)节(?:\s*[~～-]\s*第([一二三四五六七八九十百]+)节)?",
    )
    .map_err(|error| error.to_string())?;
    let place_re =
        Regex::new(r"((?:长安|友谊)校区(?:\s+\S+)?)").map_err(|error| error.to_string())?;
    let schedule_separator = Regex::new(r"(?is)<br\s*/?>|;").map_err(|error| error.to_string())?;
    let mut entries = Entries::new();
    let slots = campus.slots();
    for row_capture in row_re.captures_iter(text) {
        let row = &row_capture[1];
        let course_code = course_re
            .captures(row)
            .and_then(|capture| capture.get(1))
            .map(|value| value.as_str());
        if course_code.is_none() {
            continue;
        }
        let title = title_re
            .captures(row)
            .map(|capture| html_text(&capture[1]))
            .filter(|title| !title.is_empty())
            .or_else(|| {
                course_code.map(|code| code.split('[').next().unwrap_or(code).trim().to_string())
            })
            .ok_or("西工大课表行缺少课程名称")?;
        let teacher = teacher_re
            .captures(row)
            .map(|capture| capture[1].trim().to_string())
            .unwrap_or_default();
        let cells: Vec<_> = cell_re
            .captures_iter(row)
            .map(|capture| capture[1].to_string())
            .collect();
        let schedule = cells.get(2).ok_or("西工大课表行缺少上课安排")?;
        for part in schedule_separator
            .split(schedule)
            .map(str::trim)
            .filter(|part| !part.is_empty())
        {
            let plain = html_text(part);
            let day_capture = day_re
                .captures(&plain)
                .ok_or_else(|| format!("课程「{title}」缺少星期：{plain}"))?;
            let day = chinese_day(&day_capture[1])
                .ok_or_else(|| format!("课程「{title}」缺少星期：{plain}"))?;
            let weeks_text = &plain[..day_capture.get(0).map_or(0, |found| found.start())];
            let weeks = week_numbers(weeks_text)
                .map_err(|error| format!("课程「{title}」{error}：{plain}"))?;
            let (first, last) = period_re
                .captures(&plain)
                .and_then(|capture| {
                    let first = chinese_number(&capture[1])?;
                    let last = capture
                        .get(2)
                        .and_then(|value| chinese_number(value.as_str()))
                        .unwrap_or(first);
                    Some((first, last))
                })
                .ok_or_else(|| format!("课程「{title}」缺少节次：{plain}"))?;
            if first == 0 || last < first || last > slots.len() {
                return Err(format!("课程「{title}」节次超出所选校区作息范围"));
            }
            let place = place_re
                .captures(&plain)
                .map(|capture| capture[1].trim().to_string())
                .unwrap_or_default();
            let china = FixedOffset::east_opt(8 * 3600).unwrap();
            let at = |date: NaiveDate, time: &str| {
                let time = NaiveTime::parse_from_str(time, "%H:%M").expect("school timetable");
                china
                    .from_local_datetime(&date.and_time(time))
                    .single()
                    .unwrap()
                    .with_timezone(&Utc)
            };
            for week in weeks {
                let date = monday
                    .checked_add_signed(Duration::days(i64::from((week - 1) * 7 + day)))
                    .ok_or("西工大课表日期超出有效范围")?;
                let start = at(date, slots[first - 1].0);
                let entry = course(
                    title.clone(),
                    teacher.clone(),
                    place.clone(),
                    start,
                    Some(at(date, slots[last - 1].1)),
                );
                let key = format!("nwpu-html:{course_code:?}:{date}:{first}:{title}");
                entries.insert(Uuid::new_v5(&Uuid::NAMESPACE_OID, key.as_bytes()), entry);
            }
        }
    }
    if entries.is_empty() {
        return Err("没有从西工大课表 HTML 中解析出课程".into());
    }
    Ok(entries)
}

fn chinese_day(text: &str) -> Option<u32> {
    match text {
        "一" => Some(0),
        "二" => Some(1),
        "三" => Some(2),
        "四" => Some(3),
        "五" => Some(4),
        "六" => Some(5),
        "日" | "天" => Some(6),
        _ => None,
    }
}

fn chinese_number(text: &str) -> Option<usize> {
    let mut value = 0_usize;
    let mut digit = 0_usize;
    for ch in text.chars() {
        let current = match ch {
            '一' => 1,
            '二' => 2,
            '三' => 3,
            '四' => 4,
            '五' => 5,
            '六' => 6,
            '七' => 7,
            '八' => 8,
            '九' => 9,
            '十' => 10,
            '百' => 100,
            _ => return None,
        };
        if current == 10 || current == 100 {
            value += if digit == 0 { current } else { digit * current };
            digit = 0;
        } else {
            digit = current;
        }
    }
    Some(value + digit)
}

fn extract_label(text: &str, labels: &[&str]) -> String {
    for label in labels {
        if let Some(index) = text.find(label) {
            let value = text[index + label.len()..]
                .trim_matches(|ch: char| ch == ':' || ch == '：' || ch.is_whitespace())
                .split_whitespace()
                .next()
                .unwrap_or_default();
            if !value.is_empty() {
                return value.to_string();
            }
        }
    }
    String::new()
}

fn html_text(text: &str) -> String {
    let tags = Regex::new(r"(?is)<[^>]+>").expect("static regex");
    tags.replace_all(text, " ")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn weekday(text: &str) -> Option<u32> {
    let value = text.trim_start_matches("星期").trim_start_matches("周");
    match value {
        "一" => Some(0),
        "二" => Some(1),
        "三" => Some(2),
        "四" => Some(3),
        "五" => Some(4),
        "六" => Some(5),
        "日" | "天" => Some(6),
        _ => value
            .parse::<u32>()
            .ok()
            .filter(|day| (1..=7).contains(day))
            .map(|day| day - 1),
    }
}

fn week_numbers(text: &str) -> Result<BTreeSet<u32>> {
    let text = text
        .replace(['，', '、', ';', '；', '＋'], ",")
        .replace(['–', '—', '－', '~', '～', '至'], "-")
        .replace(['第', '周', '(', ')', '（', '）', '[', ']', ' '], "");
    let error = || format!("无法识别周次「{text}」");
    let mut weeks = BTreeSet::new();
    for part in text.split(',').filter(|part| !part.is_empty()) {
        let (range, parity) = if let Some(range) = part.strip_suffix('单') {
            (range, Some(1))
        } else if let Some(range) = part.strip_suffix('双') {
            (range, Some(0))
        } else {
            (part, None)
        };
        let (from, to) = range.split_once('-').unwrap_or((range, range));
        let (from, to) = (
            from.parse::<u32>().map_err(|_| error())?,
            to.parse::<u32>().map_err(|_| error())?,
        );
        if from == 0 || from > to || to > 60 {
            return Err(error());
        }
        weeks.extend((from..=to).filter(|week| parity.is_none_or(|p| week % 2 == p)));
    }
    if weeks.is_empty() {
        Err(error())
    } else {
        Ok(weeks)
    }
}

pub struct Calendar {
    pub entries: Entries,
    pub skipped: usize,
    pub recurring: bool,
}

pub fn ics(bytes: &[u8], now: DateTime<Utc>) -> Result<Calendar> {
    let mut result = Calendar {
        entries: Entries::new(),
        skipped: 0,
        recurring: false,
    };
    let mut overrides = BTreeMap::new();
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let window_start = now - Duration::days(180);
    let window_end = now + Duration::days(550);
    for calendar in ical::IcalParser::new(Cursor::new(bytes)) {
        let calendar = calendar.map_err(|error| format!("无法读取 ICS：{error}"))?;
        for event in calendar.events {
            let props = &event.properties;
            let find = |name: &str| props.iter().find(|p| p.name.eq_ignore_ascii_case(name));
            let uid = value(find("UID"));
            let summary = unescape(value(find("SUMMARY")));
            let place = unescape(value(find("LOCATION")));
            let cancelled = value(find("STATUS")).eq_ignore_ascii_case("CANCELLED");
            let recurrence = find("RECURRENCE-ID");
            let item_id = |time: DateTime<Utc>| {
                let key = if uid.is_empty() {
                    format!("ical:{summary}:{}", time.timestamp())
                } else {
                    format!("ical:{uid}:{}", time.timestamp())
                };
                Uuid::new_v5(&Uuid::NAMESPACE_OID, key.as_bytes())
            };
            let overridden_id = recurrence.and_then(|p| date(p).ok()).map(item_id);
            if recurrence.is_some() && overridden_id.is_none() {
                result.skipped += 1;
                continue;
            }
            if cancelled {
                if let Some(id) = overridden_id {
                    overrides.insert(id, None);
                }
                continue;
            }
            let Some(start_prop) = find("DTSTART") else {
                result.skipped += 1;
                continue;
            };
            let Ok(start) = date(start_prop) else {
                result.skipped += 1;
                continue;
            };
            let end = find("DTEND").map(date).transpose();
            let duration = match end {
                Ok(Some(end)) if end > start => Some(end - start),
                Ok(None) => {
                    if let Some(property) = find("DURATION") {
                        let Some(duration) = ics_duration(value(Some(property))) else {
                            result.skipped += 1;
                            continue;
                        };
                        Some(duration)
                    } else {
                        None
                    }
                }
                _ => {
                    result.skipped += 1;
                    continue;
                }
            };
            let entry_at = |start: DateTime<Utc>| {
                course(
                    summary.clone(),
                    String::new(),
                    place.clone(),
                    start,
                    duration.and_then(|duration| start.checked_add_signed(duration)),
                )
            };
            if !entry_at(start).validates() {
                result.skipped += 1;
                continue;
            }
            if let Some(id) = overridden_id {
                overrides.insert(id, Some(entry_at(start)));
                continue;
            }
            let rules: Vec<_> = props
                .iter()
                .filter(|p| matches!(p.name.as_str(), "RRULE" | "RDATE" | "EXDATE"))
                .collect();
            if rules.is_empty() {
                result.entries.insert(item_id(start), entry_at(start));
                continue;
            }
            result.recurring = true;
            let mut rule = property_line(start_prop);
            for property in rules {
                rule.push('\n');
                rule.push_str(&property_line(property));
            }
            let Ok(rule) = rule.parse::<RRuleSet>() else {
                result.skipped += 1;
                continue;
            };
            let occurrences = rule
                .after(window_start.with_timezone(&rrule::Tz::UTC))
                .before(window_end.with_timezone(&rrule::Tz::UTC))
                .all(10_000);
            if occurrences.limited {
                return Err("ICS 重复规则超出可导入范围，请缩短日历或降低重复频率".into());
            }
            for time in occurrences.dates {
                let time = time.with_timezone(&Utc);
                result.entries.insert(item_id(time), entry_at(time));
            }
        }
    }
    for (id, entry) in overrides {
        if let Some(entry) = entry {
            result.entries.insert(id, entry);
        } else {
            result.entries.remove(&id);
        }
    }
    if result.entries.is_empty() {
        return Err("ICS 中没有可导入的带时间事件".into());
    }
    Ok(result)
}

fn value(property: Option<&Property>) -> &str {
    property
        .and_then(|p| p.value.as_deref())
        .unwrap_or_default()
}

fn date(property: &Property) -> Result<DateTime<Utc>> {
    let text = value(Some(property));
    let parse = |text| {
        NaiveDateTime::parse_from_str(text, "%Y%m%dT%H%M%S")
            .map_err(|_| format!("无法识别日期：{text}"))
    };
    if let Some(utc) = text.strip_suffix('Z') {
        return Ok(parse(utc)?.and_utc());
    }
    let naive = parse(text)?;
    let timezone = property
        .params
        .iter()
        .flatten()
        .find(|(key, _)| key == "TZID")
        .and_then(|(_, values)| values.first());
    let time = match timezone {
        Some(zone) => zone
            .parse::<chrono_tz::Tz>()
            .ok()
            .and_then(|tz| tz.from_local_datetime(&naive).single())
            .map(|dt| dt.with_timezone(&Utc)),
        None => Local
            .from_local_datetime(&naive)
            .single()
            .map(|dt| dt.with_timezone(&Utc)),
    };
    time.ok_or_else(|| format!("无法识别时区或本地时间：{text}"))
}

fn property_line(property: &Property) -> String {
    let mut line = property.name.clone();
    for (key, values) in property.params.iter().flatten() {
        if key == "TZID" {
            line.push_str(&format!(";{key}={}", values.join(",")));
        }
    }
    line.push(':');
    line.push_str(value(Some(property)));
    line
}

fn unescape(text: &str) -> String {
    let mut output = String::new();
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        match (ch == '\\').then(|| chars.next()).flatten() {
            Some('n' | 'N') => output.push(' '),
            Some(escaped) => output.push(escaped),
            None => output.push(ch),
        }
    }
    output.trim().to_string()
}

fn ics_duration(text: &str) -> Option<Duration> {
    let mut number = 0_i64;
    let mut seconds = 0_i64;
    let mut digits = false;
    let mut time = false;
    for ch in text.trim_start_matches('+').strip_prefix('P')?.chars() {
        if let Some(digit) = ch.to_digit(10) {
            number = number.checked_mul(10)?.checked_add(digit.into())?;
            digits = true;
        } else if ch == 'T' && !digits && !time {
            time = true;
        } else {
            let unit = match (time, ch) {
                (false, 'W') => 604_800,
                (false, 'D') => 86_400,
                (true, 'H') => 3_600,
                (true, 'M') => 60,
                (true, 'S') => 1,
                _ => return None,
            };
            if !digits {
                return None;
            }
            seconds = seconds.checked_add(number.checked_mul(unit)?)?;
            digits = false;
            number = 0;
        }
    }
    (!digits && seconds > 0)
        .then(|| Duration::try_seconds(seconds))
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nwpu_csv_expands_odd_weeks_and_unquoted_final_column() {
        let text = "\u{feff}课程名称,星期,开始节数,结束节数,老师,地点,周数\r\n\
            数据结构,1,1,2,教师,\"教 A101, 长安\",1-5(单),8\r\n";
        let monday = NaiveDate::from_ymd_opt(2026, 9, 7).unwrap();
        let entries = csv(text.as_bytes(), monday, Campus::Changan).unwrap();
        let mut entries: Vec<_> = entries.values().collect();
        entries.sort_by_key(|entry| entry.start);
        assert_eq!(entries.len(), 4);
        assert_eq!(
            entries[0].start.unwrap().to_rfc3339(),
            "2026-09-07T00:30:00+00:00"
        );
        assert_eq!(
            entries[0].end.unwrap().to_rfc3339(),
            "2026-09-07T02:10:00+00:00"
        );
        assert_eq!(entries[0].place, "教 A101, 长安");
        assert_eq!(
            entries[3].start.unwrap().date_naive().to_string(),
            "2026-10-26"
        );
        assert!(csv(text.as_bytes(), monday.succ_opt().unwrap(), Campus::Changan).is_err());
    }

    #[test]
    fn ical_weekly_class_honors_time_zone_exceptions_and_moved_instance() {
        let text = "\
BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Test//EN\r\n\
BEGIN:VEVENT\r\nUID:class-1\r\nSUMMARY:数据结构\r\nLOCATION:教 A101\\, 长安\r\n\
DTSTART;TZID=Asia/Shanghai:20261102T083000\r\nDURATION:PT1H40M\r\n\
RRULE:FREQ=WEEKLY;COUNT=4\r\nEXDATE;TZID=Asia/Shanghai:20261109T083000\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:class-1\r\nRECURRENCE-ID;TZID=Asia/Shanghai:20261116T083000\r\n\
DTSTART;TZID=Asia/Shanghai:20261117T083000\r\nDTEND;TZID=Asia/Shanghai:20261117T101000\r\n\
SUMMARY:调课\r\nLOCATION:教 B102\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let now = "2026-11-18T05:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let calendar = ics(text.as_bytes(), now).unwrap();
        let mut entries: Vec<_> = calendar.entries.values().collect();
        entries.sort_by_key(|entry| entry.start);
        assert_eq!(calendar.skipped, 0);
        assert_eq!(entries.len(), 3);
        assert_eq!(
            entries[0].start.unwrap().to_rfc3339(),
            "2026-11-02T00:30:00+00:00"
        );
        assert_eq!(
            entries[0].end.unwrap().to_rfc3339(),
            "2026-11-02T02:10:00+00:00"
        );
        assert_eq!(entries[0].place, "教 A101, 长安");
        assert_eq!(
            entries[1].start.unwrap().date_naive().to_string(),
            "2026-11-17"
        );
        assert_eq!(entries[1].title, "调课");
    }

    #[test]
    fn html_course_table_uses_day_column_and_csv_compatible_fields() {
        let text = r#"
        <table class="courseTable"><tr>
          <td><div class="tdHtml"><div class="course-name">数据结构</div>1-4周 1-2节 教师：张老师 地点：教A101</div></td>
          <td></td><td></td><td></td><td></td><td></td><td></td>
        </tr></table>"#;
        let monday = NaiveDate::from_ymd_opt(2026, 9, 7).unwrap();
        let entries = html(text.as_bytes(), monday, Campus::Changan).unwrap();
        let mut values: Vec<_> = entries.values().collect();
        values.sort_by_key(|entry| entry.start);
        assert_eq!(values.len(), 4);
        assert_eq!(values[0].title, "数据结构");
        assert_eq!(values[0].teacher, "张老师");
        assert_eq!(values[0].place, "教A101");
        assert_eq!(
            values[0].start.unwrap().date_naive().to_string(),
            "2026-09-07"
        );
    }

    #[test]
    fn nwpu_html_expands_comma_separated_week_segments() {
        let text = r#"<table><tr class="lessonInfo">
          <td class="courseInfo" data-course="High Performance Computing[M10E22003]">
            <p class="showSchedules">High Performance Computing</p>
            <p>M10E22003</p>
            <p>王云岚(2004000149)</p>
          </td>
          <td class="text"><p class="inner">1-2节</p></td>
          <td>11~16,18~19周 周三 第七节~第十节 长安校区 教西D-301</td>
        </tr></table>"#;
        let monday = NaiveDate::from_ymd_opt(2026, 8, 31).unwrap();
        let entries = html(text.as_bytes(), monday, Campus::Changan).unwrap();
        let mut dates: Vec<_> = entries
            .values()
            .map(|entry| entry.start.unwrap().date_naive().to_string())
            .collect();
        dates.sort();
        assert_eq!(
            dates,
            [
                "2026-11-11", "2026-11-18", "2026-11-25", "2026-12-02", "2026-12-09",
                "2026-12-16", "2026-12-30", "2027-01-06",
            ]
        );
        assert!(entries.values().all(|entry| entry.title == "High Performance Computing"));
    }

    #[test]
    fn optional_real_nwpu_html_smoke() {
        let Ok(path) = std::env::var("COURSE_CLOCK_REAL_HTML") else {
            return;
        };
        let bytes = std::fs::read(path).unwrap();
        let entries = html(
            &bytes,
            NaiveDate::from_ymd_opt(2026, 8, 31).unwrap(),
            Campus::Changan,
        )
        .unwrap();
        println!("real NWPU HTML entries: {}", entries.len());
        assert!(entries.len() > 10);
    }
}
