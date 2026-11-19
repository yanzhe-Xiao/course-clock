use std::{
    collections::{BTreeMap, BTreeSet},
    ptr::null_mut,
    sync::LazyLock,
};

use chrono::{Datelike, Duration, Local, NaiveDate, NaiveDateTime, Timelike};
use uuid::Uuid;
use windows_sys::Win32::{
    Foundation::{HWND, RECT},
    Graphics::{
        Gdi::*,
        GdiPlus::{
            CombineModeReplace, FillModeAlternate, FlushIntentionFlush, GdipAddPathArc,
            GdipAddPathLine, GdipAddPathRectangle, GdipClosePathFigure, GdipCreateFromHDC,
            GdipCreateLineBrushFromRectI, GdipCreatePath, GdipCreatePen1, GdipCreateSolidFill,
            GdipDeleteBrush, GdipDeleteGraphics, GdipDeletePath, GdipDeletePen, GdipDrawEllipse,
            GdipDrawLine, GdipDrawPath, GdipFillEllipse, GdipFillPath, GdipFlush, GdipResetClip,
            GdipSetClipRectI, GdipSetCompositingMode, GdipSetPenDashStyle, GdipSetPenLineCap197819,
            GdipSetPenLineJoin, GdipSetPixelOffsetMode, GdipSetSmoothingMode, GdiplusStartup,
            GdiplusStartupInput, GpBrush, GpGraphics, GpPath, LineCapRound, LineJoinRound,
            LinearGradientModeVertical, PixelOffsetModeHalf, Rect, SmoothingModeAntiAlias,
            UnitPixel,
        },
    },
    UI::WindowsAndMessaging::GetClientRect,
};

use crate::{
    model::{Entry, Kind},
    win::wide,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Today,
    Week,
    Tasks,
    Conflicts,
    Import,
    Preview,
    Settings,
    Course(Uuid),
    Edit(Option<Uuid>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Page(Page),
    Back,
    OpenPicker(usize, PickerKind),
    ClearDate(usize),
    PickerMonth(i32),
    PickerStep(i32),
    PickerDate(NaiveDate),
    PickerHour(u32),
    PickerMinute(u32),
    ConfirmPicker,
    ClosePicker,
    NewCourse,
    Day(NaiveDate),
    Choose(usize, usize),
    PickFile,
    ParseImport,
    CommitImport,
    SaveEntry,
    ToggleEntry(Uuid),
    DeleteEntry(Uuid),
    SaveDav,
    Disconnect,
    Sync,
    Backup,
    Restore,
    ResolveConflict(Uuid),
    Startup,
    TestToast,
    OpenReminder,
    Snooze,
    DoneReminder,
    Dismiss,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PickerKind {
    Date,
    Time,
}

impl PickerKind {
    pub fn field(self, index: usize) -> Bounds {
        let y = 272 + index as i32 * 62;
        match self {
            Self::Date => Bounds(228, y, 183, 38),
            Self::Time => Bounds(424, y, 111, 38),
        }
    }

    pub fn popup_size(self) -> (i32, i32) {
        match self {
            Self::Date => (312, 316),
            Self::Time => (324, 350),
        }
    }
}

#[derive(Clone, Copy)]
pub struct EntryTime {
    pub value: NaiveDateTime,
    pub enabled: bool,
}

#[derive(Clone, Copy)]
pub struct Picker {
    pub index: usize,
    pub kind: PickerKind,
    pub value: NaiveDateTime,
}

#[derive(Clone, Copy)]
pub struct Bounds(pub i32, pub i32, pub i32, pub i32);

impl Bounds {
    pub fn rect(self, dpi: i32) -> RECT {
        let Self(x, y, w, h) = self;
        let s = |value| scale(value, dpi);
        RECT {
            left: s(x),
            top: s(y),
            right: s(x + w),
            bottom: s(y + h),
        }
    }
}

pub struct Hit(pub RECT, pub Action);

impl Hit {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.0.left && x < self.0.right && y >= self.0.top && y < self.0.bottom
    }
}

pub fn scale(value: i32, dpi: i32) -> i32 {
    (value * dpi + 48 * value.signum()) / 96
}

pub struct Scrollbar {
    pub track: RECT,
    pub thumb: RECT,
    pub offset: i32,
    pub limit: i32,
    pub page_height: i32,
}

#[derive(Clone, Copy)]
pub struct Palette {
    paper: u32,
    rail: u32,
    pub surface: u32,
    pub field: u32,
    line: u32,
    grid: u32,
    pub text: u32,
    muted: u32,
    accent: u32,
    accent_ink: u32,
    tint: u32,
    hero: u32,
    hero_deep: u32,
    hero_soft: u32,
    danger: u32,
    dark: bool,
}

impl Palette {
    pub fn new(dark: bool) -> Self {
        if dark {
            Self {
                paper: 0x14181a,
                rail: 0x1a2023,
                surface: 0x222a2d,
                field: 0x2b3437,
                line: 0x354044,
                grid: 0x2b3336,
                text: 0xedf3f3,
                muted: 0x9cb0b1,
                accent: 0x63d9bd,
                accent_ink: 0x0d2a22,
                tint: 0x1f3d36,
                hero: 0x10403a,
                hero_deep: 0x0b2d29,
                hero_soft: 0xb9e6da,
                danger: 0xf2777a,
                dark,
            }
        } else {
            Self {
                paper: 0xf7f8f6,
                rail: 0xeef1ee,
                surface: 0xffffff,
                field: 0xf2f5f2,
                line: 0xdfe5e0,
                grid: 0xe8ede9,
                text: 0x18211f,
                muted: 0x62736d,
                accent: 0x116b52,
                accent_ink: 0xffffff,
                tint: 0xe4f2ec,
                hero: 0x0f5c48,
                hero_deep: 0x0a4436,
                hero_soft: 0xb7dfd0,
                danger: 0xb8484c,
                dark,
            }
        }
    }

    fn course(self, color: u32) -> u32 {
        if self.dark {
            blend(self.surface, color, 25)
        } else {
            color
        }
    }
}

// Color Hunt collections: Light Sunset, Light Green, and Pastel Purple.
const COURSE_COLORS: [u32; 36] = [
    0xffc7ea, 0xfddbbb, 0xf8f7ba, 0xc5d89d, 0xcfecf3, 0xaee2ff, 0xf0c1e1, 0xfee2ad, 0xfff0be,
    0xbde3c3, 0xa6d6d6, 0xc5d3e8, 0xf5d2d2, 0xffd6a6, 0xfdf7c3, 0xa8df8e, 0xaae3e2, 0xc0c9ee,
    0xffcacc, 0xffdab3, 0xfff9bf, 0xd9e9cf, 0xcdf0ea, 0xcdc1ff, 0xffcccc, 0xffc7a7, 0xf7db91,
    0xc4f7ca, 0xb9f3e4, 0xdbc4f0, 0xf0a8d0, 0xffe6c9, 0xf4f8d3, 0xc9e9d2, 0xb4d3d9, 0xb5baff,
];

pub fn course_colors<'a>(titles: impl Iterator<Item = &'a str>) -> BTreeMap<&'a str, u32> {
    let names: BTreeSet<_> = titles.map(str::trim).collect();
    let mut used = BTreeSet::new();
    // Walk the light RGB cube in a fixed order when the curated set is exhausted.
    const CHANNEL: u32 = 96;
    const CUBE: u32 = CHANNEL * CHANNEL * CHANNEL;
    let mut extra = (0..CUBE).map(|index| {
        let n = (u64::from(index) * 37_561 % u64::from(CUBE)) as u32;
        ((160 + n / (CHANNEL * CHANNEL)) << 16)
            | ((160 + n / CHANNEL % CHANNEL) << 8)
            | (160 + n % CHANNEL)
    });
    names
        .into_iter()
        .map(|name| {
            let hash = name.bytes().fold(0_u32, |hash, byte| {
                hash.wrapping_mul(33).wrapping_add(u32::from(byte))
            }) as usize;
            let start = hash % COURSE_COLORS.len();
            let color = (0..COURSE_COLORS.len())
                .map(|offset| COURSE_COLORS[(start + offset) % COURSE_COLORS.len()])
                .find(|&color| used.insert(color))
                .or_else(|| extra.find(|&color| used.insert(color)))
                .unwrap_or(COURSE_COLORS[start]);
            (name, color)
        })
        .collect()
}

fn blend(from: u32, to: u32, percent: u32) -> u32 {
    let channel = |shift: u32| -> u32 {
        let a = (from >> shift) & 255_u32;
        let b = (to >> shift) & 255_u32;
        (a * (100 - percent) + b * percent + 50) / 100
    };
    channel(16) << 16 | channel(8) << 8 | channel(0)
}

pub struct View<'a> {
    pub page: Page,
    pub active_page: Page,
    pub day: NaiveDate,
    pub rows: &'a [(Uuid, &'a Entry)],
    pub course_colors: &'a BTreeMap<&'a str, u32>,
    pub next: Option<&'a Entry>,
    pub scroll: usize,
    pub week_scroll: Option<i32>,
    pub detail_scroll: i32,
    pub course: Option<&'a Entry>,
    pub status: &'a str,
    pub sync_status: &'a str,
    pub connected: bool,
    pub startup: bool,
    pub editing_course: bool,
    pub entry_times: Option<[EntryTime; 2]>,
    pub picker_focus: Option<Action>,
    pub fields: &'a [Field],
    pub hover: Option<Action>,
    pub semester: &'a str,
}

