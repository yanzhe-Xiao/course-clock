use std::{
    cell::RefCell,
    fs,
    path::PathBuf,
    ptr::{null, null_mut},
    sync::mpsc::{self, Receiver},
};

use chrono::{DateTime, Datelike, Duration, Local, Months, NaiveDate, Timelike, Utc};
use uuid::Uuid;
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM, LRESULT, POINT,
        RECT, WPARAM,
    },
    Graphics::Gdi::*,
    System::Threading::CreateMutexW,
    UI::{
        Controls::*,
        HiDpi::{
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow,
            SetProcessDpiAwarenessContext,
        },
        Input::KeyboardAndMouse::*,
        Shell::*,
        WindowsAndMessaging::*,
    },
};

use crate::{
    import::{self, Campus},
    model::{self, Alarm, DavSettings, Entries, Entry, Kind, RepeatRule, Result, Semester, State},
    sync::{self, Synced},
    ui::{
        self, Action, Bounds, EntryTime, Field, Hit, Page, Palette, Picker, PickerKind, Scrollbar,
        View,
    },
    win::{self, wide},
};

const CONTROLLER: &str = "CourseClock.Controller.01";
const MAIN: &str = "CourseClock.Main.01";
const TOAST: &str = "CourseClock.Reminder.01";
const PICKER: &str = "CourseClock.Picker.01";
const TRAY: u32 = WM_APP + 1;
const ACTIVATE: u32 = WM_APP + 2;
const SYNC_DONE: u32 = WM_APP + 3;
const TIMER: usize = 1;
const OPEN_MENU: u32 = 1;
const SYNC_MENU: u32 = 2;
const EXIT_MENU: u32 = 3;
const MAX_IMPORT_BYTES: usize = 32 * 1024 * 1024;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

pub fn run() -> Result<()> {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let directory = model::data_dir()?;
    fs::create_dir_all(&directory).map_err(|error| format!("无法创建数据目录：{error}"))?;
    let directory = directory
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let mutex_name = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        directory.to_string_lossy().to_lowercase().as_bytes(),
    );
    let title = wide(&format!("CourseClock {mutex_name}"));
    let mutex = OwnedHandle(unsafe { CreateMutexW(null(), 0, title.as_ptr()) });
    if mutex.0.is_null() {
        return Err("无法创建程序锁".into());
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe {
            let existing = FindWindowW(wide(CONTROLLER).as_ptr(), title.as_ptr());
            if !existing.is_null() {
                PostMessageW(existing, ACTIVATE, 0, 0);
            }
        }
        return Ok(());
    }
    let path = directory.join("state.json");
    let state = model::open_state(&path)?;
    let icon = win::icon();
    register(CONTROLLER, Some(controller), icon, 0)?;
    register(MAIN, Some(main_proc), icon, 0)?;
    register(TOAST, Some(toast_proc), icon, CS_DROPSHADOW)?;
    register(PICKER, Some(picker_proc), icon, CS_DROPSHADOW)?;
    let controller = create(
        CONTROLLER,
        &title,
        WS_POPUP,
        WS_EX_TOOLWINDOW,
        0,
        0,
        0,
        0,
        null_mut(),
    );
    if controller.is_null() {
        return Err("无法建立后台窗口".into());
    }
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let today = Local::now().date_naive();
    let app = App {
        controller,
        icon,
        exe: exe.clone(),
        path,
        state,
        main: null_mut(),
        toast: null_mut(),
        toast_data: None,
        toast_dpi: 96,
        taskbar_message: unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) },
        startup: win::startup(&exe, None).unwrap_or(false),
        dark: win::dark_mode(),
        page: Page::Today,
        day: today,
        scroll: 0,
        week_scroll: None,
        detail_scroll: 0,
        scrollbar: None,
        scroll_drag: None,
        main_hits: Vec::new(),
        toast_hits: Vec::new(),
        hover: None,
        toast_hover: None,
        input: Vec::new(),
        entry_times: None,
        picker: None,
        picker_window: null_mut(),
        picker_hits: Vec::new(),
        picker_hover: None,
        picker_focus: None,
        selects: Vec::new(),
        input_font: null_mut(),
        brush: null_mut(),
        fields: Vec::new(),
        editing_kind: Kind::Task,
        editing_before: None,
        return_page: Page::Today,
        course_origin: Page::Today,
        import_file: None,
        preview: Entries::new(),
        pending_semester: None,
        status: String::new(),
        sync_status: String::new(),
        sync_due: None,
        job: None,
        save_retry: None,
        app_date: today,
    };
    APP.with(|slot| *slot.borrow_mut() = Some(app));
    let tray_only = std::env::args_os().skip(1).any(|arg| arg == "--tray");
    with_app(|app| {
        app.tray(NIM_ADD);
        if !tray_only {
            app.open();
        }
        app.sync_due = app.state.dav.as_ref().map(|_| Utc::now());
        app.tick();
    });

    let mut msg = MSG::default();
    loop {
        let status = unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) };
        if status == 0 {
            break;
        }
        if status < 0 {
            return Err("Windows 消息循环异常".into());
        }
        let handled = handle_key(&msg);
        if !handled {
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
    Ok(())
}

struct OwnedHandle(HANDLE);
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

struct Popup {
    id: Option<Uuid>,
    entry: Entry,
    until: DateTime<Utc>,
}

struct SyncJob {
    sent: Entries,
    result: Receiver<Result<Synced>>,
}

struct App {
    controller: HWND,
    icon: HICON,
    exe: PathBuf,
    path: PathBuf,
    state: State,
    main: HWND,
    toast: HWND,
    toast_data: Option<Popup>,
    toast_dpi: i32,
    taskbar_message: u32,
    dark: bool,
    startup: bool,
    page: Page,
    day: NaiveDate,
    scroll: usize,
    week_scroll: Option<i32>,
    detail_scroll: i32,
    scrollbar: Option<Scrollbar>,
    scroll_drag: Option<i32>,
    main_hits: Vec<Hit>,
    toast_hits: Vec<Hit>,
    hover: Option<Action>,
    toast_hover: Option<Action>,
    input: Vec<HWND>,
    entry_times: Option<[EntryTime; 2]>,
    picker: Option<Picker>,
    picker_window: HWND,
    picker_hits: Vec<Hit>,
    picker_hover: Option<Action>,
    picker_focus: Option<Action>,
    selects: Vec<usize>,
    input_font: HFONT,
    brush: HBRUSH,
    fields: Vec<Field>,
    editing_kind: Kind,
    editing_before: Option<Entry>,
    return_page: Page,
    course_origin: Page,
    import_file: Option<PathBuf>,
    preview: Entries,
    pending_semester: Option<Semester>,
    status: String,
    sync_status: String,
    sync_due: Option<DateTime<Utc>>,
    job: Option<SyncJob>,
    save_retry: Option<DateTime<Utc>>,
    app_date: NaiveDate,
}

