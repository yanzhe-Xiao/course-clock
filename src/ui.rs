use std::{collections::BTreeMap, ptr::null_mut, sync::LazyLock};

use chrono::{Datelike, Duration, Local, NaiveDate, Timelike};
use uuid::Uuid;
use windows_sys::Win32::{
    Foundation::{HWND, RECT},
    Graphics::{
        Gdi::*,
        GdiPlus::{
            FillModeAlternate, FlushIntentionFlush, GdipAddPathArc, GdipAddPathLine,
            GdipAddPathRectangle, GdipClosePathFigure, GdipCreateFromHDC,
            GdipCreateLineBrushFromRectI, GdipCreatePath, GdipCreatePen1, GdipCreateSolidFill,
            GdipDeleteBrush, GdipDeleteGraphics, GdipDeletePath, GdipDeletePen, GdipDrawEllipse,
            GdipDrawLine, GdipDrawPath, GdipFillEllipse, GdipFillPath, GdipFlush,
            GdipSetCompositingMode, GdipSetPenDashStyle, GdipSetPenLineCap197819,
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
    Edit(Option<Uuid>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Page(Page),
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
    (value * dpi + 48) / 96
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

    fn course(self, index: usize) -> (u32, u32) {
        let light = [
            (0xdff0e8, 0x2f8064),
            (0xe2ebf4, 0x3b6f9e),
            (0xf3ebd9, 0x9a7231),
            (0xebe5f4, 0x6a569b),
            (0xe9eedd, 0x66793a),
        ];
        let dark = [
            (0x1e3a33, 0x6fd0ae),
            (0x1d2f3d, 0x79b3dd),
            (0x362c1c, 0xd6ab63),
            (0x2b2440, 0xae9ae2),
            (0x29301f, 0xa9c173),
        ];
        if self.dark { dark[index] } else { light[index] }
    }
}

pub struct View<'a> {
    pub page: Page,
    pub day: NaiveDate,
    pub rows: &'a [(Uuid, &'a Entry)],
    pub next: Option<&'a Entry>,
    pub scroll: usize,
    pub status: &'a str,
    pub sync_status: &'a str,
    pub connected: bool,
    pub startup: bool,
    pub editing_course: bool,
    pub fields: &'a [Field],
    pub hover: Option<Action>,
    pub conflict_ids: &'a std::collections::BTreeSet<Uuid>,
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

pub fn draw_main(window: HWND, dpi: i32, palette: Palette, view: View<'_>) -> Vec<Hit> {
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
        let active = view.page == page
            || page == Page::Import && view.page == Page::Preview
            || page == Page::Tasks && matches!(view.page, Page::Edit(_)) && !view.editing_course
            || page == Page::Week && matches!(view.page, Page::Edit(_)) && view.editing_course;
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
        _ => None,
    };
    if let Some((label, action)) = header_action {
        f.button(Bounds(w - 155, 52, 123, 36), label, action, true);
    }

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
                Bounds(x, 122, width - 250, 30),
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
                    Bounds(x + width - dx, 122, 64, 32),
                    caption,
                    Action::Day(day),
                    false,
                );
            }
            let day_width = width / 7;
            for (index, label) in ["周一", "周二", "周三", "周四", "周五", "周六", "周日"]
                .into_iter()
                .enumerate()
            {
                let date = monday + Duration::days(index as i64);
                let bounds = Bounds(x + index as i32 * day_width, 166, day_width - 6, 62);
                let action = Action::Day(date);
                let selected = date == view.day;
                let (background, stroke) = if selected {
                    (p.hero, p.hero)
                } else if f.hover == Some(action) {
                    (p.tint, p.tint)
                } else {
                    (p.surface, p.line)
                };
                f.round(bounds, 10, background, stroke);
                f.text(
                    Bounds(bounds.0, bounds.1 + 8, bounds.2, 20),
                    label,
                    11,
                    500,
                    if selected { p.hero_soft } else { p.muted },
                    true,
                );
                f.text(
                    Bounds(bounds.0, bounds.1 + 27, bounds.2, 28),
                    &date.day().to_string(),
                    21,
                    700,
                    if selected { 0xffffff } else { p.text },
                    true,
                );
                f.hit(bounds, action);
            }
            f.text(
                Bounds(x + 1, 240, width - 140, 28),
                view.semester,
                12,
                500,
                p.muted,
                false,
            );
            draw_week_grid(&mut f, x, 274, width, &view, monday, today);
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
        Page::Edit(id) => {
            draw_fields(&mut f, view.fields);
            let (note, note_x, note_y) = if view.editing_course {
                ("提前分钟留空时关闭提醒。", 0, 318)
            } else {
                ("留空可关闭提醒。", 0, 380)
            };
            f.text(
                Bounds(x + note_x, note_y, width - note_x, 26),
                note,
                12,
                400,
                p.muted,
                false,
            );
            let button_y = if view.editing_course { 366 } else { 414 };
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
    f.finish()
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

fn draw_week_grid(
    f: &mut Frame,
    x: i32,
    y: i32,
    width: i32,
    view: &View,
    monday: NaiveDate,
    today: NaiveDate,
) {
    let p = f.p;
    let bottom = f.height - 42;
    let height = (bottom - y).max(120);
    let left = x + 44;
    let grid_width = width - 44;
    let day_width = grid_width / 7;
    let start_minutes = 8 * 60;
    let end_minutes = 22 * 60;
    let scale = height as f32 / (end_minutes - start_minutes) as f32;

    let today_offset = (today - monday).num_days();
    if (0..7).contains(&today_offset) {
        let band = Bounds(
            left + today_offset as i32 * day_width,
            y,
            day_width - 1,
            height,
        );
        f.wash(band, p.accent, 14);
    }

    for hour in 8..=22 {
        let line_y = y + ((hour * 60 - start_minutes) as f32 * scale) as i32;
        f.block(Bounds(left, line_y, grid_width, 1), p.grid);
        f.text(
            Bounds(x, line_y - 10, 38, 20),
            &format!("{hour:02}:00"),
            10,
            400,
            p.muted,
            true,
        );
    }
    for day in 1..7 {
        let line_x = left + day * day_width;
        f.block(Bounds(line_x, y, 1, height), p.grid);
    }

    let now = Local::now();
    if (0..7).contains(&today_offset) {
        let minutes = now.hour() as i32 * 60 + now.minute() as i32;
        if (start_minutes..=end_minutes).contains(&minutes) {
            let line_y = y + ((minutes - start_minutes) as f32 * scale) as i32;
            let line_x = left + today_offset as i32 * day_width;
            f.block(Bounds(line_x, line_y, day_width - 1, 2), p.danger);
            f.round(Bounds(line_x - 3, line_y - 2, 6, 6), 6, p.danger, p.danger);
        }
    }

    for &(id, entry) in view.rows {
        let Some(start) = entry.start.map(|time| time.with_timezone(&Local)) else {
            continue;
        };
        let Some(end) = entry.end.map(|time| time.with_timezone(&Local)) else {
            continue;
        };
        let day = start.weekday().num_days_from_monday() as i32;
        let top = y
            + (((start.hour() * 60 + start.minute()) as i32 - start_minutes).max(0) as f32 * scale)
                as i32
            + 2;
        let finish = y
            + (((end.hour() * 60 + end.minute()) as i32 - start_minutes).max(0) as f32 * scale)
                as i32
            - 2;
        let block_height = (finish - top).max(24);
        let conflict = view.conflict_ids.contains(&id) || entry.conflict;
        let bounds = Bounds(left + day * day_width + 3, top, day_width - 7, block_height);
        let hash = entry.title.bytes().fold(0_u32, |hash, byte| {
            hash.wrapping_mul(33).wrapping_add(byte as u32)
        });
        let (fill, ink) = p.course((hash % 5) as usize);
        f.round(bounds, 7, fill, fill);
        f.round(
            Bounds(bounds.0, bounds.1 + 5, 3, (block_height - 10).max(3)),
            2,
            if conflict { p.danger } else { ink },
            if conflict { p.danger } else { ink },
        );
        if block_height >= 22 {
            let title_lines = ((block_height - 4) / 16).clamp(1, 3);
            f.wrap(
                Bounds(bounds.0 + 9, bounds.1 + 4, bounds.2 - 14, block_height - 6),
                &entry.title,
                11,
                700,
                p.text,
                title_lines,
            );
            if block_height >= 54 {
                f.text(
                    Bounds(bounds.0 + 9, bounds.1 + block_height - 20, bounds.2 - 14, 18),
                    &entry.time_label(),
                    10,
                    600,
                    ink,
                    false,
                );
            }
        }
        f.hit(bounds, Action::Page(Page::Edit(Some(id))));
    }
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
        let action = Action::Page(Page::Edit(Some(id)));
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

    fn text(
        &mut self,
        bounds: Bounds,
        value: &str,
        size: i32,
        weight: i32,
        fill: u32,
        center: bool,
    ) {
        let dpi = self.dpi;
        let font = *self.fonts.entry((size, weight)).or_insert_with(|| unsafe {
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
        });
        self.gflush();
        let mut rect = bounds.rect(dpi);
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

    fn wrap(
        &mut self,
        bounds: Bounds,
        value: &str,
        size: i32,
        weight: i32,
        fill: u32,
        lines: i32,
    ) {
        let dpi = self.dpi;
        let font = *self.fonts.entry((size, weight)).or_insert_with(|| unsafe {
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
        });
        self.gflush();
        let mut rect = bounds.rect(dpi);
        let line = scale(size + 3, dpi);
        rect.bottom = rect.bottom.min(rect.top + line * lines);
        let text = wide(value);
        unsafe {
            let previous = SelectObject(self.dc, font);
            SetTextColor(self.dc, color(fill));
            DrawTextW(
                self.dc,
                text.as_ptr(),
                (text.len() - 1) as i32,
                &mut rect,
                DT_LEFT | DT_TOP | DT_WORDBREAK | DT_END_ELLIPSIS | DT_NOPREFIX,
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
        self.hits.push(Hit(bounds.rect(self.dpi), action));
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