pub struct Field {
    pub label: &'static str,
    pub bounds: Bounds,
    pub options: Vec<&'static str>,
    pub selected: usize,
    pub slot: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Icon {
    Today,
    Week,
    Tasks,
    Alert,
    Import,
    Settings,
}

pub fn draw_main(
    window: HWND,
    dpi: i32,
    palette: Palette,
    view: View<'_>,
) -> (Vec<Hit>, Option<Scrollbar>) {
    let mut f = Frame::new(window, dpi, palette, view.hover);
    let p = f.p;
    let w = f.width;
    let h = f.height;
    f.block(Bounds(0, 0, w, h), p.paper);
    f.block(Bounds(0, 0, 196, h), p.rail);
    f.block(Bounds(195, 0, 1, h), p.line);

    f.round(Bounds(22, 26, 34, 34), 10, p.accent, p.accent);
    f.clock(39, 43, 9, p.accent_ink);
    f.text(Bounds(66, 24, 120, 40), "课时钟", 21, 700, p.text, false);

    let items = [
        (Page::Today, "今天", Icon::Today),
        (Page::Week, "周课表", Icon::Week),
        (Page::Tasks, "待办", Icon::Tasks),
        (Page::Conflicts, "冲突", Icon::Alert),
        (Page::Import, "导入课表", Icon::Import),
        (Page::Settings, "设置", Icon::Settings),
    ];
    for (index, (page, label, icon)) in items.into_iter().enumerate() {
        let y = 100 + index as i32 * 46 + if index >= 4 { 20 } else { 0 };
        let active = view.active_page == page;
        let bounds = Bounds(14, y, 168, 40);
        let action = Action::Page(page);
        if active {
            f.round(bounds, 9, p.surface, p.surface);
        } else if f.hover == Some(action) {
            f.round(bounds, 9, p.tint, p.tint);
        }
        f.icon(
            Bounds(31, y + 12, 17, 17),
            icon,
            if active { p.accent } else { p.muted },
            active,
        );
        f.text(
            Bounds(58, y, 110, 40),
            label,
            14,
            if active { 700 } else { 500 },
            if active { p.text } else { p.muted },
            false,
        );
        f.hit(bounds, action);
    }
    f.block(Bounds(24, h - 74, 148, 1), p.line);
    f.round(Bounds(24, h - 47, 7, 7), 7, p.accent, p.accent);
    f.text(
        Bounds(38, h - 56, 146, 24),
        if view.connected {
            "WebDAV 已配置"
        } else {
            "本地运行"
        },
        11,
        500,
        p.muted,
        false,
    );

    let x = 228;
    let width = w - x - 32;
    let today = Local::now().date_naive();
    let title = match view.page {
        Page::Today => "今天",
        Page::Week => "周课表",
        Page::Tasks => "待办",
        Page::Conflicts => "冲突处理",
        Page::Import => "导入课表",
        Page::Preview => "导入预览",
        Page::Settings => "设置",
        Page::Course(_) => "课程详情",
        Page::Edit(id) => match (id.is_some(), view.editing_course) {
            (false, false) => "新建待办",
            (true, false) => "编辑待办",
            (false, true) => "新建课程",
            (true, true) => "编辑课程",
        },
    };
    f.text(
        Bounds(x + 1, 28, width - 240, 22),
        &day_label(today),
        12,
        500,
        p.muted,
        false,
    );
    f.text(
        Bounds(x, 50, width - 200, 44),
        title,
        29,
        700,
        p.text,
        false,
    );

    let header_action = match view.page {
        Page::Today | Page::Tasks => Some(("+ 新建待办", Action::Page(Page::Edit(None)))),
        Page::Week => Some(("+ 新建课程", Action::NewCourse)),
        Page::Preview => Some(("确认导入", Action::CommitImport)),
        Page::Course(id) if view.course.is_some() => {
            Some(("编辑课程", Action::Page(Page::Edit(Some(id)))))
        }
        _ => None,
    };
    if let Some((label, action)) = header_action {
        f.button(Bounds(w - 155, 52, 123, 36), label, action, true);
    }
    if matches!(view.page, Page::Course(_))
        || matches!(view.page, Page::Edit(_)) && view.editing_course
    {
        let back_x = if header_action.is_some() {
            w - 243
        } else {
            w - 108
        };
        f.button(Bounds(back_x, 52, 76, 36), "返回", Action::Back, false);
    }

    let mut scrollbar = None;
    match view.page {
        Page::Today => {
            draw_hero(&mut f, x, width, view.next);
            f.text(
                Bounds(x + 1, 252, width, 28),
                "今天的安排",
                17,
                700,
                p.text,
                false,
            );
            draw_rows(&mut f, x, 292, width, &view, "今天还没有课程或待办");
        }
        Page::Week => {
            let monday =
                view.day - Duration::days(view.day.weekday().num_days_from_monday().into());
            let sunday = monday + Duration::days(6);
            f.text(
                Bounds(x, 103, width - 226, 27),
                &format!("{}  –  {}", monday.format("%m.%d"), sunday.format("%m.%d")),
                19,
                600,
                p.text,
                false,
            );
            for (dx, caption, day) in [
                (206, "上周", view.day - Duration::days(7)),
                (136, "本周", today),
                (66, "下周", view.day + Duration::days(7)),
            ] {
                f.button(
                    Bounds(x + width - dx, 111, 64, 32),
                    caption,
                    Action::Day(day),
                    false,
                );
            }
            for (index, label) in ["周一", "周二", "周三", "周四", "周五", "周六", "周日"]
                .into_iter()
                .enumerate()
            {
                let date = monday + Duration::days(index as i64);
                let (left, day_width) = week_column(x, width, index as i32);
                let bounds = Bounds(left + 3, 158, day_width - 6, 44);
                let action = Action::Day(date);
                let selected = date == view.day;
                let (background, stroke) = if selected {
                    (p.tint, p.accent)
                } else if f.hover == Some(action) {
                    (p.tint, p.tint)
                } else {
                    (p.surface, p.line)
                };
                f.round(bounds, 10, background, stroke);
                f.text(
                    Bounds(bounds.0, bounds.1 + 3, bounds.2, 16),
                    label,
                    11,
                    500,
                    if selected { p.accent } else { p.muted },
                    true,
                );
                f.text(
                    Bounds(bounds.0, bounds.1 + 18, bounds.2, 22),
                    &date.day().to_string(),
                    17,
                    700,
                    if selected { p.accent } else { p.text },
                    true,
                );
                f.hit(bounds, action);
            }
            f.text(
                Bounds(x + 1, 127, width - 227, 20),
                view.semester,
                11,
                500,
                p.muted,
                false,
            );
            scrollbar = draw_week_grid(&mut f, x, 212, width, &view, monday, today);
        }
        Page::Tasks => {
            let remaining = view.rows.iter().filter(|(_, entry)| !entry.done).count();
            f.text(
                Bounds(x + 1, 112, width, 26),
                &format!("{remaining} 项待完成"),
                13,
                500,
                p.muted,
                false,
            );
            draw_rows(&mut f, x, 148, width, &view, "还没有待办");
        }
        Page::Conflicts => {
            f.text(
                Bounds(x + 1, 112, width - 130, 26),
                &format!("{} 条记录需要处理", view.rows.len()),
                13,
                500,
                p.muted,
                false,
            );
            draw_conflicts(&mut f, x, 148, width, &view);
        }
        Page::Import => {
            f.text(
                Bounds(x + 1, 104, width, 26),
                "支持西工大 CSV、教务系统 HTML 和日历 ICS",
                13,
                500,
                p.muted,
                false,
            );
            draw_fields(&mut f, view.fields);
            f.button(
                Bounds(x + 359, 170, 100, 38),
                "浏览…",
                Action::PickFile,
                false,
            );
            f.text(
                Bounds(x + 1, 380, width, 26),
                "导入 CSV 或 HTML 时，需要填写第一教学周和校区作息。",
                12,
                400,
                p.muted,
                false,
            );
            f.button(
                Bounds(x, 420, 138, 40),
                "生成预览",
                Action::ParseImport,
                true,
            );
            draw_drop_hint(&mut f, x, 490, width);
        }
        Page::Preview => {
            f.text(
                Bounds(x + 1, 112, width - 160, 26),
                &format!("共 {} 次课程 · 请核对日期和教室", view.rows.len()),
                13,
                500,
                p.muted,
                false,
            );
            draw_rows(&mut f, x, 148, width, &view, "预览没有课程");
        }
        Page::Settings => {
            f.text(
                Bounds(x + 1, 100, width, 27),
                "WebDAV 双机同步",
                16,
                700,
                p.text,
                false,
            );
            draw_fields(&mut f, view.fields);
            f.text(
                Bounds(x + 1, 352, width, 26),
                view.sync_status,
                12,
                400,
                p.muted,
                false,
            );
            f.button(Bounds(x, 388, 118, 38), "保存并同步", Action::SaveDav, true);
            if view.connected {
                f.button(
                    Bounds(x + 130, 388, 100, 38),
                    "立即同步",
                    Action::Sync,
                    false,
                );
                f.button(
                    Bounds(x + 242, 388, 100, 38),
                    "断开连接",
                    Action::Disconnect,
                    false,
                );
            }
            f.block(Bounds(x, 446, width, 1), p.line);
            f.text(
                Bounds(x + 1, 462, 200, 29),
                "Windows 提醒",
                16,
                700,
                p.text,
                false,
            );
            f.toggle(
                Bounds(x, 500, 168, 38),
                "登录时启动",
                view.startup,
                Action::Startup,
            );
            f.button(
                Bounds(x + 180, 500, 112, 38),
                "试播提醒",
                Action::TestToast,
                false,
            );
            f.button(Bounds(x, 552, 116, 38), "导出备份", Action::Backup, false);
            f.button(
                Bounds(x + 128, 552, 116, 38),
                "恢复备份",
                Action::Restore,
                false,
            );
        }
        Page::Course(_) => {
            scrollbar = draw_course_detail(&mut f, x, width, view.course, view.detail_scroll);
        }
        Page::Edit(id) => {
            draw_fields(&mut f, view.fields);
            if let Some(times) = view.entry_times {
                draw_entry_times(&mut f, times, view.editing_course, view.picker_focus);
            }
            let (note_y, button_y) = if view.editing_course {
                (382, 416)
            } else {
                (442, 476)
            };
            f.text(
                Bounds(x, note_y, width, 26),
                "提前分钟留空可关闭提醒。",
                12,
                400,
                p.muted,
                false,
            );
            f.button(
                Bounds(x, button_y, 104, 38),
                "保存",
                Action::SaveEntry,
                true,
            );
            if let Some(id) = id {
                if !view.editing_course {
                    f.button(
                        Bounds(x + 116, button_y, 124, 38),
                        "切换完成状态",
                        Action::ToggleEntry(id),
                        false,
                    );
                }
                f.button(
                    Bounds(x + width - 96, button_y, 96, 38),
                    "删除",
                    Action::DeleteEntry(id),
                    false,
                );
            }
        }
    }
    if !view.status.is_empty() {
        let status_width = if matches!(
            view.page,
            Page::Today | Page::Week | Page::Tasks | Page::Preview
        ) {
            width - 116
        } else {
            width
        };
        f.block(Bounds(x, h - 34, status_width, 34), p.paper);
        f.round(Bounds(x, h - 25, 5, 5), 5, p.accent, p.accent);
        f.text(
            Bounds(x + 12, h - 33, status_width - 12, 26),
            view.status,
            12,
            500,
            p.accent,
            false,
        );
    }
    (f.finish(), scrollbar)
}

fn draw_fields(f: &mut Frame, fields: &[Field]) {
    for field in fields {
        let Bounds(x, y, width, height) = field.bounds;
        f.text(
            Bounds(x + 1, y - 24, width, 20),
            field.label,
            11,
            600,
            f.p.muted,
            false,
        );
        if field.options.is_empty() {
            f.round(field.bounds, 9, f.p.field, f.p.field);
            continue;
        }
        let gap = 6;
        let count = field.options.len() as i32;
        let segment = (width - gap * (count - 1)) / count;
        for (option, label) in field.options.iter().enumerate() {
            let bx = x + option as i32 * (segment + gap);
            let b = Bounds(bx, y, segment.min(width - (bx - x)), height);
            let selected = field.selected == option;
            f.round(
                b,
                9,
                if selected { f.p.accent } else { f.p.field },
                if selected { f.p.accent } else { f.p.field },
            );
            f.text(
                b,
                label,
                12,
                if selected { 700 } else { 500 },
                if selected { f.p.accent_ink } else { f.p.text },
                true,
            );
            f.hit(b, Action::Choose(field.slot, option));
        }
    }
}

fn draw_entry_times(f: &mut Frame, times: [EntryTime; 2], course: bool, focus: Option<Action>) {
    let p = f.p;
    let start_label = if course {
        "上课日期"
    } else {
        "截止日期（可选）"
    };
    for (index, (time, date_label)) in times
        .into_iter()
        .zip([start_label, "结束日期（可选）"])
        .enumerate()
    {
        for kind in [PickerKind::Date, PickerKind::Time] {
            let bounds @ Bounds(x, y, width, height) = kind.field(index);
            let date = kind == PickerKind::Date;
            let clear = date && time.enabled && (index == 1 || !course);
            let enabled = (index == 0 || times[0].enabled) && (date || time.enabled);
            let action = Action::OpenPicker(index, kind);
            let hovered = f.hover == Some(action);
            let border = if focus == Some(action) {
                p.accent
            } else {
                p.field
            };
            f.text(
                Bounds(x + 1, y - 24, width, 20),
                if date { date_label } else { "时刻" },
                11,
                600,
                p.muted,
                false,
            );
            f.round(
                bounds,
                9,
                if hovered { p.tint } else { p.field },
                if hovered && focus != Some(action) {
                    p.tint
                } else {
                    border
                },
            );
            let ink = if enabled { p.accent } else { p.muted };
            if date {
                f.icon(Bounds(x + 11, y + 12, 14, 14), Icon::Week, ink, false);
            } else {
                f.clock(x + 17, y + 19, 7, ink);
            }
            let value = if time.enabled {
                time.value
                    .format(if date { "%Y-%m-%d" } else { "%H:%M" })
                    .to_string()
            } else if date {
                "选择日期".into()
            } else {
                "—".into()
            };
            f.text(
                Bounds(x + 34, y, width - if clear { 68 } else { 52 }, height),
                &value,
                14,
                500,
                if time.enabled { p.text } else { p.muted },
                false,
            );
            if enabled {
                f.text(
                    Bounds(x + width - if clear { 52 } else { 24 }, y, 20, height),
                    "⌄",
                    16,
                    500,
                    p.muted,
                    true,
                );
                f.hit(bounds, action);
            }
            if clear {
                let action = Action::ClearDate(index);
                let button = Bounds(x + width - 28, y + 6, 22, 26);
                if f.hover == Some(action) {
                    f.round(button, 6, p.surface, p.surface);
                }
                f.text(button, "×", 16, 500, p.muted, true);
                f.hit(button, action);
            }
        }
    }
}

pub fn draw_picker(
    window: HWND,
    dpi: i32,
    palette: Palette,
    picker: Picker,
    hover: Option<Action>,
) -> Vec<Hit> {
    let mut f = Frame::new(window, dpi, palette, hover);
    let p = f.p;
    let (w, h) = picker.kind.popup_size();
    f.block(Bounds(0, 0, w, h), p.paper);
    f.round(Bounds(0, 0, w - 1, h - 1), 14, p.surface, p.line);
    match picker.kind {
        PickerKind::Date => draw_date_picker(&mut f, picker.value.date(), w),
        PickerKind::Time => draw_time_picker(&mut f, picker.value, w),
    }
    f.finish()
}

fn draw_date_picker(f: &mut Frame, selected: NaiveDate, width: i32) {
    let p = f.p;
    f.button(Bounds(14, 13, 32, 30), "‹", Action::PickerMonth(-1), false);
    f.button(
        Bounds(width - 46, 13, 32, 30),
        "›",
        Action::PickerMonth(1),
        false,
    );
    f.text(
        Bounds(50, 10, width - 100, 36),
        &format!("{} 年 {} 月", selected.year(), selected.month()),
        17,
        700,
        p.text,
        true,
    );
    let labels = ["一", "二", "三", "四", "五", "六", "日"];
    for (day, label) in labels.into_iter().enumerate() {
        f.text(
            Bounds(16 + day as i32 * 40, 50, 39, 20),
            label,
            11,
            600,
            p.muted,
            true,
        );
    }
    let first = selected.with_day(1).unwrap();
    let weekday = i64::from(first.weekday().num_days_from_monday());
    let today = Local::now().date_naive();
    for day in 0..42 {
        let Some(date) = first.checked_add_signed(Duration::days(i64::from(day) - weekday)) else {
            continue;
        };
        let bounds = Bounds(16 + day % 7 * 40, 73 + day / 7 * 31, 39, 29);
        let action = Action::PickerDate(date);
        let selected_day = date == selected;
        let color = if selected_day {
            f.round(bounds, 8, p.accent, p.accent);
            p.accent_ink
        } else if date == today || f.hover == Some(action) {
            f.round(bounds, 8, p.tint, p.tint);
            p.accent
        } else if date.month() == selected.month() {
            p.text
        } else {
            p.muted
        };
        f.text(
            bounds,
            &date.day().to_string(),
            13,
            if selected_day { 700 } else { 500 },
            color,
            true,
        );
        f.hit(bounds, action);
    }
    f.block(Bounds(16, 270, width - 32, 1), p.line);
    f.button(
        Bounds(16, 278, 68, 28),
        "今天",
        Action::PickerDate(today),
        false,
    );
    f.button(
        Bounds(width - 84, 278, 68, 28),
        "取消",
        Action::ClosePicker,
        false,
    );
}

fn draw_time_picker(f: &mut Frame, value: NaiveDateTime, width: i32) {
    let p = f.p;
    f.text(Bounds(16, 11, 150, 34), "选择时刻", 17, 700, p.text, false);
    let preview = Bounds(width - 91, 12, 74, 32);
    f.round(preview, 9, p.tint, p.tint);
    f.text(
        preview,
        &value.format("%H:%M").to_string(),
        18,
        700,
        p.accent,
        true,
    );
    f.text(Bounds(16, 49, 70, 22), "小时", 12, 600, p.muted, false);
    for hour in 0..24 {
        let index = hour as i32;
        let bounds = Bounds(16 + index % 6 * 49, 75 + index / 6 * 28, 45, 24);
        draw_time_option(
            f,
            bounds,
            hour,
            hour == value.hour(),
            Action::PickerHour(hour),
        );
    }
    f.text(Bounds(16, 189, 70, 28), "分钟", 12, 600, p.muted, false);
    f.button(
        Bounds(width - 100, 191, 27, 24),
        "−",
        Action::PickerStep(-1),
        false,
    );
    f.text(
        Bounds(width - 73, 191, 28, 24),
        &format!("{:02}", value.minute()),
        12,
        700,
        p.text,
        true,
    );
    f.button(
        Bounds(width - 45, 191, 27, 24),
        "+",
        Action::PickerStep(1),
        false,
    );
    for index in 0..12 {
        let minute = index as u32 * 5;
        let bounds = Bounds(16 + index % 6 * 49, 222 + index / 6 * 28, 45, 24);
        draw_time_option(
            f,
            bounds,
            minute,
            minute == value.minute(),
            Action::PickerMinute(minute),
        );
    }
    f.block(Bounds(16, 290, width - 32, 1), p.line);
    f.button(
        Bounds(width - 172, 302, 70, 34),
        "取消",
        Action::ClosePicker,
        false,
    );
    f.button(
        Bounds(width - 90, 302, 74, 34),
        "确定",
        Action::ConfirmPicker,
        true,
    );
}

fn draw_time_option(f: &mut Frame, bounds: Bounds, value: u32, selected: bool, action: Action) {
    let p = f.p;
    let active = selected || f.hover == Some(action);
    let fill = if active { p.tint } else { p.field };
    f.round(bounds, 6, fill, if selected { p.accent } else { fill });
    f.text(
        bounds,
        &format!("{value:02}"),
        12,
        if active { 700 } else { 500 },
        if active { p.accent } else { p.text },
        true,
    );
    f.hit(bounds, action);
}

fn draw_drop_hint(f: &mut Frame, x: i32, y: i32, width: i32) {
    let p = f.p;
    let bounds = Bounds(x, y, width, 84);
    f.dashed(bounds, 10, p.line);
    f.icon(Bounds(x + 18, y + 32, 20, 20), Icon::Import, p.muted, false);
    f.text(
        Bounds(x + 50, y + 20, width - 70, 24),
        "也可以把 CSV、HTML 或 ICS 文件直接拖进窗口",
        12,
        500,
        p.text,
        false,
    );
    f.text(
        Bounds(x + 50, y + 44, width - 70, 22),
        "拖入后会立即跳到导入页，并自动填入文件路径。",
        11,
        400,
        p.muted,
        false,
    );
}

fn day_label(day: NaiveDate) -> String {
    let weekday = [
        "星期一",
        "星期二",
        "星期三",
        "星期四",
        "星期五",
        "星期六",
        "星期日",
    ];
    format!(
        "{}月{}日  ·  {}",
        day.month(),
        day.day(),
        weekday[day.weekday().num_days_from_monday() as usize]
    )
}

fn draw_hero(f: &mut Frame, x: i32, width: i32, next: Option<&Entry>) {
    let p = f.p;
    let bounds = Bounds(x, 106, width, 126);
    f.round_gradient(bounds, 16, p.hero, p.hero_deep);
    f.clock(x + width - 56, 169, 30, 0x3f7f6d);
    f.text(
        Bounds(x + 22, 120, width - 120, 22),
        "接下来的课程",
        11,
        600,
        p.hero_soft,
        false,
    );
    if let Some(entry) = next {
        f.text(
            Bounds(x + 21, 143, width - 140, 36),
            &entry.title,
            22,
            700,
            0xffffff,
            false,
        );
        let date = entry.start.unwrap().with_timezone(&Local).date_naive();
        let day = if date == Local::now().date_naive() {
            "今天".into()
        } else {
            date.format("%m/%d").to_string()
        };
        let detail = entry.detail_label();
        let text = if detail.is_empty() {
            format!("{day}  {}", entry.time_label())
        } else {
            format!("{day}  {}    ·    {detail}", entry.time_label())
        };
        f.text(
            Bounds(x + 22, 186, width - 140, 26),
            &text,
            12,
            500,
            p.hero_soft,
            false,
        );
    } else {
        f.text(
            Bounds(x + 21, 143, width - 140, 36),
            "暂无后续课程",
            22,
            700,
            0xffffff,
            false,
        );
        f.button(
            Bounds(x + 22, 185, 92, 28),
            "导入课表",
            Action::Page(Page::Import),
            true,
        );
    }
}

const SECONDS_PER_HOUR: i32 = 3600;
const DAY_SECONDS: i32 = 24 * SECONDS_PER_HOUR;
const HOUR_HEIGHT: i32 = 80;
const TIME_GUTTER: i32 = 44;
const SCROLL_GUTTER: i32 = 16;

fn week_column(x: i32, width: i32, day: i32) -> (i32, i32) {
    let grid_width = width - TIME_GUTTER - SCROLL_GUTTER;
    let offset = grid_width * day / 7;
    (
        x + TIME_GUTTER + offset,
        grid_width * (day + 1) / 7 - offset,
    )
}

struct WeekSlot<'a> {
    id: Uuid,
    entry: &'a Entry,
    start: i32,
    end: i32,
    lane: i32,
}