impl App {
    fn tray(&self, command: NOTIFY_ICON_MESSAGE) {
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.controller,
            uID: 1,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: TRAY,
            hIcon: self.icon,
            ..Default::default()
        };
        let tip = wide("课时钟 · 课程与待办提醒");
        data.szTip[..tip.len()].copy_from_slice(&tip);
        unsafe {
            Shell_NotifyIconW(command, &data);
        }
    }

    fn open(&mut self) {
        if self.main.is_null() {
            let (area, dpi) = win::work_area();
            let w = ui::scale(960, dpi).min(area.right - area.left);
            let h = ui::scale(710, dpi).min(area.bottom - area.top);
            let x = area.left + (area.right - area.left - w) / 2;
            let y = area.top + (area.bottom - area.top - h) / 2;
            self.main = create(
                MAIN,
                &wide("课时钟"),
                WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
                0,
                x,
                y,
                w,
                h,
                null_mut(),
            );
            if self.main.is_null() {
                self.status = "无法打开主窗口".into();
                return;
            }
            unsafe {
                DragAcceptFiles(self.main, 1);
            }
            win::frame_theme(self.main, self.dark, true);
            self.rebuild_form();
        }
        unsafe {
            ShowWindow(
                self.main,
                if IsIconic(self.main) != 0 {
                    SW_RESTORE
                } else {
                    SW_SHOW
                },
            );
            SetForegroundWindow(self.main);
        }
        self.repaint();
        self.arm_timer();
    }

    fn close_main(&mut self) {
        self.close_picker();
        self.entry_times = None;
        self.picker_focus = None;
        self.clear_form();
        unsafe {
            DestroyWindow(self.main);
        }
        self.main = null_mut();
        self.page = Page::Today;
        self.scroll = 0;
        self.week_scroll = None;
        self.detail_scroll = 0;
        self.scrollbar = None;
        self.scroll_drag = None;
        self.main_hits.clear();
        self.hover = None;
        self.preview.clear();
        self.arm_timer();
    }

    fn repaint(&self) {
        if !self.main.is_null() {
            unsafe {
                InvalidateRect(self.main, null(), 0);
            }
        }
    }

    fn navigate(&mut self, page: Page) {
        if let Page::Edit(id) = page {
            let entry = id.and_then(|id| self.state.entries.get(&id).cloned());
            if id.is_some() && entry.is_none() {
                self.status = "这条记录已删除".into();
                return;
            }
            if !matches!(self.page, Page::Edit(_)) {
                self.return_page = self.page;
            }
            self.editing_before = entry;
            self.editing_kind = self
                .editing_before
                .as_ref()
                .map_or(Kind::Task, |entry| entry.kind);
        } else if matches!(page, Page::Course(_)) {
            self.course_origin = match self.page {
                Page::Course(_) => self.course_origin,
                Page::Edit(_) if matches!(self.return_page, Page::Course(_)) => self.course_origin,
                Page::Edit(_) => self.return_page,
                origin => origin,
            };
            self.detail_scroll = 0;
        }
        self.set_page(page);
    }

    fn set_page(&mut self, page: Page) {
        self.close_picker();
        self.picker_focus = None;
        self.page = page;
        self.selects.clear();
        self.scroll = 0;
        self.scrollbar = None;
        self.hover = None;
        self.main_hits.clear();
        self.status.clear();
        self.reset_entry_times();
        self.rebuild_form();
        self.repaint();
    }

    fn clear_form(&mut self) {
        unsafe {
            for handle in self.input.drain(..) {
                DestroyWindow(handle);
            }
            if !self.input_font.is_null() {
                DeleteObject(self.input_font);
                self.input_font = null_mut();
            }
            if !self.brush.is_null() {
                DeleteObject(self.brush);
                self.brush = null_mut();
            }
        }
        self.fields.clear();
    }

    fn rebuild_form(&mut self) {
        self.clear_form();
        if self.main.is_null() {
            return;
        }
        let dpi = dpi(self.main);
        let p = Palette::new(self.dark);
        unsafe {
            self.brush = CreateSolidBrush(ui::color(p.field));
            self.input_font = CreateFontW(
                -ui::scale(15, dpi),
                0,
                0,
                0,
                400,
                0,
                0,
                0,
                DEFAULT_CHARSET.into(),
                OUT_DEFAULT_PRECIS.into(),
                CLIP_DEFAULT_PRECIS.into(),
                CLEARTYPE_QUALITY.into(),
                DEFAULT_PITCH.into(),
                wide("Microsoft YaHei UI").as_ptr(),
            );
        }
        let mut specs = Vec::new();
        match self.page {
            Page::Import => {
                let path = self
                    .import_file
                    .as_ref()
                    .map(|path| path.to_string_lossy())
                    .unwrap_or_default();
                specs.push(Input::edit(
                    "文件路径",
                    Bounds(228, 172, 351, 38),
                    &path,
                    "选择、拖入文件，或粘贴路径",
                ));
                specs.push(Input::edit(
                    "第一教学周的周一",
                    Bounds(228, 248, 200, 38),
                    &self
                        .state
                        .semester
                        .first_monday
                        .map(|date| date.format("%Y-%m-%d").to_string())
                        .unwrap_or_default(),
                    "YYYY-MM-DD",
                ));
                specs.push(Input {
                    label: "校区作息",
                    bounds: Bounds(228, 322, 468, 38),
                    value: String::new(),
                    hint: "",
                    options: Campus::OPTIONS.iter().map(|option| option.0).collect(),
                    masked: false,
                    default: if self.state.semester.campus == Campus::OPTIONS[1].0 {
                        1
                    } else if self.state.semester.campus == Campus::OPTIONS[2].0 {
                        2
                    } else {
                        0
                    },
                });
            }
            Page::Settings => {
                let dav = self.state.dav.as_ref();
                specs.push(Input::edit(
                    "WebDAV 目录地址",
                    Bounds(228, 168, 468, 38),
                    dav.map_or("", |s| &s.url),
                    "https://example.com/dav/course-clock/",
                ));
                specs.push(Input::edit(
                    "账号",
                    Bounds(228, 236, 222, 38),
                    dav.map_or("", |s| &s.username),
                    "",
                ));
                let mut password = Input::edit(
                    "应用密码",
                    Bounds(464, 236, 232, 38),
                    "",
                    if dav.is_some() {
                        "留空沿用已保存的密码"
                    } else {
                        ""
                    },
                );
                password.masked = true;
                specs.push(password);
                specs.push(Input::edit(
                    "同步间隔（分钟）",
                    Bounds(228, 300, 222, 38),
                    &dav.map_or(5, |s| s.interval).to_string(),
                    "",
                ));
            }
            Page::Edit(_) => {
                let entry = self.editing_before.as_ref();
                let is_course = self.editing_kind == Kind::Course;
                let lead = if entry.is_none() {
                    if is_course {
                        "10".into()
                    } else {
                        String::new()
                    }
                } else {
                    entry
                        .and_then(|e| e.lead)
                        .map(|lead| lead.to_string())
                        .unwrap_or_default()
                };
                specs.push(Input::edit(
                    "名称",
                    Bounds(228, 142, 468, 38),
                    entry.map_or("", |e| &e.title),
                    "",
                ));
                if is_course {
                    specs.push(Input::edit(
                        "教师",
                        Bounds(228, 208, 222, 38),
                        entry.map_or("", |e| &e.teacher),
                        "可选",
                    ));
                    specs.push(Input::edit(
                        "地点",
                        Bounds(464, 208, 232, 38),
                        entry.map_or("", |e| &e.place),
                        "可选",
                    ));
                } else {
                    specs.push(Input::edit(
                        "地点",
                        Bounds(228, 208, 468, 38),
                        entry.map_or("", |e| &e.place),
                        "可选",
                    ));
                }
                specs.push(Input::edit(
                    "提前分钟",
                    Bounds(550, 334, 146, 38),
                    &lead,
                    "",
                ));
                if !is_course {
                    specs.push(Input {
                        label: "重复",
                        bounds: Bounds(228, 396, 348, 38),
                        value: String::new(),
                        hint: "",
                        options: RepeatRule::OPTIONS.to_vec(),
                        masked: false,
                        default: entry
                            .and_then(|e| e.repeat)
                            .and_then(|repeat| {
                                RepeatRule::OPTIONS
                                    .iter()
                                    .position(|label| *label == repeat.label())
                            })
                            .unwrap_or(0),
                    });
                }
            }
            _ => {}
        };
        let mut slot = 0;
        for spec in specs {
            if spec.options.is_empty() {
                let bounds = spec.bounds.rect(dpi);
                let inset = ui::scale(4, dpi);
                let height = ui::scale(20, dpi);
                let handle = create(
                    "EDIT",
                    &wide(&spec.value),
                    WS_CHILD
                        | WS_VISIBLE
                        | WS_TABSTOP
                        | ES_AUTOHSCROLL as u32
                        | if spec.masked { ES_PASSWORD as u32 } else { 0 },
                    0,
                    bounds.left,
                    (bounds.top + bounds.bottom - height) / 2,
                    bounds.right - bounds.left,
                    height,
                    self.main,
                );
                let pad = ui::scale(12 - inset, dpi);
                unsafe {
                    SendMessageW(handle, WM_SETFONT, self.input_font as usize, 1);
                    SendMessageW(handle, EM_LIMITTEXT, 2048, 0);
                    SendMessageW(
                        handle,
                        EM_SETMARGINS,
                        (EC_LEFTMARGIN | EC_RIGHTMARGIN) as usize,
                        pad as isize | ((pad as isize) << 16),
                    );
                    if !spec.hint.is_empty() {
                        SendMessageW(
                            handle,
                            EM_SETCUEBANNER,
                            0,
                            wide(spec.hint).as_ptr() as isize,
                        );
                    }
                }
                self.input.push(handle);
            }
            let index = slot;
            let selected = if spec.options.is_empty() {
                0
            } else {
                slot += 1;
                self.selects
                    .get(index)
                    .copied()
                    .unwrap_or(spec.default)
                    .min(spec.options.len() - 1)
            };
            self.fields.push(Field {
                label: spec.label,
                bounds: spec.bounds,
                options: spec.options,
                selected,
                slot: index,
            });
        }
        if matches!(self.page, Page::Edit(_)) {
            if let Some(handle) = self.input.first() {
                unsafe {
                    SetFocus(*handle);
                    SendMessageW(*handle, EM_SETSEL, 0, -1);
                }
            }
        }
    }

    fn reset_entry_times(&mut self) {
        if !matches!(self.page, Page::Edit(_)) {
            self.entry_times = None;
            return;
        }
        let is_course = self.editing_kind == Kind::Course;
        let entry = self.editing_before.as_ref();
        let start = entry.and_then(|entry| entry.start);
        let end = entry.and_then(|entry| entry.end);
        let start_value = start
            .map(|time| time.with_timezone(&Local).naive_local())
            .unwrap_or_else(|| {
                if is_course {
                    self.day.and_hms_opt(8, 0, 0).unwrap()
                } else {
                    let now = Local::now();
                    now.date_naive()
                        .and_hms_opt(now.hour(), now.minute(), 0)
                        .unwrap()
                }
            });
        let end_value = end
            .map(|time| time.with_timezone(&Local).naive_local())
            .unwrap_or(start_value + Duration::hours(1));
        self.entry_times = Some([
            EntryTime {
                value: start_value,
                enabled: is_course || start.is_some(),
            },
            EntryTime {
                value: end_value,
                enabled: end.is_some(),
            },
        ]);
    }

    fn open_picker(&mut self, index: usize, kind: PickerKind) -> Result<()> {
        let Some(times) = self.entry_times else {
            return Ok(());
        };
        let Some(value) = times.get(index).copied() else {
            return Ok(());
        };
        if index == 1 && !times[0].enabled {
            return Ok(());
        }
        if kind == PickerKind::Time && !value.enabled {
            return Ok(());
        }
        self.close_picker();
        let dpi = dpi(self.main);
        let field = kind.field(index).rect(dpi);
        let (width, height) = kind.popup_size();
        let (w, h) = (ui::scale(width, dpi), ui::scale(height, dpi));
        let gap = ui::scale(6, dpi);
        let mut point = POINT {
            x: field.left,
            y: field.top,
        };
        let mut monitor = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let area = unsafe {
            ClientToScreen(self.main, &mut point);
            let screen = MonitorFromWindow(self.main, MONITOR_DEFAULTTONEAREST);
            if GetMonitorInfoW(screen, &mut monitor) != 0 {
                monitor.rcWork
            } else {
                win::work_area().0
            }
        };
        let x = point.x.clamp(area.left, (area.right - w).max(area.left));
        let below = point.y + field.bottom - field.top + gap;
        let y = if below + h <= area.bottom {
            below
        } else {
            point.y - gap - h
        };
        let y = y.clamp(area.top, (area.bottom - h).max(area.top));
        self.picker_window = create(
            PICKER,
            &wide(if kind == PickerKind::Date {
                "选择日期"
            } else {
                "选择时刻"
            }),
            WS_POPUP,
            WS_EX_TOOLWINDOW,
            x,
            y,
            w,
            h,
            self.main,
        );
        if self.picker_window.is_null() {
            return Err("无法打开日期和时间选择器".into());
        }
        self.picker = Some(Picker {
            index,
            kind,
            value: value.value,
        });
        self.picker_focus = Some(Action::OpenPicker(index, kind));
        win::round_region(self.picker_window, ui::scale(14, dpi));
        unsafe {
            ShowWindow(self.picker_window, SW_SHOW);
            SetFocus(self.picker_window);
        }
        Ok(())
    }

    fn close_picker(&mut self) {
        let window = std::mem::replace(&mut self.picker_window, null_mut());
        self.picker = None;
        self.picker_hits.clear();
        self.picker_hover = None;
        if !window.is_null() {
            unsafe {
                let had_focus = GetFocus() == window;
                DestroyWindow(window);
                if had_focus {
                    SetFocus(self.main);
                }
            }
        }
    }

    fn confirm_picker(&mut self) {
        if let (Some(picker), Some(times)) = (self.picker, &mut self.entry_times) {
            times[picker.index] = EntryTime {
                value: picker.value,
                enabled: true,
            };
            if picker.index == 0 && !times[1].enabled {
                times[1].value = picker.value + Duration::hours(1);
            }
        }
        self.close_picker();
    }

    fn form_values(&self) -> Vec<String> {
        self.input
            .iter()
            .map(|&handle| unsafe {
                let mut text = vec![0_u16; GetWindowTextLengthW(handle) as usize + 1];
                let length = GetWindowTextW(handle, text.as_mut_ptr(), text.len() as i32);
                String::from_utf16_lossy(&text[..length as usize])
            })
            .collect()
    }

    fn paint(&mut self) {
        let now = Local::now();
        let conflict_ids = model::overlapping(&self.state.entries, self.day);
        let week_start =
            self.day - Duration::days(self.day.weekday().num_days_from_monday().into());
        let week_end = week_start + Duration::days(6);
        let all = if self.page == Page::Preview {
            &self.preview
        } else {
            &self.state.entries
        };
        let mut rows: Vec<_> = all
            .iter()
            .filter_map(|(&id, entry)| {
                let date = entry
                    .start
                    .map(|start| start.with_timezone(&Local).date_naive());
                let show = match self.page {
                    Page::Today => {
                        date == Some(now.date_naive()) || entry.kind == Kind::Task && date.is_none()
                    }
                    Page::Week => {
                        entry.kind == Kind::Course
                            && date.is_some_and(|date| {
                                date <= week_end
                                    && (date >= week_start
                                        || entry.course_end().is_some_and(|end| {
                                            end.with_timezone(&Local).date_naive() >= week_start
                                        }))
                            })
                    }
                    Page::Tasks => entry.kind == Kind::Task,
                    Page::Conflicts => entry.conflict || conflict_ids.contains(&id),
                    Page::Preview => true,
                    _ => false,
                };
                show.then_some((id, entry))
            })
            .collect();
        rows.sort_by_key(|(_, entry)| (entry.done, entry.start.is_none(), entry.start));
        self.scroll = self.scroll.min(rows.len().saturating_sub(1));
        let next = if self.page == Page::Today {
            self.state
                .entries
                .values()
                .filter(|entry| {
                    entry.kind == Kind::Course
                        && !entry.done
                        && entry
                            .start
                            .is_some_and(|date| date >= now.with_timezone(&Utc))
                })
                .min_by_key(|entry| entry.start)
        } else {
            None
        };
        let sync_status = if self.sync_status.is_empty() {
            "在两台电脑上填写同一个 WebDAV 目录。"
        } else {
            &self.sync_status
        };
        let semester_label = self
            .state
            .semester
            .first_monday
            .map(|monday| {
                let week = ((self.day - monday).num_days().div_euclid(7) + 1).max(1);
                format!("{}  ·  第 {week} 周", self.state.semester.name)
            })
            .unwrap_or_else(|| self.state.semester.name.clone());
        let editing_course = self.editing_kind == Kind::Course;
        let active_page = match self.page {
            Page::Preview => Page::Import,
            Page::Course(_) => self.course_origin,
            Page::Edit(_) if !editing_course => Page::Tasks,
            Page::Edit(_) if matches!(self.return_page, Page::Course(_)) => self.course_origin,
            Page::Edit(_) => Page::Week,
            page => page,
        };
        let course = if let Page::Course(id) = self.page {
            self.state.entries.get(&id)
        } else {
            None
        };
        let course_colors = if self.page == Page::Week {
            ui::course_colors(
                self.state
                    .entries
                    .values()
                    .filter(|entry| entry.kind == Kind::Course)
                    .map(|entry| entry.title.as_str()),
            )
        } else {
            Default::default()
        };
        let (hits, scrollbar) = ui::draw_main(
            self.main,
            dpi(self.main),
            Palette::new(self.dark),
            View {
                page: self.page,
                active_page,
                day: self.day,
                rows: &rows,
                course_colors: &course_colors,
                next,
                scroll: self.scroll,
                week_scroll: self.week_scroll,
                detail_scroll: self.detail_scroll,
                course,
                status: &self.status,
                sync_status,
                connected: self.state.dav.is_some(),
                startup: self.startup,
                editing_course,
                entry_times: self.entry_times,
                picker_focus: self
                    .picker_focus
                    .filter(|_| unsafe { GetFocus() } == self.main),
                fields: &self.fields,
                hover: self.hover,
                semester: &semester_label,
            },
        );
        self.main_hits = hits;
        self.scrollbar = scrollbar;
        if let Some(bar) = &self.scrollbar {
            if self.page == Page::Week {
                self.week_scroll = Some(bar.offset);
            } else {
                self.detail_scroll = bar.offset;
            }
        }
    }

    fn selected_file(&mut self, path: PathBuf) {
        if !is_ext(&path, "csv")
            && !is_ext(&path, "ics")
            && !is_ext(&path, "html")
            && !is_ext(&path, "htm")
        {
            self.set_page(Page::Import);
            self.status = "请选择 CSV 或 ICS 文件".into();
            return;
        }
        self.import_file = Some(path.clone());
        if self.page == Page::Import {
            if let Some(&handle) = self.input.first() {
                unsafe {
                    SetWindowTextW(handle, win::wide_os(path.as_os_str()).as_ptr());
                }
            }
            self.status.clear();
            self.repaint();
        } else {
            self.set_page(Page::Import);
        }
    }

    fn open_preview(&mut self) -> Result<()> {
        let values = self.form_values();
        let path = values
            .first()
            .map(|text| text.trim().trim_matches('"'))
            .filter(|text| !text.is_empty())
            .map(PathBuf::from)
            .ok_or("请先选择课表文件")?;
        use std::io::Read;
        let mut bytes = Vec::new();
        fs::File::open(&path)
            .map_err(|error| format!("无法打开导入文件：{error}"))?
            .take(MAX_IMPORT_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if bytes.len() > MAX_IMPORT_BYTES {
            return Err("课表文件超出 32 MB".into());
        }
        let mut source_path = path.clone();
        if (is_ext(&path, "html") || is_ext(&path, "htm"))
            && !bytes
                .windows(b"lessonInfo".len())
                .any(|window| window == b"lessonInfo")
        {
            let candidate = path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join(format!(
                    "{}_files",
                    path.file_stem()
                        .and_then(|stem| stem.to_str())
                        .unwrap_or("首页")
                ))
                .join("course-table.html");
            if candidate.is_file() {
                bytes =
                    fs::read(&candidate).map_err(|error| format!("无法读取课表子页面：{error}"))?;
                if bytes.len() > MAX_IMPORT_BYTES {
                    return Err("课表文件超出 32 MB".into());
                }
                source_path = candidate;
            }
        }
        let mut notes = String::new();
        let campus_index = self.selects.first().copied().unwrap_or(0);
        let (campus_label, campus) = *Campus::OPTIONS.get(campus_index).ok_or("请选择校区作息")?;
        self.preview = if is_ext(&path, "csv") {
            let monday = values
                .get(1)
                .and_then(|text| NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d").ok())
                .filter(|date| (1900..=2200).contains(&date.year()))
                .ok_or("请填写第一教学周的周一，格式 YYYY-MM-DD")?;
            self.pending_semester = Some(Semester {
                name: "西工大课程".into(),
                first_monday: Some(monday),
                campus: campus_label.into(),
            });
            import::csv(&bytes, monday, campus)?
        } else if is_ext(&path, "ics") {
            let calendar = import::ics(&bytes, Utc::now())?;
            if calendar.recurring {
                notes = "重复事件展开至未来约 18 个月".into();
            }
            if calendar.skipped != 0 {
                if !notes.is_empty() {
                    notes.push_str(" · ");
                }
                notes.push_str(&format!("跳过 {} 个全天或无法解析的事件", calendar.skipped));
            }
            calendar.entries
        } else if is_ext(&path, "html") || is_ext(&path, "htm") {
            let monday = self
                .form_values()
                .get(1)
                .and_then(|text| NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d").ok())
                .filter(|date| (1900..=2200).contains(&date.year()))
                .ok_or("请填写第一教学周的周一，格式 YYYY-MM-DD")?;
            self.pending_semester = Some(Semester {
                name: "西工大课程".into(),
                first_monday: Some(monday),
                campus: campus_label.into(),
            });
            import::html(&bytes, monday, campus)?
        } else {
            return Err("请选择 CSV 或 ICS 文件".into());
        };
        self.import_file = Some(source_path);
        self.set_page(Page::Preview);
        self.status = notes;
        Ok(())
    }

    fn save_entry(&mut self) -> Result<()> {
        let Page::Edit(id) = self.page else {
            return Ok(());
        };
        let values = self.form_values();
        let is_course = self.editing_kind == Kind::Course;
        let (title, teacher, place, lead, repeat) = if is_course {
            let [title, teacher, place, lead] = values.as_slice() else {
                return Err("无法读取课程表单内容".into());
            };
            (title, teacher.as_str(), place, lead, None::<RepeatRule>)
        } else {
            let [title, place, lead] = values.as_slice() else {
                return Err("无法读取待办表单内容".into());
            };
            let repeat = self.selects.first().map_or_else(
                || self.editing_before.as_ref().and_then(|entry| entry.repeat),
                |&index| {
                    RepeatRule::OPTIONS
                        .get(index)
                        .and_then(|label| RepeatRule::from_label(label))
                },
            );
            (title, "", place, lead, repeat)
        };
        let [start, end] = self.entry_times.ok_or("无法读取日期和时间")?;
        let read = |field: EntryTime, saved: Option<DateTime<Utc>>| {
            field
                .enabled
                .then(|| {
                    saved
                        .filter(|time| time.with_timezone(&Local).naive_local() == field.value)
                        .map_or_else(|| model::local_datetime(field.value), Ok)
                })
                .transpose()
        };
        let start = read(
            start,
            self.editing_before.as_ref().and_then(|entry| entry.start),
        )?;
        let end = read(
            end,
            self.editing_before.as_ref().and_then(|entry| entry.end),
        )?;
        let lead = if lead.trim().is_empty() {
            None
        } else {
            Some(
                lead.trim()
                    .parse::<u32>()
                    .ok()
                    .filter(|lead| *lead <= 10_080)
                    .ok_or("提醒提前量须为 0–10080 分钟")?,
            )
        };
        if let Some(id) = id
            && self.state.entries.get(&id) != self.editing_before.as_ref()
        {
            return Err("这条记录在编辑期间已被同步更新，请重新打开后编辑".into());
        }
        let entry = Entry {
            kind: self.editing_kind,
            title: title.trim().into(),
            teacher: teacher.trim().into(),
            place: place.trim().into(),
            start,
            end,
            lead,
            done: self.editing_before.as_ref().is_some_and(|entry| entry.done),
            repeat,
            conflict: false,
        };
        if !entry.validates() {
            return Err("请检查名称和时间。课程及提醒需要开始时间，结束时间需晚于开始".into());
        }
        let mut entries = self.state.entries.clone();
        entries.insert(id.unwrap_or_else(Uuid::new_v4), entry);
        self.commit(entries)?;
        self.set_page(self.return_page);
        self.status = "已保存".into();
        Ok(())
    }

    fn commit(&mut self, entries: Entries) -> Result<()> {
        let mut state = self.state.clone();
        state.update_entries(entries);
        model::save(&self.path, &state)?;
        self.state = state;
        self.save_retry = None;
        if self.state.dav.is_some() {
            self.sync_due = Some(Utc::now() + Duration::seconds(3));
        }
        self.tick();
        Ok(())
    }

    fn toggle_entry(&mut self, id: Uuid) -> Result<()> {
        let mut entries = self.state.entries.clone();
        if let Some(entry) = entries.get_mut(&id) {
            entry.done = !entry.done;
            let next = if entry.done {
                entry.repeat.and_then(|repeat| {
                    let start = entry.start?;
                    let mut next = entry.clone();
                    next.start = Some(repeat.next(start));
                    next.end = entry
                        .end
                        .and_then(|end| end.checked_add_signed(next.start.unwrap() - start));
                    next.done = false;
                    next.conflict = false;
                    Some((Uuid::new_v4(), next))
                })
            } else {
                None
            };
            self.commit(entries)?;
            if let Some((next_id, next_entry)) = next {
                let mut entries = self.state.entries.clone();
                entries.insert(next_id, next_entry);
                self.commit(entries)?;
            }
            if self.page == Page::Edit(Some(id)) {
                self.navigate(Page::Edit(Some(id)));
            }
        }
        Ok(())
    }

    fn save_dav(&mut self) -> Result<()> {
        if self.job.is_some() {
            return Err("当前同步正在进行，请稍后保存".into());
        }
        let values = self.form_values();
        let [url, username, password, interval] = values.as_slice() else {
            return Err("无法读取同步设置".into());
        };
        let url = sync::endpoint(url)?.to_string();
        let interval = interval
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|value| (1..=1_440).contains(value))
            .ok_or("同步间隔须为 1–1440 分钟")?;
        let username = username.trim().to_string();
        if username.contains(':') {
            return Err("WebDAV 账号不能含英文冒号".into());
        }
        let same = self
            .state
            .dav
            .as_ref()
            .filter(|old| old.url == url && old.username == username);
        let password = match same {
            Some(old) if password.is_empty() => old.password.clone(),
            _ => sync::protect(password)?,
        };
        let mut state = self.state.clone();
        if self.state.dav.as_ref().is_none_or(|old| old.url != url) {
            state.base.clear();
            state.dav_verified = false;
        }
        state.dav = Some(DavSettings {
            url,
            username,
            password,
            interval,
        });
        model::save(&self.path, &state)?;
        self.state = state;
        self.rebuild_form();
        self.start_sync();
        Ok(())
    }

    fn disconnect(&mut self) -> Result<()> {
        if self.job.is_some() {
            return Err("当前同步正在进行，请稍后断开".into());
        }
        let mut state = self.state.clone();
        state.dav = None;
        state.dav_verified = false;
        state.base.clear();
        model::save(&self.path, &state)?;
        self.state = state;
        self.sync_due = None;
        self.sync_status.clear();
        self.rebuild_form();
        Ok(())
    }

    fn start_sync(&mut self) {
        if self.job.is_some() {
            return;
        }
        let Some(settings) = self.state.dav.clone() else {
            return;
        };
        self.sync_due = None;
        self.sync_status = "正在同步…".into();
        let verified = self.state.dav_verified;
        let base = self.state.base.clone();
        let sent = self.state.entries.clone();
        let local = sent.clone();
        let semester = self.state.semester.clone();
        let (tx, result) = mpsc::channel();
        let window = self.controller as usize;
        match std::thread::Builder::new()
            .name("course-clock-dav".into())
            .spawn(move || {
                let _ = tx.send(sync::sync(&settings, verified, &base, &local, &semester));
                unsafe {
                    PostMessageW(window as HWND, SYNC_DONE, 0, 0);
                }
            }) {
            Ok(_) => self.job = Some(SyncJob { sent, result }),
            Err(error) => {
                self.sync_status = format!("无法启动同步：{error}");
                self.sync_due = Some(Utc::now() + Duration::minutes(1));
            }
        }
        self.repaint();
        self.arm_timer();
    }

    fn finish_sync(&mut self) {
        let Some(job) = self.job.take() else { return };
        let interval = self
            .state
            .dav
            .as_ref()
            .map_or(5, |settings| settings.interval);
        self.sync_due = Some(Utc::now() + Duration::minutes(interval.into()));
        let result = job
            .result
            .try_recv()
            .unwrap_or_else(|error| Err(format!("同步任务中断：{error}")));
        self.sync_status = match result {
            Ok(remote) => {
                let (entries, rebase_conflicts) =
                    model::merge(&job.sent, &self.state.entries, &remote.entries);
                let mut state = self.state.clone();
                state.update_entries(entries);
                state.base = remote.entries;
                if let Some(semester) = remote.semester {
                    state.semester = semester;
                }
                state.dav_verified = true;
                if let Err(error) = model::save(&self.path, &state) {
                    error
                } else {
                    if state.entries != state.base {
                        self.sync_due = Some(Utc::now() + Duration::seconds(3));
                    }
                    self.state = state;
                    self.save_retry = None;
                    let time = Local::now().format("%H:%M");
                    let conflicts = remote.conflicts + rebase_conflicts;
                    if conflicts > 0 {
                        format!("{time} 已同步 · 保留了 {conflicts} 处冲突修改")
                    } else {
                        format!("{time} 已同步")
                    }
                }
            }
            Err(error) => error,
        };
        self.repaint();
        self.tick();
    }

    fn show_toast(&mut self, id: Option<Uuid>, entry: Entry) -> bool {
        self.close_toast();
        let (area, dpi) = win::work_area();
        self.toast_dpi = dpi;
        let size = |n| ui::scale(n, dpi);
        let w = size(ui::TOAST_WIDTH);
        let h = size(ui::TOAST_HEIGHT);
        self.toast = create(
            TOAST,
            &wide("课时钟提醒"),
            WS_POPUP,
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            area.right - w - size(18),
            area.top + size(18),
            w,
            h,
            null_mut(),
        );
        if self.toast.is_null() {
            return false;
        }
        self.toast_data = Some(Popup {
            id,
            entry,
            until: Utc::now() + Duration::seconds(32),
        });
        win::frame_theme(self.toast, self.dark, true);
        win::round_region(self.toast, ui::scale(13, dpi));
        unsafe {
            ShowWindow(self.toast, SW_SHOWNOACTIVATE);
            InvalidateRect(self.toast, null(), 0);
        }
        true
    }

    fn close_toast(&mut self) {
        if !self.toast.is_null() {
            unsafe {
                DestroyWindow(self.toast);
            }
            self.toast = null_mut();
        }
        self.toast_data = None;
        self.toast_hits.clear();
        self.toast_hover = None;
    }

    fn tick(&mut self) {
        let now = Utc::now();
        let date = now.with_timezone(&Local).date_naive();
        if date != self.app_date {
            if self.day == self.app_date {
                self.select_day(date);
            }
            self.app_date = date;
        }
        if self.toast_data.as_ref().is_some_and(|popup| {
            popup.until <= now
                || popup.id.is_some_and(|id| {
                    self.state
                        .entries
                        .get(&id)
                        .is_none_or(|entry| entry.done || entry != &popup.entry)
                })
        }) {
            self.close_toast();
        }
        if self.toast_data.is_none() {
            let due = self
                .state
                .entries
                .iter()
                .filter_map(|(id, entry)| {
                    let due = self.state.alarm_for(id, entry)?;
                    (due <= now && alarm_deadline(&self.state, id, entry, due) >= now)
                        .then_some((due, *id, entry))
                })
                .min_by_key(|(due, id, _)| (*due, *id))
                .map(|(due, id, entry)| (due, id, entry.clone()));
            if let Some((due, id, entry)) = due
                && now >= self.save_retry.unwrap_or(now)
            {
                if self.show_toast(Some(id), entry) {
                    let snoozed = self
                        .state
                        .alarms
                        .get(&id)
                        .is_some_and(|alarm| alarm.snoozed);
                    self.state.alarms.insert(
                        id,
                        Alarm {
                            due,
                            snoozed,
                            fired: true,
                        },
                    );
                    self.persist_alarms(now);
                } else {
                    self.save_retry = Some(now + Duration::minutes(1));
                }
            }
        }
        if self.save_retry.is_some_and(|retry| retry <= now) {
            self.persist_alarms(now);
        }
        if self.sync_due.is_some_and(|due| due <= now) {
            self.start_sync();
        }
        self.repaint();
        self.arm_timer();
    }

    fn persist_alarms(&mut self, now: DateTime<Utc>) {
        match model::save(&self.path, &self.state) {
            Ok(()) => self.save_retry = None,
            Err(error) => {
                self.status = error;
                self.save_retry = Some(now + Duration::minutes(1));
            }
        }
    }

    fn arm_timer(&self) {
        unsafe {
            KillTimer(self.controller, TIMER);
        }
        let now = Utc::now();
        let mut next = self.sync_due.filter(|_| self.job.is_none());
        let mut consider = |time: DateTime<Utc>| {
            if next.is_none_or(|current| time < current) {
                next = Some(time);
            }
        };
        if let Some(retry) = self.save_retry {
            consider(retry);
        }
        if let Some(popup) = &self.toast_data {
            consider(popup.until);
        } else {
            for (id, entry) in &self.state.entries {
                if let Some(due) = self.state.alarm_for(id, entry)
                    && alarm_deadline(&self.state, id, entry, due) >= now
                {
                    consider(due.max(self.save_retry.unwrap_or(due)));
                }
            }
        }
        if !self.main.is_null() && unsafe { IsIconic(self.main) } == 0 {
            let tomorrow = self.app_date.succ_opt();
            if let Some(tomorrow) =
                tomorrow.and_then(|date| model::local_time(&format!("{date} 00:00")).ok())
            {
                consider(tomorrow);
            }
            if self.page == Page::Today {
                for entry in self
                    .state
                    .entries
                    .values()
                    .filter(|entry| entry.kind == Kind::Course && !entry.done)
                {
                    if let Some(time) = entry.start.filter(|time| *time > now) {
                        consider(time);
                    }
                }
            }
        }
        if let Some(next) = next {
            let millis = (next - now)
                .num_milliseconds()
                .saturating_add(5)
                .clamp(10, i32::MAX as i64);
            unsafe {
                SetTimer(self.controller, TIMER, millis as u32, None);
            }
        }
    }

    fn act(&mut self, action: Action) -> Result<()> {
        match action {
            Action::Page(page) => self.navigate(page),
            Action::OpenPicker(index, kind) => self.open_picker(index, kind)?,
            Action::ClosePicker => self.close_picker(),
            Action::ConfirmPicker => self.confirm_picker(),
            Action::ClearDate(index) => {
                if let Some(times) = &mut self.entry_times {
                    if index == 1 {
                        times[1].enabled = false;
                    } else if self.editing_kind == Kind::Task {
                        times[0].enabled = false;
                        times[1].enabled = false;
                    }
                }
            }
            Action::PickerDate(date) => {
                if let Some(picker) = &mut self.picker {
                    picker.value = date.and_time(picker.value.time());
                    self.confirm_picker();
                }
            }
            Action::PickerMonth(_)
            | Action::PickerStep(_)
            | Action::PickerHour(_)
            | Action::PickerMinute(_) => {
                if let Some(picker) = &mut self.picker {
                    let value = picker.value;
                    picker.value = match action {
                        Action::PickerMonth(delta) if delta < 0 => {
                            value.checked_sub_months(Months::new(delta.unsigned_abs()))
                        }
                        Action::PickerMonth(delta) => {
                            value.checked_add_months(Months::new(delta as u32))
                        }
                        Action::PickerStep(delta) if picker.kind == PickerKind::Date => {
                            value.checked_add_signed(Duration::days(delta.into()))
                        }
                        Action::PickerStep(delta) => {
                            let (time, _) = value
                                .time()
                                .overflowing_add_signed(Duration::minutes(delta.into()));
                            Some(value.date().and_time(time))
                        }
                        Action::PickerHour(hour) => value.with_hour(hour),
                        Action::PickerMinute(minute) => value.with_minute(minute),
                        _ => None,
                    }
                    .unwrap_or(value);
                }
            }
            Action::Back => {
                let page = match self.page {
                    Page::Course(_) => self.course_origin,
                    Page::Edit(_) => self.return_page,
                    Page::Preview => Page::Import,
                    _ => return Ok(()),
                };
                self.set_page(page);
            }
            Action::NewCourse => {
                self.return_page = self.page;
                self.editing_kind = Kind::Course;
                self.editing_before = None;
                self.set_page(Page::Edit(None));
            }
            Action::Day(date) => {
                self.select_day(date);
                self.set_page(Page::Week);
            }
            Action::Choose(slot, option) => {
                if slot >= self.selects.len() {
                    self.selects.resize(slot + 1, 0);
                }
                self.selects[slot] = option;
                for field in &mut self.fields {
                    if field.options.is_empty() {
                        field.selected = 0;
                    } else {
                        field.selected = self
                            .selects
                            .get(field.slot)
                            .copied()
                            .unwrap_or(0)
                            .min(field.options.len() - 1);
                    }
                }
                self.repaint();
            }
            Action::ParseImport => self.open_preview()?,
            Action::CommitImport => {
                let mut entries = self.state.entries.clone();
                let stale: Vec<Uuid> = entries
                    .iter()
                    .filter(|(_, old)| {
                        old.kind == Kind::Course
                            && self.preview.values().any(|fresh| {
                                fresh.start == old.start
                                    && old.title != fresh.title
                                    && old.title.trim_start_matches("研究生").trim()
                                        == fresh.title.trim()
                            })
                    })
                    .map(|(id, _)| *id)
                    .collect();
                for id in &stale {
                    entries.remove(id);
                }
                let mut count = 0;
                for (&id, source) in &self.preview {
                    let mut entry = source.clone();
                    if let Some(old) = entries.get(&id) {
                        entry.lead = old.lead;
                    }
                    if entries.insert(id, entry.clone()).as_ref() != Some(&entry) {
                        count += 1;
                    }
                }
                self.commit(entries)?;
                if let Some(semester) = self.pending_semester.take() {
                    self.state.semester = semester;
                    model::save(&self.path, &self.state)?;
                }
                self.preview.clear();
                self.day = self.app_date;
                self.week_scroll = None;
                self.set_page(Page::Week);
                self.status = format!("已新增或更新 {count} 次课程");
            }
            Action::SaveEntry => self.save_entry()?,
            Action::ToggleEntry(id) => self.toggle_entry(id)?,
            Action::DeleteEntry(id) => {
                if self.page != Page::Edit(Some(id)) {
                    return Ok(());
                }
                if self.state.entries.get(&id) != self.editing_before.as_ref() {
                    return Err("这条记录在编辑期间已被同步更新，请重新打开后操作".into());
                }
                let mut entries = self.state.entries.clone();
                entries.remove(&id);
                self.commit(entries)?;
                let return_page = if self.return_page == Page::Course(id) {
                    self.course_origin
                } else {
                    self.return_page
                };
                self.set_page(return_page);
                self.status = "已删除".into();
            }
            Action::SaveDav => self.save_dav()?,
            Action::Backup => {
                let path = model::backup(&self.path, &self.state)?;
                self.status = format!(
                    "备份已保存：{}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                );
            }
            Action::Restore => {
                let backup = win::pick_backup(self.main)?;
                let restored = model::open_state(&backup)?;
                if restored.entries.values().any(|entry| !entry.validates()) {
                    return Err("备份中存在无效记录，已停止恢复".into());
                }
                let _ = model::backup(&self.path, &self.state)?;
                model::save(&self.path, &restored)?;
                self.state = restored;
                self.status = "已恢复备份，原数据已另存为备份".into();
                self.rebuild_form();
            }
            Action::ResolveConflict(id) => {
                let mut entries = self.state.entries.clone();
                if let Some(entry) = entries.get_mut(&id) {
                    entry.conflict = false;
                }
                self.commit(entries)?;
            }
            Action::Disconnect => self.disconnect()?,
            Action::Sync => self.start_sync(),
            Action::Startup => self.startup = win::startup(&self.exe, Some(!self.startup))?,
            Action::TestToast => {
                let start = Utc::now() + Duration::minutes(10);
                if self
                    .toast_data
                    .as_ref()
                    .is_some_and(|item| item.id.is_some())
                {
                    return Err("已有一条提醒正在显示".into());
                }
                if !self.show_toast(
                    None,
                    Entry {
                        kind: Kind::Course,
                        title: "数据结构".into(),
                        teacher: "示例教师".into(),
                        place: "长安校区 · 教 A101".into(),
                        start: Some(start),
                        end: Some(start + Duration::minutes(100)),
                        lead: Some(10),
                        done: false,
                        repeat: None,
                        conflict: false,
                    },
                ) {
                    return Err("无法打开提醒窗口".into());
                }
            }
            Action::OpenReminder | Action::Snooze | Action::DoneReminder | Action::Dismiss => {
                let id = self.toast_data.as_ref().and_then(|popup| popup.id);
                if let Some(id) = id {
                    match action {
                        Action::OpenReminder => {
                            self.open();
                            let page = if self
                                .state
                                .entries
                                .get(&id)
                                .is_some_and(|entry| entry.kind == Kind::Course)
                            {
                                Page::Course(id)
                            } else {
                                Page::Edit(Some(id))
                            };
                            self.navigate(page);
                        }
                        Action::Snooze => {
                            let now = Utc::now();
                            self.state.alarms.insert(
                                id,
                                Alarm {
                                    due: now + Duration::minutes(5),
                                    snoozed: true,
                                    fired: false,
                                },
                            );
                            self.persist_alarms(now);
                        }
                        Action::DoneReminder => self.toggle_entry(id)?,
                        _ => {}
                    }
                }
                self.close_toast();
                self.tick();
            }
            Action::PickFile => {}
        }
        self.repaint();
        if !self.picker_window.is_null() {
            unsafe { InvalidateRect(self.picker_window, null(), 0) };
        }
        self.arm_timer();
        Ok(())
    }

    fn select_day(&mut self, day: NaiveDate) {
        let monday = self.day - Duration::days(self.day.weekday().num_days_from_monday().into());
        if !(monday..monday + Duration::days(7)).contains(&day) {
            self.week_scroll = None;
        }
        self.day = day;
    }

    fn scroll_to(&mut self, offset: i32) {
        let Some(bar) = &self.scrollbar else { return };
        let offset = offset.clamp(0, bar.limit);
        match self.page {
            Page::Week => self.week_scroll = Some(offset),
            Page::Course(_) => self.detail_scroll = offset,
            _ => return,
        }
        self.repaint();
    }
}

fn alarm_deadline(state: &State, id: &Uuid, entry: &Entry, due: DateTime<Utc>) -> DateTime<Utc> {
    let start = entry.start.unwrap_or(due);
    let deadline = match entry.kind {
        Kind::Course => entry.end.unwrap_or(start + Duration::minutes(20)),
        Kind::Task => start + Duration::hours(24),
    };
    if state.alarms.get(id).is_some_and(|alarm| alarm.snoozed) {
        deadline.max(due + Duration::hours(1))
    } else {
        deadline
    }
}

struct Input {
    label: &'static str,
    bounds: Bounds,
    value: String,
    hint: &'static str,
    options: Vec<&'static str>,
    masked: bool,
    default: usize,
}

impl Input {
    fn edit(label: &'static str, bounds: Bounds, value: &str, hint: &'static str) -> Self {
        Self {
            label,
            bounds,
            value: value.into(),
            hint,
            options: Vec::new(),
            masked: false,
            default: 0,
        }
    }
}

fn is_ext(path: &std::path::Path, ext: &str) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case(ext))
}

