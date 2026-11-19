use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

use chrono::{
    DateTime, Datelike, Duration, Local, NaiveDate, NaiveDateTime, TimeZone, Utc, Weekday,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use windows_sys::Win32::Storage::FileSystem::{
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
};

pub type Result<T> = std::result::Result<T, String>;
pub type Entries = BTreeMap<Uuid, Entry>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Course,
    Task,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepeatRule {
    Daily,
    Weekdays,
    Weekly,
}

impl RepeatRule {
    pub const OPTIONS: [&'static str; 4] = ["不重复", "每天", "每个工作日", "每周"];

    pub fn from_label(label: &str) -> Option<Self> {
        match label.trim() {
            "每天" => Some(Self::Daily),
            "每个工作日" => Some(Self::Weekdays),
            "每周" => Some(Self::Weekly),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Daily => "每天",
            Self::Weekdays => "每个工作日",
            Self::Weekly => "每周",
        }
    }

    pub fn next(self, time: DateTime<Utc>) -> DateTime<Utc> {
        match self {
            Self::Daily => time + Duration::days(1),
            Self::Weekly => time + Duration::weeks(1),
            Self::Weekdays => {
                let mut next = time + Duration::days(1);
                while matches!(
                    next.with_timezone(&Local).weekday(),
                    Weekday::Sat | Weekday::Sun
                ) {
                    next += Duration::days(1);
                }
                next
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Semester {
    pub name: String,
    pub first_monday: Option<NaiveDate>,
    pub campus: String,
}

impl Default for Semester {
    fn default() -> Self {
        Self {
            name: "未设置学期".into(),
            first_monday: None,
            campus: "长安校区".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub kind: Kind,
    pub title: String,
    #[serde(default)]
    pub teacher: String,
    pub place: String,
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
    pub lead: Option<u32>,
    pub done: bool,
    #[serde(default)]
    pub repeat: Option<RepeatRule>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub conflict: bool,
}

impl Entry {
    pub fn course_end(&self) -> Option<DateTime<Utc>> {
        self.end
            .or_else(|| self.start.map(|start| start + Duration::minutes(45)))
    }

    pub fn validates(&self) -> bool {
        !self.title.trim().is_empty()
            && self.title.chars().count() <= 300
            && self.teacher.chars().count() <= 200
            && self.place.chars().count() <= 300
            && self
                .lead
                .is_none_or(|lead| lead <= 10_080 && self.start.is_some())
            && self
                .end
                .is_none_or(|end| self.start.is_some_and(|start| end > start))
            && self.repeat.is_none_or(|_| self.start.is_some())
            && (self.kind != Kind::Course || self.start.is_some())
    }

    pub fn time_label(&self) -> String {
        let Some(start) = self.start.map(|time| time.with_timezone(&Local)) else {
            return "无截止时间".into();
        };
        match self.end.map(|end| end.with_timezone(&Local)) {
            Some(end) if end.date_naive() == start.date_naive() => {
                format!("{}–{}", start.format("%H:%M"), end.format("%H:%M"))
            }
            Some(end) => format!("{}–{}", start.format("%H:%M"), end.format("%m/%d %H:%M")),
            None => start.format("%H:%M").to_string(),
        }
    }

    pub fn detail_label(&self) -> String {
        [self.teacher.trim(), self.place.trim()]
            .into_iter()
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("   ·   ")
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DavSettings {
    pub url: String,
    pub username: String,
    pub password: Vec<u8>,
    pub interval: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Alarm {
    pub due: DateTime<Utc>,
    pub snoozed: bool,
    pub fired: bool,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub entries: Entries,
    pub base: Entries,
    pub alarms: BTreeMap<Uuid, Alarm>,
    pub dav: Option<DavSettings>,
    pub dav_verified: bool,
    pub semester: Semester,
}

impl State {
    pub fn alarm_for(&self, id: &Uuid, entry: &Entry) -> Option<DateTime<Utc>> {
        if entry.done {
            return None;
        }
        let start = entry.start?;
        if let Some(alarm) = self.alarms.get(id) {
            return (!alarm.fired).then_some(alarm.due);
        }
        start.checked_sub_signed(Duration::minutes(entry.lead?.into()))
    }

    pub fn update_entries(&mut self, entries: Entries) {
        self.alarms.retain(|id, _| {
            let Some(new) = entries.get(id) else {
                return false;
            };
            self.entries.get(id).is_some_and(|old| {
                old.start == new.start
                    && old.lead == new.lead
                    && old.done == new.done
                    && old.repeat == new.repeat
            })
        });
        self.entries = entries;
    }
}

pub fn local_time(text: &str) -> Result<DateTime<Utc>> {
    let naive = NaiveDateTime::parse_from_str(text.trim(), "%Y-%m-%d %H:%M")
        .map_err(|_| "请输入日期和时间，例如 2026-11-18 08:30".to_string())?;
    local_datetime(naive)
}

pub fn local_datetime(naive: NaiveDateTime) -> Result<DateTime<Utc>> {
    Local
        .from_local_datetime(&naive)
        .single()
        .map(|time| time.with_timezone(&Utc))
        .ok_or_else(|| "该本地时间不存在或有两个可能值，请换一个时间".into())
}

pub fn merge(base: &Entries, local: &Entries, remote: &Entries) -> (Entries, usize) {
    let ids: BTreeSet<_> = base
        .keys()
        .chain(local.keys())
        .chain(remote.keys())
        .copied()
        .collect();
    let mut merged = Entries::new();
    let mut copies = Vec::new();
    let mut conflicts = 0;
    for id in ids {
        let (before, ours, theirs) = (base.get(&id), local.get(&id), remote.get(&id));
        let chosen = if ours == theirs || theirs == before {
            ours
        } else if ours == before {
            theirs
        } else {
            conflicts += 1;
            if let (Some(ours), Some(theirs)) = (ours, theirs) {
                if let Some(combined) = merge_entry(before, ours, theirs) {
                    conflicts -= 1;
                    merged.insert(id, combined);
                    continue;
                }
                let key = Uuid::new_v5(&id, &serde_json::to_vec(ours).expect("serialize entry"));
                let mut copy = ours.clone();
                copy.conflict = true;
                copies.push((key, copy));
            }
            theirs.or(ours)
        };
        if let Some(entry) = chosen {
            merged.insert(id, entry.clone());
        }
    }
    for (id, entry) in copies {
        merged.entry(id).or_insert(entry);
    }
    (merged, conflicts)
}

fn merge_entry(before: Option<&Entry>, ours: &Entry, theirs: &Entry) -> Option<Entry> {
    fn field<T: Clone + PartialEq>(before: &T, ours: &T, theirs: &T) -> Option<T> {
        if ours == theirs || theirs == before {
            Some(ours.clone())
        } else if ours == before {
            Some(theirs.clone())
        } else {
            None
        }
    }
    let before = before?;
    let entry = Entry {
        kind: field(&before.kind, &ours.kind, &theirs.kind)?,
        title: field(&before.title, &ours.title, &theirs.title)?,
        teacher: field(&before.teacher, &ours.teacher, &theirs.teacher)?,
        place: field(&before.place, &ours.place, &theirs.place)?,
        start: field(&before.start, &ours.start, &theirs.start)?,
        end: field(&before.end, &ours.end, &theirs.end)?,
        lead: field(&before.lead, &ours.lead, &theirs.lead)?,
        done: field(&before.done, &ours.done, &theirs.done)?,
        repeat: field(&before.repeat, &ours.repeat, &theirs.repeat)?,
        conflict: ours.conflict || theirs.conflict,
    };
    entry.validates().then_some(entry)
}

pub fn overlapping(entries: &Entries, date: NaiveDate) -> BTreeSet<Uuid> {
    let mut ids = BTreeSet::new();
    let mut courses: Vec<_> = entries
        .iter()
        .filter_map(|(id, entry)| {
            let start = entry.start?;
            let local = start.with_timezone(&Local);
            (entry.kind == Kind::Course && local.date_naive() == date).then_some((
                *id,
                start,
                entry.course_end()?,
            ))
        })
        .collect();
    courses.sort_by_key(|(_, start, _)| *start);
    for (index, (left_id, left_start, left_end)) in courses.iter().enumerate() {
        for (right_id, right_start, right_end) in courses.iter().skip(index + 1) {
            if *right_start >= *left_end {
                break;
            }
            if *right_start < *left_end && *left_start < *right_end {
                ids.insert(*left_id);
                ids.insert(*right_id);
            }
        }
    }
    ids
}

pub fn data_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("COURSE_CLOCK_DATA") {
        return Ok(PathBuf::from(dir));
    }
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    Ok(exe
        .parent()
        .ok_or("找不到程序目录")?
        .join("CourseClock-data"))
}

pub fn open_state(path: &Path) -> Result<State> {
    match File::open(path) {
        Ok(file) => serde_json::from_reader(io::BufReader::new(file))
            .map_err(|error| format!("无法读取本地数据 {}：{error}", path.display())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(State::default()),
        Err(error) => Err(format!("无法打开 {}：{error}", path.display())),
    }
}

pub fn save(path: &Path, state: &State) -> Result<()> {
    fn write(path: &Path, temporary: &Path, state: &State) -> io::Result<()> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(temporary)?;
        serde_json::to_writer(&mut file, state)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        let from = crate::win::wide_os(temporary.as_os_str());
        let to = crate::win::wide_os(path.as_os_str());
        if unsafe {
            MoveFileExW(
                from.as_ptr(),
                to.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    let temporary = path.with_extension(format!("{}.tmp", Uuid::new_v4()));
    if let Err(error) = write(path, &temporary, state) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("保存本地数据失败：{error}"));
    }
    Ok(())
}

pub fn backup(path: &Path, state: &State) -> Result<PathBuf> {
    let directory = path.parent().ok_or("找不到本地数据目录")?.join("backups");
    fs::create_dir_all(&directory).map_err(|error| format!("无法创建备份目录：{error}"))?;
    let stamp = Local::now().format("%Y%m%d-%H%M%S");
    let destination = directory.join(format!("state-{stamp}.json"));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&destination)
        .map_err(|error| format!("无法创建备份文件：{error}"))?;
    serde_json::to_writer_pretty(&mut file, state).map_err(|error| error.to_string())?;
    file.flush().map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;

    let mut files: Vec<_> = fs::read_dir(&directory)
        .map_err(|error| error.to_string())?
        .filter_map(|item| item.ok())
        .filter(|item| item.path().extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort_by_key(|item| item.file_name());
    for item in files.into_iter().rev().skip(12) {
        let _ = fs::remove_file(item.path());
    }
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn course(start: &str, end: &str) -> Entry {
        Entry {
            kind: Kind::Course,
            title: "测试".into(),
            teacher: String::new(),
            place: String::new(),
            start: Some(start.parse().unwrap()),
            end: Some(end.parse().unwrap()),
            lead: Some(10),
            done: false,
            repeat: None,
            conflict: false,
        }
    }

    #[test]
    fn overlaps_are_reported_for_the_same_local_day() {
        let mut entries = Entries::new();
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        entries.insert(
            first,
            course("2026-11-18T01:00:00Z", "2026-11-18T02:00:00Z"),
        );
        entries.insert(
            second,
            course("2026-11-18T01:45:00Z", "2026-11-18T03:00:00Z"),
        );
        let ids = overlapping(&entries, NaiveDate::from_ymd_opt(2026, 11, 18).unwrap());
        assert_eq!(ids, BTreeSet::from([first, second]));
    }

    #[test]
    fn field_merge_preserves_independent_changes() {
        let id = Uuid::new_v4();
        let base = Entries::from([(id, course("2026-11-18T01:00:00Z", "2026-11-18T02:00:00Z"))]);
        let mut local = base.clone();
        local.get_mut(&id).unwrap().place = "教A101".into();
        let mut remote = base.clone();
        remote.get_mut(&id).unwrap().teacher = "张老师".into();
        let (merged, conflicts) = merge(&base, &local, &remote);
        assert_eq!(conflicts, 0);
        assert_eq!(merged[&id].place, "教A101");
        assert_eq!(merged[&id].teacher, "张老师");
    }

    #[test]
    fn repeated_task_skips_weekends() {
        let friday = "2026-11-20T01:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let next = RepeatRule::Weekdays.next(friday).with_timezone(&Local);
        assert_eq!(next.weekday(), Weekday::Mon);
    }

    #[test]
    fn old_state_json_gets_new_defaults() {
        let state: State = serde_json::from_str(
            r#"{"entries":{"00000000-0000-0000-0000-000000000001":{"kind":"Task","title":"旧数据","place":"","start":null,"end":null,"lead":null,"done":false}}}"#,
        )
        .unwrap();
        let entry = state.entries.values().next().unwrap();
        assert!(entry.teacher.is_empty());
        assert!(entry.repeat.is_none());
        assert_eq!(state.semester, Semester::default());
    }
}