fn week_slots<'a>(rows: &[(Uuid, &'a Entry)], monday: NaiveDate) -> [Vec<WeekSlot<'a>>; 7] {
    let mut days: [Vec<_>; 7] = std::array::from_fn(|_| Vec::new());
    for &(id, entry) in rows {
        let (Some(start), Some(end)) = (entry.start, entry.course_end()) else {
            continue;
        };
        let start = start.with_timezone(&Local);
        let end = end.with_timezone(&Local);
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
            days[day as usize].push(WeekSlot {
                id,
                entry,
                start: start_second,
                end: end_second.max(start_second + 1),
                lane: 0,
            });
        }
    }
    for slots in &mut days {
        slots.sort_by_key(|slot| (slot.start, slot.end));
    }
    days
}

struct WeekCard<'a> {
    id: Uuid,
    x: i32,
    width: i32,
    padding: i32,
    start: i32,
    end: i32,
    color: u32,
    conflict: bool,
    labels: [(&'a str, i32, i32, i32); 3],
}

impl WeekCard<'_> {
    fn min_height(&self) -> i32 {
        let text_height: i32 = self
            .labels
            .iter()
            .filter(|label| label.3 != 0)
            .map(|label| label.3 + 1)
            .sum();
        if text_height == 0 {
            1
        } else {
            text_height + 12
        }
    }
}