fn dpi(window: HWND) -> i32 {
    unsafe { GetDpiForWindow(window) }.max(96) as i32
}

fn with_app<T>(f: impl FnOnce(&mut App) -> T) -> Option<T> {
    APP.with(|slot| slot.try_borrow_mut().ok()?.as_mut().map(f))
}

fn handle_key(msg: &MSG) -> bool {
    if msg.message != WM_KEYDOWN {
        return false;
    }
    let Some((main, page, fields, focus, times)) = with_app(|app| {
        (
            app.main,
            app.page,
            app.input.clone(),
            app.picker_focus,
            app.entry_times,
        )
    }) else {
        return false;
    };
    if main.is_null() || (msg.hwnd != main && unsafe { IsChild(main, msg.hwnd) } == 0) {
        return false;
    }
    let key = msg.wParam as u16;
    match key {
        VK_TAB if !fields.is_empty() => {
            let mut stops: Vec<_> = fields.into_iter().map(|window| (window, None)).collect();
            if let Some(times) = times {
                let lead = stops.len() - 1;
                let pickers = times
                    .into_iter()
                    .enumerate()
                    .filter(|(index, _)| *index == 0 || times[0].enabled)
                    .flat_map(|(index, value)| {
                        [
                            Some(Action::OpenPicker(index, PickerKind::Date)),
                            value
                                .enabled
                                .then_some(Action::OpenPicker(index, PickerKind::Time)),
                        ]
                        .into_iter()
                        .flatten()
                        .map(|action| (main, Some(action)))
                    });
                stops.splice(lead..lead, pickers);
            }
            let backwards = unsafe { GetKeyState(VK_SHIFT as i32) } < 0;
            let index = stops.iter().position(|&(window, action)| {
                msg.hwnd == window && (window != main || action == focus)
            });
            let next = match index {
                Some(index) if backwards => (index + stops.len() - 1) % stops.len(),
                Some(index) => (index + 1) % stops.len(),
                None if backwards => stops.len() - 1,
                None => 0,
            };
            let (window, action) = stops[next];
            with_app(|app| {
                app.picker_focus = action;
                app.repaint();
            });
            unsafe { SetFocus(window) };
            true
        }
        VK_RETURN | VK_SPACE if msg.hwnd == main && focus.is_some() => {
            perform(main, focus.unwrap());
            true
        }
        VK_DELETE if msg.hwnd == main => {
            if let Some(Action::OpenPicker(index, PickerKind::Date)) = focus {
                perform(main, Action::ClearDate(index));
                true
            } else {
                false
            }
        }
        VK_RETURN => {
            let action = match page {
                Page::Edit(_) => Action::SaveEntry,
                Page::Settings => Action::SaveDav,
                Page::Import if !fields.is_empty() => Action::ParseImport,
                _ => return false,
            };
            perform(main, action);
            true
        }
        VK_ESCAPE if matches!(page, Page::Course(_) | Page::Edit(_) | Page::Preview) => {
            perform(main, Action::Back);
            true
        }
        VK_F5 => {
            perform(main, Action::Sync);
            true
        }
        VK_UP | VK_PRIOR | VK_DOWN | VK_NEXT if msg.hwnd == main => {
            let key = msg.wParam as u16;
            let direction = if matches!(key, VK_UP | VK_PRIOR) {
                -1
            } else {
                1
            };
            scroll(direction, matches!(key, VK_PRIOR | VK_NEXT));
            true
        }
        _ => false,
    }
}

