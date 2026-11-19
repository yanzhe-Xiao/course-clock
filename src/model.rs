use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

use chrono::{
    DateTime, Datelike, Duration, Local, NaiveDate, NaiveDateTime, TimeZone, Timelike, Utc, Weekday,
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

fn merge_value<T: Clone + PartialEq>(base: &T, local: &T, remote: &T) -> T {
    if local == remote || remote == base {
        local.clone()
    } else {
        remote.clone()
    }
}

pub fn merge_semester(base: Option<&Semester>, local: &Semester, remote: &Semester) -> Semester {
    let Some(base) = base else {
        return if *remote == Semester::default() {
            local.clone()
        } else {
            remote.clone()
        };
    };
    Semester {
        name: merge_value(&base.name, &local.name, &remote.name),
        first_monday: merge_value(
            &base.first_monday,
            &local.first_monday,
            &remote.first_monday,
        ),
        campus: merge_value(&base.campus, &local.campus, &remote.campus),
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
            .or_else(|| self.start?.checked_add_signed(Duration::minutes(45)))
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_semester: Option<Semester>,
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

    pub fn update_semester(&mut self, semester: Semester) {
        if self.dav_verified && self.base_semester.is_none() && self.semester != semester {
            // Before the baseline was stored, successful sync adopted the remote semester.
            self.base_semester = Some(self.semester.clone());
        }
        self.semester = semester;
    }
}

pub fn toggle_task(entries: &mut Entries, id: Uuid) -> bool {
    let Some(next) = ({
        let Some(entry) = entries.get_mut(&id) else {
            return false;
        };
        entry.done = !entry.done;
        if entry.done {
            entry.repeat.zip(entry.start).map(|(repeat, start)| {
                let mut next = entry.clone();
                let next_start = repeat.next(start);
                next.start = Some(next_start);
                next.end = entry
                    .end
                    .and_then(|end| end.checked_add_signed(next_start - start));
                next.done = false;
                next.conflict = false;
                next
            })
        } else {
            None
        }
    }) else {
        return true;
    };
    let next_id = Uuid::new_v5(&id, b"course-clock:next-repeat");
    let already_exists = entries.iter().any(|(&candidate_id, candidate)| {
        candidate_id != id && !candidate.done && candidate == &next
    });
    if !already_exists {
        entries.entry(next_id).or_insert(next);
    }
    true
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

pub fn week_start(date: NaiveDate) -> NaiveDate {
    date - Duration::days(date.weekday().num_days_from_monday().into())
}

pub struct CourseSlot {
    pub id: Uuid,
    pub start: i32,
    pub end: i32,
    pub overlaps: bool,
}

pub fn week_courses(entries: &Entries, monday: NaiveDate) -> [Vec<CourseSlot>; 7] {
    const DAY_SECONDS: i32 = 86_400;
    let mut days: [Vec<CourseSlot>; 7] = std::array::from_fn(|_| Vec::new());
    for (&id, entry) in entries
        .iter()
        .filter(|(_, entry)| entry.kind == Kind::Course)
    {
        let (Some(start), Some(end)) = (entry.start, entry.course_end()) else {
            continue;
        };
        let (start, end) = (start.with_timezone(&Local), end.with_timezone(&Local));
        let first = (start.date_naive() - monday).num_days();
        let last = (end.date_naive() - monday).num_days();
        for day in first.max(0)..=last.min(6) {
            let start_second = if day == first {
                start.num_seconds_from_midnight() as i32
            } else {
                0
            };
            let end_second = if day == last {
                end.num_seconds_from_midnight() as i32
            } else {
                DAY_SECONDS
            };
            if day == last && day != first && end_second == 0 {
                continue;
            }
            days[day as usize].push(CourseSlot {
                id,
                start: start_second,
                end: end_second.max(start_second + 1),
                overlaps: false,
            });
        }
    }
    for slots in &mut days {
        slots.sort_by_key(|slot| (slot.start, slot.end, slot.id));
        let mut latest_end = 0;
        for index in 0..slots.len() {
            let next_start = slots.get(index + 1).map_or(DAY_SECONDS, |next| next.start);
            let slot = &mut slots[index];
            slot.overlaps = slot.start < latest_end || next_start < slot.end;
            latest_end = latest_end.max(slot.end);
        }
    }
    days
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

    fn task(repeat: RepeatRule) -> Entry {
        Entry {
            kind: Kind::Task,
            title: "周期任务".into(),
            teacher: String::new(),
            place: String::new(),
            start: Some("2026-11-18T09:00:00Z".parse().unwrap()),
            end: None,
            lead: Some(10),
            done: false,
            repeat: Some(repeat),
            conflict: false,
        }
    }

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
        let monday = week_start(NaiveDate::from_ymd_opt(2026, 11, 18).unwrap());
        let overlaps: BTreeSet<_> = week_courses(&entries, monday)
            .into_iter()
            .flatten()
            .filter(|slot| slot.overlaps)
            .map(|slot| slot.id)
            .collect();
        assert_eq!(overlaps, BTreeSet::from([first, second]));
    }

    #[test]
    fn overnight_course_overlaps_the_next_days_course() {
        let monday = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
        let local_time = |text| {
            local_datetime(NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").unwrap()).unwrap()
        };
        let mut entries = Entries::new();
        let overnight = Uuid::new_v4();
        let next_day = Uuid::new_v4();
        let mut overnight_entry = course("2026-11-17T15:30:00Z", "2026-11-17T16:30:00Z");
        overnight_entry.start = Some(local_time("2026-11-17 23:30"));
        overnight_entry.end = Some(local_time("2026-11-18 00:30"));
        entries.insert(overnight, overnight_entry);
        let mut next_day_entry = course("2026-11-18T01:00:00Z", "2026-11-18T02:00:00Z");
        next_day_entry.start = Some(local_time("2026-11-18 00:15"));
        next_day_entry.end = Some(local_time("2026-11-18 01:00"));
        entries.insert(next_day, next_day_entry);
        let overlaps: BTreeSet<_> = week_courses(&entries, monday)
            .into_iter()
            .flatten()
            .filter(|slot| slot.overlaps)
            .map(|slot| slot.id)
            .collect();
        assert_eq!(overlaps, BTreeSet::from([overnight, next_day]));
    }

    #[test]
    fn repeating_task_completion_is_atomic_and_idempotent() {
        let id = Uuid::new_v4();
        let mut entries = Entries::from([(id, task(RepeatRule::Daily))]);
        assert!(toggle_task(&mut entries, id));
        let next = Uuid::new_v5(&id, b"course-clock:next-repeat");
        let next_entry = entries[&next].clone();
        assert!(toggle_task(&mut entries, id));
        assert!(toggle_task(&mut entries, id));
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[&next], next_entry);
    }

    #[test]
    fn repeating_task_completion_reuses_legacy_next_occurrence() {
        let id = Uuid::new_v4();
        let legacy_next_id = Uuid::new_v4();
        let mut entries = Entries::from([(id, task(RepeatRule::Daily))]);
        let mut legacy_next = task(RepeatRule::Daily);
        legacy_next.start = Some(RepeatRule::Daily.next(entries[&id].start.unwrap()));
        entries.insert(legacy_next_id, legacy_next.clone());
        toggle_task(&mut entries, id);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[&legacy_next_id], legacy_next);
    }

    #[test]
    fn semester_merge_keeps_local_changes_since_the_baseline() {
        let base = Semester::default();
        let remote = Semester {
            name: "远端学期".into(),
            first_monday: None,
            campus: base.campus.clone(),
        };
        let local = Semester {
            name: "本地学期".into(),
            first_monday: NaiveDate::from_ymd_opt(2026, 8, 31),
            campus: base.campus.clone(),
        };
        assert_eq!(
            merge_semester(Some(&base), &local, &remote),
            Semester {
                name: remote.name,
                first_monday: local.first_monday,
                campus: base.campus.clone(),
            }
        );
        assert_eq!(merge_semester(Some(&base), &local, &base), local);
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