fn week_cards<'a>(
    f: &mut Frame,
    x: i32,
    width: i32,
    view: &View<'a>,
    monday: NaiveDate,
) -> Vec<WeekCard<'a>> {
    let mut cards = Vec::new();
    for (day, mut slots) in week_slots(view.rows, monday).into_iter().enumerate() {
        let (left, width) = week_column(x, width, day as i32);
        let mut group_start = 0;
        let mut group_end = 0;
        let mut lane_ends = Vec::new();
        for index in 0..=slots.len() {
            if index == slots.len() || slots[index].start >= group_end {
                let lanes = lane_ends.len() as i32;
                for slot in &slots[group_start..index] {
                    let lane_left = (width - 4) * slot.lane / lanes;
                    let lane_right = (width - 4) * (slot.lane + 1) / lanes;
                    let width = (lane_right - lane_left - 2).max(1);
                    let padding = if width >= 40 {
                        5
                    } else {
                        (width / 10).clamp(1, 3)
                    };
                    let text_width = width - 2 * padding;
                    let title = slot.entry.title.trim();
                    let (title_size, detail_size) = if width < 40 { (10, 9) } else { (11, 10) };
                    let labels = [
                        (title, title_size, 700),
                        (slot.entry.teacher.trim(), detail_size, 500),
                        (classroom_label(&slot.entry.place), detail_size, 500),
                    ]
                    .map(|(text, size, weight)| {
                        let height = if text.is_empty() || text_width < size {
                            0
                        } else {
                            f.wrapped_height(text_width, text, size, weight)
                        };
                        (text, size, weight, height)
                    });
                    cards.push(WeekCard {
                        id: slot.id,
                        x: left + 2 + lane_left,
                        width,
                        padding,
                        start: slot.start,
                        end: slot.end,
                        color: view
                            .course_colors
                            .get(title)
                            .copied()
                            .unwrap_or(COURSE_COLORS[0]),
                        conflict: lanes > 1 || slot.entry.conflict,
                        labels,
                    });
                }
                group_start = index;
                lane_ends.clear();
                group_end = 0;
            }
            let Some(slot) = slots.get_mut(index) else {
                break;
            };
            let lane = lane_ends.iter().position(|&end| end <= slot.start);
            slot.lane = lane.unwrap_or(lane_ends.len()) as i32;
            if let Some(lane) = lane {
                lane_ends[lane] = slot.end;
            } else {
                lane_ends.push(slot.end);
            }
            group_end = group_end.max(slot.end);
        }
    }
    cards
}

fn week_time_marks(
    start: i32,
    end: i32,
    intervals: impl Iterator<Item = (i32, i32, i32)>,
) -> BTreeMap<i32, i32> {
    let mut marks = BTreeMap::from([(start, 0), (end, 0)]);
    let mut starts = BTreeMap::<i32, Vec<_>>::new();
    for (start, end, min_height) in intervals {
        marks.insert(start, 0);
        marks.insert(end, 0);
        starts.entry(start).or_default().push((end, min_height));
    }
    // All seven columns share the extra space needed by wrapped labels.
    let mut extra = 0;
    let mut ends = BTreeMap::<i32, i32>::new();
    for (&second, position) in &mut marks {
        let nominal = (second - start) * HOUR_HEIGHT / SECONDS_PER_HOUR + extra;
        *position = nominal.max(ends.remove(&second).unwrap_or(0));
        extra += *position - nominal;
        for &(end, min_height) in starts.get(&second).into_iter().flatten() {
            let required = ends.entry(end).or_default();
            *required = (*required).max(*position + min_height);
        }
    }
    marks
}