fn perform(owner: HWND, action: Action) {
    match action {
        Action::PickFile => {
            if let Some(path) = win::pick_file(owner) {
                with_app(|app| app.selected_file(path));
            }
            return;
        }
        Action::DeleteEntry(_)
            if !win::confirm(
                owner,
                "删除记录",
                "确定删除这条记录？已连接的设备也会同步这次删除。",
            ) =>
        {
            return;
        }
        _ => {}
    }
    let result = with_app(|app| app.act(action));
    if let Some(Err(error)) = result {
        with_app(|app| {
            app.status = error.clone();
            app.repaint();
        });
        if owner.is_null() || unsafe { IsWindowVisible(owner) } == 0 {
            win::alert(null_mut(), "课时钟", &error);
        }
    }
}

fn register(name: &str, proc: WNDPROC, icon: HICON, style: u32) -> Result<()> {
    let name = wide(name);
    let data = WNDCLASSW {
        lpfnWndProc: proc,
        hInstance: win::instance(),
        hIcon: icon,
        lpszClassName: name.as_ptr(),
        style,
        hCursor: unsafe { LoadCursorW(null_mut(), IDC_ARROW) },
        ..Default::default()
    };
    if unsafe { RegisterClassW(&data) } == 0 {
        Err("无法注册 Windows 窗口".into())
    } else {
        Ok(())
    }
}

#[expect(clippy::too_many_arguments)]
fn create(
    class: &str,
    title: &[u16],
    style: u32,
    ex: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    parent: HWND,
) -> HWND {
    unsafe {
        CreateWindowExW(
            ex,
            wide(class).as_ptr(),
            title.as_ptr(),
            style,
            x,
            y,
            w,
            h,
            parent,
            null_mut(),
            win::instance(),
            null(),
        )
    }
}

unsafe extern "system" fn controller(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match message {
        WM_TIMER if w == TIMER => {
            with_app(App::tick);
        }
        WM_TIMECHANGE => {
            with_app(App::tick);
        }
        WM_POWERBROADCAST => {
            if w == PBT_APMRESUMEAUTOMATIC as usize || w == PBT_APMRESUMESUSPEND as usize {
                with_app(App::tick);
            }
            return 1;
        }
        WM_SETTINGCHANGE | WM_THEMECHANGED => {
            with_app(|app| {
                app.dark = win::dark_mode();
                if !app.main.is_null() {
                    win::frame_theme(app.main, app.dark, true);
                    if !app.brush.is_null() {
                        unsafe {
                            DeleteObject(app.brush);
                        }
                    }
                    app.brush =
                        unsafe { CreateSolidBrush(ui::color(Palette::new(app.dark).surface)) };
                }
                app.repaint();
            });
        }
        SYNC_DONE => {
            with_app(App::finish_sync);
        }
        ACTIVATE => {
            with_app(App::open);
        }
        TRAY => match l as u32 {
            WM_LBUTTONUP => {
                with_app(App::open);
            }
            WM_RBUTTONUP | WM_CONTEXTMENU => tray_menu(hwnd),
            _ => {}
        },
        WM_CLOSE => {
            with_app(|app| {
                if app.job.is_some() {
                    app.status = "正在写入远端，请稍后退出".into();
                    app.open();
                } else {
                    app.tray(NIM_DELETE);
                    app.close_toast();
                    unsafe {
                        PostQuitMessage(0);
                    }
                }
            });
        }
        _ => {
            with_app(|app| {
                if message == app.taskbar_message {
                    app.tray(NIM_ADD);
                }
            });
            return unsafe { DefWindowProcW(hwnd, message, w, l) };
        }
    }
    0
}