fn week_time_position(marks: &BTreeMap<i32, i32>, second: i32) -> i32 {
    let Some((&before, &top)) = marks.range(..=second).next_back() else {
        return 0;
    };
    if second == before {
        return top;
    }
    let Some((&after, &bottom)) = marks.range(second..).next() else {
        return top;
    };
    top + (i64::from(bottom - top) * i64::from(second - before) / i64::from(after - before)) as i32
}

fn draw_week_grid(
    f: &mut Frame,
    x: i32,
    y: i32,
    width: i32,
    view: &View,
    monday: NaiveDate,
    today: NaiveDate,
) -> Option<Scrollbar> {
    let p = f.p;
    let cards = week_cards(f, x, width, view, monday);
    let first_hour = cards.iter().map(|card| card.start / SECONDS_PER_HOUR).min();
    let last_hour = cards
        .iter()
        .map(|card| (card.end + SECONDS_PER_HOUR - 1) / SECONDS_PER_HOUR)
        .max();
    let start_hour = first_hour.unwrap_or(8).min(8);
    let end_hour = last_hour.unwrap_or(22).max(22);
    let marks = week_time_marks(
        start_hour * SECONDS_PER_HOUR,
        end_hour * SECONDS_PER_HOUR,
        cards
            .iter()
            .map(|card| (card.start, card.end, card.min_height())),
    );
    let position = |second| week_time_position(&marks, second);
    let bottom_margin = if view.status.is_empty() { 22 } else { 42 };
    let height = (f.height - bottom_margin - y).max(1);
    let content = Bounds(x, y, width - SCROLL_GUTTER, height);
    let requested_offset = view
        .week_scroll
        .unwrap_or_else(|| position(first_hour.unwrap_or(8) * SECONDS_PER_HOUR));
    let (offset, scrollbar) = f.scrollbar(
        content,
        position(end_hour * SECONDS_PER_HOUR) + 24,
        requested_offset,
    );
    let time_y = |second| y + 12 + position(second) - offset;
    let today_offset = (today - monday).num_days();
    let left = x + TIME_GUTTER;
    let grid_width = content.2 - TIME_GUTTER;

    f.round(content, 10, p.surface, p.line);
    f.with_clip(Bounds(x + 1, y + 1, content.2 - 2, height - 2), |f| {
        if (0..7).contains(&today_offset) {
            let (left, day_width) = week_column(x, width, today_offset as i32);
            f.wash(
                Bounds(left + 1, y + 3, day_width - 2, height - 6),
                p.accent,
                13,
            );
        }
        for hour in start_hour..=end_hour {
            let line_y = time_y(hour * SECONDS_PER_HOUR);
            f.block(Bounds(left, line_y, grid_width - 1, 1), p.grid);
            f.text(
                Bounds(x, line_y - 10, 38, 20),
                &format!("{hour:02}:00"),
                10,
                400,
                p.muted,
                true,
            );
        }
        for day in 0..7 {
            let (line_x, _) = week_column(x, width, day);
            f.block(Bounds(line_x, y + 1, 1, height - 2), p.grid);
        }

        let second = Local::now().num_seconds_from_midnight() as i32;
        if (0..7).contains(&today_offset)
            && (start_hour * SECONDS_PER_HOUR..=end_hour * SECONDS_PER_HOUR).contains(&second)
        {
            let line_y = time_y(second);
            let (line_x, day_width) = week_column(x, width, today_offset as i32);
            f.block(Bounds(line_x, line_y, day_width - 1, 2), p.danger);
            f.round(Bounds(line_x - 3, line_y - 2, 6, 6), 6, p.danger, p.danger);
        }

        for card in &cards {
            let top = time_y(card.start);
            let bottom = time_y(card.end);
            if bottom > y && top < y + height {
                draw_week_course(f, card, top, bottom);
            }
        }
    });
    scrollbar
}

fn classroom_label(place: &str) -> &str {
    let separator = |c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '·' | '・' | ',' | '，' | '、' | ':' | '：' | '/' | '／' | '-' | '—' | '–'
            )
    };
    let opening = |c| separator(c) || matches!(c, '(' | '（' | '[' | '【');
    let closing = |c| separator(c) || matches!(c, ')' | '）' | ']' | '】');
    let boundary = |c| opening(c) || closing(c);
    let is_campus = |name: &str| name.ends_with("校区") || matches!(name, "长安" | "友谊");

    let place = place.trim();
    let known_campuses = ["长安校区", "友谊校区"];
    let end = place.trim_end_matches(closing);
    let trailing = known_campuses
        .iter()
        .find_map(|campus| end.strip_suffix(campus))
        .or_else(|| {
            end.rsplit_once(boundary)
                .filter(|(_, campus)| is_campus(campus))
                .map(|(name, _)| name)
        });
    let room = trailing.map_or(place, |name| name.trim_end_matches(opening));
    let start = room.trim_start_matches(opening);
    let leading = known_campuses
        .into_iter()
        .find(|campus| start.starts_with(campus))
        .or_else(|| start.split(boundary).next().filter(|name| is_campus(name)));
    leading.map_or(room, |campus| {
        start[campus.len()..].trim_start_matches(closing)
    })
}

#[cfg(test)]
mod tests {
    use super::{
        COURSE_COLORS, classroom_label, course_colors, week_time_marks, week_time_position,
    };
    use std::collections::BTreeSet;

    #[test]
    fn weekly_classroom_preserves_room_names() {
        for (place, expected) in [
            ("长安校区 教东D1-403", "教东D1-403"),
            ("友谊校区教西 D-301", "教西 D-301"),
            ("长安校区 · 教学楼 A101", "教学楼 A101"),
            ("（东区校区）计算中心 2 号机房", "计算中心 2 号机房"),
            ("教 A101, 长安", "教 A101"),
            ("教学楼 A101（友谊校区）", "教学楼 A101"),
            ("教学楼 A101（分组） 友谊校区", "教学楼 A101（分组）"),
            ("教室A101友谊校区", "教室A101"),
            ("友谊校区", ""),
            ("校区计算中心 A102", "校区计算中心 A102"),
            ("创新楼（东） 1201", "创新楼（东） 1201"),
        ] {
            assert_eq!(classroom_label(place), expected, "{place}");
        }
    }

    #[test]
    fn weekly_colors_distinguish_course_names() {
        let titles: Vec<_> = (0..48).map(|index| format!("课程 {index}")).collect();
        let colors = course_colors(titles.iter().map(String::as_str));
        assert_eq!(
            colors,
            course_colors(titles.iter().rev().map(String::as_str))
        );
        assert_eq!(
            colors.values().copied().collect::<BTreeSet<_>>().len(),
            titles.len()
        );
        assert!(
            COURSE_COLORS
                .into_iter()
                .all(|color| colors.values().any(|&selected| color == selected))
        );
        let repeats = course_colors(["高等数学", " 高等数学 "].into_iter());
        assert_eq!(repeats.len(), 1);
    }

    #[test]
    fn weekly_timeline_makes_room_for_overlapping_course_labels() {
        let marks = week_time_marks(
            0,
            7200,
            [(900, 1800, 70), (1350, 2400, 120), (1800, 3600, 80)].into_iter(),
        );
        let y = |second| week_time_position(&marks, second);
        assert!(y(1800) - y(900) >= 70);
        assert!(y(2400) - y(1350) >= 120);
        assert!(y(3600) - y(1800) >= 80);
        assert!(y(1350) < y(1500) && y(1500) < y(1800));
        assert_eq!(y(7200) - y(3600), 80);
    }
}

fn draw_week_course(f: &mut Frame, card: &WeekCard<'_>, top: i32, bottom: i32) {
    let p = f.p;
    let action = Action::Page(Page::Course(card.id));
    let fill = p.course(card.color);
    let stroke = if card.conflict {
        p.danger
    } else if f.hover == Some(action) {
        blend(fill, p.text, 45)
    } else {
        fill
    };
    let inset = ((bottom - top - 1) / 2).min(2);
    let bounds = Bounds(card.x, top + inset, card.width, bottom - top - 2 * inset);
    f.round(bounds, 8, fill, stroke);
    let mut y = bounds.1 + 4;
    let metadata = blend(p.text, fill, 28);
    for (text, size, weight, height) in card.labels {
        if height > 0 {
            f.wrap(
                Bounds(
                    card.x + card.padding,
                    y,
                    card.width - 2 * card.padding,
                    height,
                ),
                text,
                size,
                weight,
                if weight == 700 { p.text } else { metadata },
            );
            y += height + 1;
        }
    }
    f.hit(bounds, action);
}