fn tray_menu(window: HWND) {
    unsafe {
        let menu = CreatePopupMenu();
        let mut point = POINT::default();
        GetCursorPos(&mut point);
        AppendMenuW(
            menu,
            MF_STRING,
            OPEN_MENU as usize,
            wide("打开课时钟").as_ptr(),
        );
        let configured = with_app(|app| app.state.dav.is_some()).unwrap_or(false);
        AppendMenuW(
            menu,
            MF_STRING | if configured { 0 } else { MF_GRAYED },
            SYNC_MENU as usize,
            wide("立即同步").as_ptr(),
        );
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        AppendMenuW(menu, MF_STRING, EXIT_MENU as usize, wide("退出").as_ptr());
        SetForegroundWindow(window);
        let picked = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            window,
            null(),
        ) as u32;
        PostMessageW(window, WM_NULL, 0, 0);
        DestroyMenu(menu);
        match picked {
            OPEN_MENU => {
                with_app(App::open);
            }
            SYNC_MENU => {
                with_app(App::start_sync);
            }
            EXIT_MENU => {
                PostMessageW(window, WM_CLOSE, 0, 0);
            }
            _ => {}
        }
    }
}

unsafe extern "system" fn main_proc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match message {
        WM_PAINT => {
            if with_app(|app| {
                if app.main == hwnd {
                    app.paint();
                } else {
                    unsafe {
                        ValidateRect(hwnd, null());
                    }
                }
            })
            .is_none()
            {
                unsafe {
                    ValidateRect(hwnd, null());
                }
            }
        }
        WM_MOVE | WM_SIZE => {
            with_app(App::close_picker);
            unsafe { InvalidateRect(hwnd, null(), 0) };
        }
        WM_ERASEBKGND => return 1,
        WM_GETMINMAXINFO => unsafe {
            let info = &mut *(l as *mut MINMAXINFO);
            let (area, scale) = win::work_area();
            info.ptMinTrackSize.x = ui::scale(754, scale).min(area.right - area.left);
            info.ptMinTrackSize.y = ui::scale(621, scale).min(area.bottom - area.top);
        },
        WM_DPICHANGED => {
            let rect = unsafe { &*(l as *const RECT) };
            let values = with_app(|app| app.form_values());
            unsafe {
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            with_app(|app| {
                app.rebuild_form();
                if let Some(values) = values {
                    for (handle, value) in app.input.iter().zip(values) {
                        unsafe {
                            SetWindowTextW(*handle, wide(&value).as_ptr());
                        }
                    }
                }
                app.repaint();
            });
        }
        WM_CLOSE => {
            with_app(App::close_main);
        }
        WM_LBUTTONDOWN if begin_scroll(hwnd, l) => {}
        WM_LBUTTONUP => {
            if with_app(|app| app.scroll_drag.take().is_some()).unwrap_or(false) {
                unsafe { ReleaseCapture() };
            } else {
                handle_click(hwnd, l);
            }
        }
        WM_CAPTURECHANGED => {
            with_app(|app| app.scroll_drag = None);
        }
        WM_DROPFILES => {
            let drop = w as HDROP;
            let size = unsafe { DragQueryFileW(drop, 0, null_mut(), 0) };
            let mut path = vec![0_u16; size as usize + 1];
            let count = unsafe {
                let count = DragQueryFileW(drop, 0, path.as_mut_ptr(), size + 1);
                DragFinish(drop);
                count
            };
            if count > 0 {
                use std::os::windows::ffi::OsStringExt;
                let path = std::ffi::OsString::from_wide(&path[..count as usize]);
                with_app(|app| app.selected_file(PathBuf::from(path)));
            }
        }
        WM_MOUSEMOVE if drag_scroll(l) => {}
        WM_MOUSEMOVE | WM_MOUSELEAVE => handle_hover(hwnd, message, l),
        WM_MOUSEWHEEL => scroll(-(((w >> 16) as i16) as i32).signum(), false),
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC | WM_CTLCOLORLISTBOX => {
            if let Some(brush) = with_app(|app| {
                let p = Palette::new(app.dark);
                unsafe {
                    SetTextColor(w as HDC, ui::color(p.text));
                    SetBkColor(w as HDC, ui::color(p.field));
                }
                app.brush as LRESULT
            }) && brush != 0
            {
                return brush;
            }
            return unsafe { DefWindowProcW(hwnd, message, w, l) };
        }
        _ => return unsafe { DefWindowProcW(hwnd, message, w, l) },
    }
    0
}

unsafe extern "system" fn toast_proc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match message {
        WM_MOUSEACTIVATE => return MA_NOACTIVATE as LRESULT,
        WM_ERASEBKGND => return 1,
        WM_PAINT => {
            if with_app(|app| {
                if app.toast == hwnd
                    && let Some(data) = &app.toast_data
                {
                    app.toast_hits = ui::draw_toast(
                        hwnd,
                        app.toast_dpi,
                        Palette::new(app.dark),
                        &data.entry,
                        data.id.is_none(),
                        app.toast_hover,
                    );
                } else {
                    unsafe {
                        ValidateRect(hwnd, null());
                    }
                }
            })
            .is_none()
            {
                unsafe {
                    ValidateRect(hwnd, null());
                }
            }
        }
        WM_LBUTTONUP => handle_click(hwnd, l),
        WM_MOUSEMOVE | WM_MOUSELEAVE => handle_hover(hwnd, message, l),
        _ => return unsafe { DefWindowProcW(hwnd, message, w, l) },
    }
    0
}

unsafe extern "system" fn picker_proc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match message {
        WM_ERASEBKGND => return 1,
        WM_PAINT => {
            let painted = with_app(|app| {
                if app.picker_window == hwnd
                    && let Some(picker) = app.picker
                {
                    app.picker_hits = ui::draw_picker(
                        hwnd,
                        dpi(hwnd),
                        Palette::new(app.dark),
                        picker,
                        app.picker_hover,
                    );
                    true
                } else {
                    false
                }
            })
            .unwrap_or(false);
            if !painted {
                unsafe { ValidateRect(hwnd, null()) };
            }
        }
        WM_ACTIVATE if w as u16 == WA_INACTIVE as u16 => unsafe {
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
        },
        WM_CLOSE => {
            with_app(|app| {
                if app.picker_window == hwnd {
                    app.close_picker();
                    app.repaint();
                }
            });
        }
        WM_LBUTTONUP => handle_click(hwnd, l),
        WM_MOUSEMOVE | WM_MOUSELEAVE => handle_hover(hwnd, message, l),
        WM_KEYDOWN => {
            let Some(kind) = with_app(|app| app.picker.map(|picker| picker.kind)).flatten() else {
                return 0;
            };
            let action = match w as u16 {
                VK_ESCAPE => Action::ClosePicker,
                VK_RETURN | VK_SPACE => Action::ConfirmPicker,
                VK_LEFT => Action::PickerStep(-1),
                VK_RIGHT => Action::PickerStep(1),
                VK_UP if kind == PickerKind::Date => Action::PickerStep(-7),
                VK_DOWN if kind == PickerKind::Date => Action::PickerStep(7),
                VK_UP => Action::PickerStep(1),
                VK_DOWN => Action::PickerStep(-1),
                VK_PRIOR if kind == PickerKind::Date => Action::PickerMonth(-1),
                VK_NEXT if kind == PickerKind::Date => Action::PickerMonth(1),
                VK_PRIOR => Action::PickerStep(60),
                VK_NEXT => Action::PickerStep(-60),
                _ => return unsafe { DefWindowProcW(hwnd, message, w, l) },
            };
            perform(hwnd, action);
        }
        _ => return unsafe { DefWindowProcW(hwnd, message, w, l) },
    }
    0
}