fn draw_course_detail(
    f: &mut Frame,
    x: i32,
    width: i32,
    entry: Option<&Entry>,
    scroll: i32,
) -> Option<Scrollbar> {
    let p = f.p;
    let Some(entry) = entry else {
        f.text(
            Bounds(x, 120, width, 36),
            "这门课程已删除",
            14,
            500,
            p.muted,
            false,
        );
        return None;
    };
    let viewport = Bounds(x, 116, width - SCROLL_GUTTER, (f.height - 158).max(1));
    let title_width = viewport.2 - 40;
    let title_height = f.wrapped_height(title_width, &entry.title, 22, 700);
    let value_width = viewport.2 - 112;
    let time = entry.start.map_or_else(
        || "未填写".into(),
        |start| {
            format!(
                "{}  ·  {}",
                start.with_timezone(&Local).format("%Y-%m-%d"),
                entry.time_label()
            )
        },
    );
    let lead = match entry.lead {
        Some(0) => "上课时提醒".into(),
        Some(minutes) => format!("提前 {minutes} 分钟"),
        None => "已关闭".into(),
    };
    let fields = [
        ("时间", time.as_str()),
        ("地点", entry.place.trim()),
        ("教师", entry.teacher.trim()),
        ("提醒", lead.as_str()),
    ]
    .map(|(label, value)| {
        let value = if value.is_empty() { "未填写" } else { value };
        (
            label,
            value,
            f.wrapped_height(value_width, value, 14, 500).max(22),
        )
    });
    let height = 58 + title_height + fields.iter().map(|(_, _, h)| h + 14).sum::<i32>();
    let (offset, scrollbar) = f.scrollbar(viewport, height, scroll);
    f.with_clip(viewport, |f| {
        let top = viewport.1 - offset;
        f.round(
            Bounds(x, top, viewport.2 - 1, height),
            12,
            p.surface,
            p.line,
        );
        f.wrap(
            Bounds(x + 20, top + 18, title_width, title_height),
            &entry.title,
            22,
            700,
            p.text,
        );
        let separator = top + 32 + title_height;
        f.block(Bounds(x + 20, separator, title_width, 1), p.line);
        let mut y = separator + 14;
        for (label, value, field_height) in fields {
            f.text(Bounds(x + 20, y, 64, 22), label, 12, 600, p.muted, false);
            f.wrap(
                Bounds(x + 92, y + 1, value_width, field_height),
                value,
                14,
                500,
                p.text,
            );
            y += field_height + 14;
        }
    });
    scrollbar
}

fn draw_conflicts(f: &mut Frame, x: i32, y: i32, w: i32, view: &View) {
    let p = f.p;
    if view.rows.is_empty() {
        f.round(Bounds(x, y, w, 66), 10, p.surface, p.line);
        f.text(
            Bounds(x + 18, y, w - 36, 66),
            "没有待处理的冲突",
            13,
            400,
            p.muted,
            false,
        );
        return;
    }
    for (index, &(id, entry)) in view.rows.iter().enumerate() {
        let top = y + index as i32 * 70;
        let row = Bounds(x, top, w, 62);
        f.round(row, 10, p.surface, p.line);
        f.round(Bounds(x, top + 12, 3, 38), 2, p.danger, p.danger);
        f.text(
            Bounds(x + 18, top + 8, w - 220, 24),
            &entry.title,
            14,
            700,
            p.text,
            false,
        );
        let detail = format!("{}    ·    {}", entry.time_label(), entry.detail_label());
        f.text(
            Bounds(x + 18, top + 33, w - 220, 21),
            &detail,
            11,
            400,
            p.muted,
            false,
        );
        f.button(
            Bounds(x + w - 178, top + 15, 84, 32),
            "编辑",
            Action::Page(Page::Edit(Some(id))),
            false,
        );
        f.button(
            Bounds(x + w - 88, top + 15, 72, 32),
            "已处理",
            Action::ResolveConflict(id),
            false,
        );
    }
}

fn draw_rows(f: &mut Frame, x: i32, y: i32, w: i32, view: &View, empty: &str) {
    let p = f.p;
    if view.rows.is_empty() {
        f.round(Bounds(x, y, w, 84), 10, p.surface, p.line);
        let icon = if view.page == Page::Tasks {
            Icon::Tasks
        } else {
            Icon::Today
        };
        f.icon(Bounds(x + 20, y + 32, 20, 20), icon, p.muted, false);
        f.text(
            Bounds(x + 52, y + 20, w - 70, 24),
            empty,
            13,
            500,
            p.text,
            false,
        );
        f.text(
            Bounds(x + 52, y + 44, w - 70, 22),
            "导入课表或新建待办后，这里会显示最近的内容。",
            11,
            400,
            p.muted,
            false,
        );
        return;
    }
    let available = ((f.height - y - 42) / 64).max(1) as usize;
    let start = view.scroll.min(view.rows.len().saturating_sub(available));
    for (index, &(id, entry)) in view.rows.iter().skip(start).take(available).enumerate() {
        let top = y + index as i32 * 64;
        let row = Bounds(x, top, w, 57);
        let action = Action::Page(if entry.kind == Kind::Course {
            Page::Course(id)
        } else {
            Page::Edit(Some(id))
        });
        let (bg, stroke) = if f.hover == Some(action) {
            (p.tint, p.tint)
        } else {
            (p.surface, p.line)
        };
        f.round(row, 10, bg, stroke);
        if view.page != Page::Preview {
            f.hit(row, action);
        }

        let date = entry
            .start
            .map(|start| start.with_timezone(&Local).format("%m/%d").to_string());
        let clock = entry.time_label();
        let title = if entry.conflict {
            format!("{}  ·  冲突副本", entry.title)
        } else {
            entry.title.clone()
        };
        if entry.kind == Kind::Course {
            f.round(Bounds(x + 10, top + 11, 104, 35), 8, p.tint, p.tint);
            let label = if view.page == Page::Preview {
                date.as_deref().unwrap_or_default()
            } else {
                &clock
            };
            f.text(
                Bounds(x + 11, top + 10, 102, 37),
                label,
                12,
                700,
                p.accent,
                true,
            );
            f.text(
                Bounds(x + 128, top + 6, w - 148, 24),
                &title,
                14,
                700,
                p.text,
                false,
            );
            let subtitle = if view.page == Page::Preview {
                format!("{clock}    {}", entry.detail_label())
            } else {
                entry.detail_label()
            };
            f.text(
                Bounds(x + 128, top + 30, w - 148, 22),
                &subtitle,
                11,
                400,
                p.muted,
                false,
            );
        } else {
            f.round(Bounds(x + 10, top + 11, 36, 35), 8, p.field, p.field);
            f.text(
                Bounds(x + 10, top + 10, 36, 37),
                if entry.done { "✓" } else { "" },
                15,
                700,
                p.accent,
                true,
            );
            if !entry.done {
                f.round(Bounds(x + 20, top + 21, 16, 15), 4, p.surface, p.muted);
            }
            f.hit(Bounds(x + 10, top + 11, 36, 35), Action::ToggleEntry(id));
            f.text(
                Bounds(x + 58, top + 6, w - 78, 24),
                &title,
                14,
                700,
                if entry.done { p.muted } else { p.text },
                false,
            );
            let subtitle = [date.as_deref().unwrap_or_default(), &clock, &entry.place]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join("   ·   ");
            f.text(
                Bounds(x + 58, top + 30, w - 78, 22),
                &subtitle,
                11,
                400,
                p.muted,
                false,
            );
        }
    }
    if view.rows.len() > available {
        let range = format!(
            "{}–{} / {}",
            start + 1,
            (start + available).min(view.rows.len()),
            view.rows.len()
        );
        f.text(
            Bounds(x + w - 100, f.height - 33, 100, 25),
            &range,
            11,
            400,
            p.muted,
            true,
        );
    }
}

pub const TOAST_WIDTH: i32 = 380;
pub const TOAST_HEIGHT: i32 = 186;

pub fn draw_toast(
    window: HWND,
    dpi: i32,
    p: Palette,
    entry: &Entry,
    test: bool,
    hover: Option<Action>,
) -> Vec<Hit> {
    let mut f = Frame::new(window, dpi, p, hover);
    f.round(
        Bounds(0, 1, TOAST_WIDTH, TOAST_HEIGHT - 2),
        13,
        p.surface,
        p.line,
    );
    f.round(Bounds(14, 14, 30, 30), 9, p.hero, p.hero);
    f.clock(29, 29, 8, 0xdcf2ea);
    let label = if test {
        "提醒预览"
    } else if entry.kind == Kind::Course {
        "课程提醒"
    } else {
        "待办提醒"
    };
    f.text(Bounds(56, 15, 245, 24), label, 12, 700, p.accent, false);
    f.button(Bounds(338, 13, 28, 28), "×", Action::Dismiss, false);
    f.text(
        Bounds(18, 48, 344, 34),
        &entry.title,
        21,
        700,
        p.text,
        false,
    );
    f.text(
        Bounds(19, 86, 336, 21),
        &entry.time_label(),
        13,
        600,
        p.text,
        false,
    );
    let detail = entry.detail_label();
    let place = if detail.is_empty() && entry.kind == Kind::Course {
        "未填写地点"
    } else {
        &detail
    };
    f.text(Bounds(19, 108, 336, 21), place, 12, 400, p.muted, false);
    if !test {
        f.button(Bounds(18, 141, 92, 31), "查看", Action::OpenReminder, true);
        f.button(Bounds(120, 141, 108, 31), "5 分钟后", Action::Snooze, false);
        if entry.kind == Kind::Task {
            f.button(
                Bounds(238, 141, 108, 31),
                "完成",
                Action::DoneReminder,
                false,
            );
        }
    }
    f.finish()
}

fn gp_ready() -> bool {
    static READY: LazyLock<bool> = LazyLock::new(|| unsafe {
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            ..Default::default()
        };
        let mut token = 0_usize;
        GdiplusStartup(&mut token, &input, null_mut()) == 0
    });
    *READY
}

fn argb(rgb: u32) -> u32 {
    0xff00_0000 | rgb
}

struct Frame {
    hwnd: HWND,
    paint: PAINTSTRUCT,
    target: HDC,
    dc: HDC,
    bitmap: HBITMAP,
    old_bitmap: HGDIOBJ,
    fonts: BTreeMap<(i32, i32), HFONT>,
    pixels: RECT,
    dpi: i32,
    width: i32,
    height: i32,
    p: Palette,
    hits: Vec<Hit>,
    clip: Option<RECT>,
    hover: Option<Action>,
    g: *mut GpGraphics,
}

impl Frame {
    fn new(hwnd: HWND, dpi: i32, p: Palette, hover: Option<Action>) -> Self {
        let mut paint = PAINTSTRUCT::default();
        let mut pixels = RECT::default();
        unsafe {
            let target = BeginPaint(hwnd, &mut paint);
            GetClientRect(hwnd, &mut pixels);
            let dc = CreateCompatibleDC(target);
            let bitmap = CreateCompatibleBitmap(target, pixels.right, pixels.bottom);
            let old_bitmap = SelectObject(dc, bitmap);
            SetBkMode(dc, TRANSPARENT as i32);
            let mut g = null_mut();
            if gp_ready() {
                GdipCreateFromHDC(dc, &mut g);
                if !g.is_null() {
                    GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
                    GdipSetPixelOffsetMode(g, PixelOffsetModeHalf);
                    GdipSetCompositingMode(g, 0);
                }
            }
            Self {
                hwnd,
                paint,
                target,
                dc,
                bitmap,
                old_bitmap,
                fonts: BTreeMap::new(),
                pixels,
                dpi,
                width: pixels.right * 96 / dpi,
                height: pixels.bottom * 96 / dpi,
                p,
                hits: Vec::new(),
                clip: None,
                hover,
                g,
            }
        }
    }

    fn finish(mut self) -> Vec<Hit> {
        std::mem::take(&mut self.hits)
    }

    fn gflush(&self) {
        if !self.g.is_null() {
            unsafe {
                GdipFlush(self.g, FlushIntentionFlush);
            }
        }
    }

    fn with_clip(&mut self, bounds: Bounds, draw: impl FnOnce(&mut Self)) {
        let rect = bounds.rect(self.dpi);
        self.gflush();
        let saved = unsafe {
            let saved = SaveDC(self.dc);
            IntersectClipRect(self.dc, rect.left, rect.top, rect.right, rect.bottom);
            if !self.g.is_null() {
                GdipSetClipRectI(
                    self.g,
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    CombineModeReplace,
                );
            }
            saved
        };
        self.clip = Some(rect);
        draw(self);
        self.clip = None;
        self.gflush();
        unsafe {
            if !self.g.is_null() {
                GdipResetClip(self.g);
            }
            RestoreDC(self.dc, saved);
        }
    }

    fn scrollbar(
        &self,
        viewport: Bounds,
        content_height: i32,
        offset: i32,
    ) -> (i32, Option<Scrollbar>) {
        let Bounds(x, y, width, height) = viewport;
        let limit = content_height - height;
        if limit <= 0 {
            return (0, None);
        }
        let offset = offset.clamp(0, limit);
        let track = Bounds(x + width + 5, y, 10, height);
        let thumb_height = (height * height / content_height)
            .min(height)
            .max(28.min(height));
        let thumb_y = y + (height - thumb_height) * offset / limit;
        let thumb = Bounds(track.0, thumb_y, track.2, thumb_height);
        self.round(track, 5, self.p.field, self.p.field);
        self.round(thumb, 5, self.p.muted, self.p.muted);
        (
            offset,
            Some(Scrollbar {
                track: track.rect(self.dpi),
                thumb: thumb.rect(self.dpi),
                offset,
                limit,
                page_height: height,
            }),
        )
    }

    fn px(&self, bounds: Bounds) -> (f32, f32, f32, f32) {
        let rect = bounds.rect(self.dpi);
        (
            rect.left as f32,
            rect.top as f32,
            (rect.right - rect.left) as f32,
            (rect.bottom - rect.top) as f32,
        )
    }

    fn path(&self, bounds: Bounds, radius: i32) -> *mut GpPath {
        let (x, y, w, h) = self.px(bounds);
        let r = (scale(radius, self.dpi) as f32).min(w / 2.0).min(h / 2.0);
        unsafe {
            let mut path = null_mut();
            if GdipCreatePath(FillModeAlternate, &mut path) != 0 {
                return null_mut();
            }
            let d = r * 2.0;
            let right = x + w;
            let bottom = y + h;
            GdipAddPathArc(path, x, y, d, d, 180.0, 90.0);
            GdipAddPathArc(path, right - d, y, d, d, 270.0, 90.0);
            GdipAddPathArc(path, right - d, bottom - d, d, d, 0.0, 90.0);
            GdipAddPathArc(path, x, bottom - d, d, d, 90.0, 90.0);
            GdipClosePathFigure(path);
            path
        }
    }

    fn block(&self, bounds: Bounds, fill: u32) {
        self.gflush();
        unsafe {
            let brush = CreateSolidBrush(color(fill));
            FillRect(self.dc, &bounds.rect(self.dpi), brush);
            DeleteObject(brush);
        }
    }

    fn wash(&self, bounds: Bounds, fill: u32, alpha: u32) {
        if self.g.is_null() {
            return;
        }
        self.gflush();
        let (x, y, w, h) = self.px(bounds);
        unsafe {
            let mut brush = null_mut();
            if GdipCreateSolidFill((alpha << 24) | fill, &mut brush) != 0 {
                return;
            }
            let brush = brush.cast::<GpBrush>();
            let mut path = null_mut();
            GdipCreatePath(FillModeAlternate, &mut path);
            GdipAddPathRectangle(path, x, y, w, h);
            GdipFillPath(self.g, brush, path);
            GdipDeletePath(path);
            GdipDeleteBrush(brush);
        }
    }

    fn round(&self, bounds: Bounds, radius: i32, fill: u32, stroke: u32) {
        if self.g.is_null() || radius <= 0 {
            self.gflush();
            unsafe {
                let brush = CreateSolidBrush(color(fill));
                FillRect(self.dc, &bounds.rect(self.dpi), brush);
                DeleteObject(brush);
            }
            return;
        }
        self.gflush();
        let path = self.path(bounds, radius);
        if path.is_null() {
            return;
        }
        unsafe {
            let mut brush = null_mut();
            if GdipCreateSolidFill(argb(fill), &mut brush) == 0 {
                let brush = brush.cast::<GpBrush>();
                GdipFillPath(self.g, brush, path);
                GdipDeleteBrush(brush);
            }
            if fill != stroke {
                let mut pen = null_mut();
                if GdipCreatePen1(argb(stroke), 1.0, UnitPixel, &mut pen) == 0 {
                    GdipDrawPath(self.g, pen, path);
                    GdipDeletePen(pen);
                }
            }
            GdipDeletePath(path);
        }
    }

    fn round_gradient(&self, bounds: Bounds, radius: i32, top: u32, bottom: u32) {
        let path = self.path(bounds, radius);
        if path.is_null() || self.g.is_null() {
            self.round(bounds, radius, top, top);
            return;
        }
        self.gflush();
        let rect = bounds.rect(self.dpi);
        unsafe {
            let native = Rect {
                X: rect.left,
                Y: rect.top,
                Width: rect.right - rect.left,
                Height: rect.bottom - rect.top,
            };
            let mut brush = null_mut();
            if GdipCreateLineBrushFromRectI(
                &native,
                argb(top),
                argb(bottom),
                LinearGradientModeVertical,
                0,
                &mut brush,
            ) == 0
            {
                let brush = brush.cast::<GpBrush>();
                GdipFillPath(self.g, brush, path);
                GdipDeleteBrush(brush);
            }
            GdipDeletePath(path);
        }
    }

    fn dashed(&self, bounds: Bounds, radius: i32, stroke: u32) {
        let path = self.path(bounds, radius);
        if path.is_null() || self.g.is_null() {
            return;
        }
        self.gflush();
        unsafe {
            let mut pen = null_mut();
            if GdipCreatePen1(argb(stroke), scale(1, self.dpi) as f32, UnitPixel, &mut pen) == 0 {
                GdipSetPenDashStyle(pen, 1);
                GdipDrawPath(self.g, pen, path);
                GdipDeletePen(pen);
            }
            GdipDeletePath(path);
        }
    }

    fn font(&mut self, size: i32, weight: i32) -> HFONT {
        let dpi = self.dpi;
        *self.fonts.entry((size, weight)).or_insert_with(|| unsafe {
            CreateFontW(
                -scale(size, dpi),
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET.into(),
                OUT_DEFAULT_PRECIS.into(),
                CLIP_DEFAULT_PRECIS.into(),
                CLEARTYPE_QUALITY.into(),
                DEFAULT_PITCH.into(),
                wide("Microsoft YaHei UI").as_ptr(),
            )
        })
    }