fn hit(hits: &[Hit], pos: LPARAM) -> Option<Action> {
    let x = pos as i16 as i32;
    let y = (pos >> 16) as i16 as i32;
    hits.iter()
        .rev()
        .find(|item| item.contains(x, y))
        .map(|item| item.1)
}

fn handle_click(window: HWND, pos: LPARAM) {
    let action = with_app(|app| {
        if app.toast == window {
            hit(&app.toast_hits, pos).or(Some(Action::Dismiss))
        } else if app.picker_window == window {
            hit(&app.picker_hits, pos)
        } else if app.main == window {
            let action = hit(&app.main_hits, pos);
            if !matches!(action, Some(Action::OpenPicker(..) | Action::ClearDate(_)))
                && app.picker_focus.take().is_some()
            {
                app.repaint();
            }
            action
        } else {
            None
        }
    })
    .flatten();
    if let Some(action) = action {
        perform(window, action);
    }
}

fn handle_hover(window: HWND, message: u32, pos: LPARAM) {
    with_app(|app| {
        let toast = app.toast == window;
        let (hits, hovered) = if toast {
            (&app.toast_hits, &mut app.toast_hover)
        } else if app.picker_window == window {
            (&app.picker_hits, &mut app.picker_hover)
        } else {
            (&app.main_hits, &mut app.hover)
        };
        let action = if message == WM_MOUSELEAVE {
            None
        } else {
            hit(hits, pos)
        };
        if action != *hovered {
            *hovered = action;
            unsafe {
                InvalidateRect(window, null(), 0);
            }
        }
        if message == WM_MOUSEMOVE && (action.is_some() || toast) {
            let mut tracking = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: window,
                dwHoverTime: 0,
            };
            unsafe {
                SetCursor(LoadCursorW(null_mut(), IDC_HAND));
                TrackMouseEvent(&mut tracking);
            }
        }
    });
}

fn begin_scroll(window: HWND, pos: LPARAM) -> bool {
    let point = POINT {
        x: pos as i16 as i32,
        y: (pos >> 16) as i16 as i32,
    };
    let Some(handled) = with_app(|app| {
        let Some(bar) = &app.scrollbar else {
            return false;
        };
        if unsafe { PtInRect(&bar.track, point) } == 0 {
            return false;
        }
        if unsafe { PtInRect(&bar.thumb, point) } != 0 {
            app.scroll_drag = Some(point.y - bar.thumb.top);
        } else {
            let direction = if point.y < bar.thumb.top { -1 } else { 1 };
            app.scroll_to(bar.offset + direction * bar.page_height);
        }
        true
    }) else {
        return false;
    };
    if with_app(|app| app.scroll_drag.is_some()).unwrap_or(false) {
        unsafe { SetCapture(window) };
    }
    handled
}

fn drag_scroll(pos: LPARAM) -> bool {
    with_app(|app| {
        let (Some(grab), Some(bar)) = (app.scroll_drag, &app.scrollbar) else {
            return false;
        };
        let travel = bar.track.bottom - bar.track.top - (bar.thumb.bottom - bar.thumb.top);
        if travel > 0 {
            let y = (pos >> 16) as i16 as i32;
            let moved = (y - grab - bar.track.top).clamp(0, travel);
            app.scroll_to((moved * bar.limit + travel / 2) / travel);
        }
        true
    })
    .unwrap_or(false)
}

fn scroll(delta: i32, by_page: bool) {
    with_app(|app| {
        if let Some(bar) = &app.scrollbar {
            let position = if app.page == Page::Week {
                app.week_scroll.unwrap_or(bar.offset)
            } else {
                app.detail_scroll
            };
            let step = if by_page {
                (bar.page_height - 24).max(1)
            } else {
                60
            };
            app.scroll_to(position + delta * step);
        } else if matches!(app.page, Page::Today | Page::Tasks | Page::Preview) {
            app.scroll = app.scroll.saturating_add_signed(delta as isize);
            app.repaint();
        }
    });
}