    fn text(
        &mut self,
        bounds: Bounds,
        value: &str,
        size: i32,
        weight: i32,
        fill: u32,
        center: bool,
    ) {
        let font = self.font(size, weight);
        self.gflush();
        let mut rect = bounds.rect(self.dpi);
        let text = wide(value);
        unsafe {
            let previous = SelectObject(self.dc, font);
            SetTextColor(self.dc, color(fill));
            let align = if center { DT_CENTER } else { DT_LEFT };
            DrawTextW(
                self.dc,
                text.as_ptr(),
                (text.len() - 1) as i32,
                &mut rect,
                align | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
            SelectObject(self.dc, previous);
        }
    }

    fn wrapped_height(&mut self, width: i32, value: &str, size: i32, weight: i32) -> i32 {
        let font = self.font(size, weight);
        let mut rect = Bounds(0, 0, width, 0).rect(self.dpi);
        let text = wide(value);
        let pixels = unsafe {
            let previous = SelectObject(self.dc, font);
            let pixels = DrawTextW(
                self.dc,
                text.as_ptr(),
                (text.len() - 1) as i32,
                &mut rect,
                DT_CALCRECT | DT_WORDBREAK | DT_EDITCONTROL | DT_NOPREFIX,
            );
            SelectObject(self.dc, previous);
            pixels
        };
        (pixels * 96 + self.dpi - 1) / self.dpi
    }

    fn wrap(&mut self, bounds: Bounds, value: &str, size: i32, weight: i32, fill: u32) {
        let font = self.font(size, weight);
        self.gflush();
        let mut rect = bounds.rect(self.dpi);
        let text = wide(value);
        unsafe {
            let previous = SelectObject(self.dc, font);
            SetTextColor(self.dc, color(fill));
            DrawTextW(
                self.dc,
                text.as_ptr(),
                (text.len() - 1) as i32,
                &mut rect,
                DT_WORDBREAK | DT_EDITCONTROL | DT_NOPREFIX,
            );
            SelectObject(self.dc, previous);
        }
    }

    fn clock(&self, x: i32, y: i32, radius: i32, stroke: u32) {
        let p = |value| scale(value, self.dpi) as f32;
        let (cx, cy, r) = (p(x), p(y), p(radius));
        if self.g.is_null() {
            return;
        }
        self.gflush();
        unsafe {
            let mut pen = null_mut();
            if GdipCreatePen1(argb(stroke), p(2).max(1.0), UnitPixel, &mut pen) != 0 {
                return;
            }
            GdipSetPenLineCap197819(pen, LineCapRound, LineCapRound, LineCapRound);
            GdipDrawEllipse(self.g, pen, cx - r, cy - r, r * 2.0, r * 2.0);
            GdipDrawLine(self.g, pen, cx, cy - r / 2.0, cx, cy);
            GdipDrawLine(self.g, pen, cx, cy, cx + r / 2.0, cy);
            GdipDeletePen(pen);
        }
    }

    fn icon(&mut self, bounds: Bounds, icon: Icon, stroke: u32, active: bool) {
        let (x, y, w, h) = self.px(bounds);
        let width = if active { 1.7 } else { 1.5 } * scale(1, self.dpi) as f32;
        if self.g.is_null() {
            return;
        }
        self.gflush();
        let cx = x + w / 2.0;
        let cy = y + h / 2.0;
        unsafe {
            let mut pen = null_mut();
            if GdipCreatePen1(argb(stroke), width, UnitPixel, &mut pen) != 0 {
                return;
            }
            GdipSetPenLineJoin(pen, LineJoinRound);
            GdipSetPenLineCap197819(pen, LineCapRound, LineCapRound, LineCapRound);
            let mut dot = null_mut();
            GdipCreateSolidFill(argb(stroke), &mut dot);
            let dot = dot.cast::<GpBrush>();
            let path = |f: &mut dyn FnMut(*mut GpPath)| {
                let mut p = null_mut();
                if GdipCreatePath(FillModeAlternate, &mut p) == 0 {
                    f(p);
                    GdipDeletePath(p);
                }
            };
            match icon {
                Icon::Today => {
                    GdipDrawEllipse(self.g, pen, x + 1.5, y + 1.5, w - 3.0, h - 3.0);
                    if !dot.is_null() {
                        GdipFillEllipse(
                            self.g,
                            dot,
                            cx - w * 0.16,
                            cy - h * 0.16,
                            w * 0.32,
                            h * 0.32,
                        );
                    }
                }
                Icon::Week => {
                    path(&mut |p| {
                        GdipAddPathRectangle(p, x + 1.0, y + 3.0, w - 2.0, h - 4.0);
                        GdipAddPathLine(p, x + 1.0, y + h * 0.38, x + w - 1.0, y + h * 0.38);
                        GdipAddPathLine(p, x + w * 0.36, y + 3.0, x + w * 0.36, y + h - 1.0);
                        GdipDrawPath(self.g, pen, p);
                    });
                    GdipDrawLine(self.g, pen, x + 5.0, y + 1.0, x + 5.0, y + 2.0);
                    GdipDrawLine(self.g, pen, x + w - 5.0, y + 1.0, x + w - 5.0, y + 2.0);
                }
                Icon::Tasks => {
                    path(&mut |p| {
                        GdipAddPathRectangle(p, x + 1.0, y + 1.0, w - 2.0, h - 2.0);
                        GdipDrawPath(self.g, pen, p);
                    });
                    path(&mut |p| {
                        GdipAddPathLine(p, x + w * 0.24, cy, x + w * 0.44, y + h * 0.72);
                        GdipAddPathLine(p, x + w * 0.44, y + h * 0.72, x + w * 0.78, y + h * 0.3);
                        GdipDrawPath(self.g, pen, p);
                    });
                }
                Icon::Alert => {
                    path(&mut |p| {
                        GdipAddPathLine(p, cx, y + 1.0, x + w - 1.0, y + h - 1.0);
                        GdipAddPathLine(p, x + w - 1.0, y + h - 1.0, x + 1.0, y + h - 1.0);
                        GdipAddPathLine(p, x + 1.0, y + h - 1.0, cx, y + 1.0);
                        GdipDrawPath(self.g, pen, p);
                    });
                    GdipDrawLine(self.g, pen, cx, y + h * 0.42, cx, y + h * 0.68);
                    if !dot.is_null() {
                        GdipFillEllipse(self.g, dot, cx - 1.2, y + h * 0.78, 2.4, 2.4);
                    }
                }
                Icon::Import => {
                    GdipDrawLine(self.g, pen, cx, y + 1.0, cx, y + h * 0.62);
                    path(&mut |p| {
                        GdipAddPathLine(p, cx - w * 0.22, y + h * 0.4, cx, y + h * 0.64);
                        GdipAddPathLine(p, cx, y + h * 0.64, cx + w * 0.22, y + h * 0.4);
                        GdipDrawPath(self.g, pen, p);
                    });
                    path(&mut |p| {
                        GdipAddPathLine(p, x + 2.0, y + h - 3.0, x + w - 2.0, y + h - 3.0);
                        GdipDrawPath(self.g, pen, p);
                    });
                }
                Icon::Settings => {
                    for (line, knob) in [(0.32_f32, 0.34_f32), (0.7, 0.66)] {
                        let ly = y + h * line;
                        GdipDrawLine(self.g, pen, x + 1.0, ly, x + w - 1.0, ly);
                        let kx = x + w * knob;
                        if !dot.is_null() {
                            GdipFillEllipse(self.g, dot, kx - 2.6, ly - 2.6, 5.2, 5.2);
                        }
                    }
                }
            }
            if !dot.is_null() {
                GdipDeleteBrush(dot);
            }
            GdipDeletePen(pen);
        }
    }

    fn toggle(&mut self, bounds: Bounds, label: &str, on: bool, action: Action) {
        let p = self.p;
        let hover = self.hover == Some(action);
        self.round(
            bounds,
            9,
            if hover { p.tint } else { p.surface },
            if hover { p.accent } else { p.line },
        );
        let track = Bounds(bounds.0 + 12, bounds.1 + 11, 34, 16);
        self.round(
            track,
            8,
            if on { p.accent } else { p.line },
            if on { p.accent } else { p.line },
        );
        let knob = Bounds(
            if on { track.0 + 20 } else { track.0 + 2 },
            track.1 + 2,
            12,
            12,
        );
        let knob_fill = if on { 0xffffff } else { p.muted };
        self.round(knob, 6, knob_fill, knob_fill);
        self.text(
            Bounds(bounds.0 + 56, bounds.1, bounds.2 - 56, bounds.3),
            label,
            12,
            500,
            p.text,
            false,
        );
        self.hit(bounds, action);
    }

    fn button(&mut self, bounds: Bounds, label: &str, action: Action, primary: bool) {
        let p = self.p;
        let (background, stroke, text) = if primary {
            let fill = if self.hover == Some(action) {
                p.hero
            } else {
                p.accent
            };
            (fill, fill, p.accent_ink)
        } else if self.hover == Some(action) {
            (p.tint, p.tint, p.accent)
        } else {
            (p.surface, p.line, p.text)
        };
        self.round(bounds, 9, background, stroke);
        self.text(
            bounds,
            label,
            12,
            if primary { 700 } else { 500 },
            text,
            true,
        );
        self.hit(bounds, action);
    }

    fn hit(&mut self, bounds: Bounds, action: Action) {
        let mut rect = bounds.rect(self.dpi);
        if let Some(clip) = self.clip {
            rect.left = rect.left.max(clip.left);
            rect.top = rect.top.max(clip.top);
            rect.right = rect.right.min(clip.right);
            rect.bottom = rect.bottom.min(clip.bottom);
        }
        if rect.left < rect.right && rect.top < rect.bottom {
            self.hits.push(Hit(rect, action));
        }
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        unsafe {
            if !self.g.is_null() {
                GdipFlush(self.g, FlushIntentionFlush);
                GdipDeleteGraphics(self.g);
                self.g = null_mut();
            }
            BitBlt(
                self.target,
                0,
                0,
                self.pixels.right,
                self.pixels.bottom,
                self.dc,
                0,
                0,
                SRCCOPY,
            );
            SelectObject(self.dc, self.old_bitmap);
            for font in self.fonts.values() {
                DeleteObject(*font);
            }
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
            EndPaint(self.hwnd, &self.paint);
        }
    }
}

pub fn color(value: u32) -> u32 {
    ((value & 0xff0000) >> 16) | (value & 0x00ff00) | ((value & 0xff) << 16)
}
