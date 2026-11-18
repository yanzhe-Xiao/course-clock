use std::{
    ffi::c_void,
    io,
    ptr::{null, null_mut},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use url::{Host, Url};
use uuid::Uuid;
use windows_sys::Win32::{
    Foundation::LocalFree,
    Networking::WinHttp::*,
    Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    },
};

use crate::{
    model::{DavSettings, Entries, Result, Semester, merge},
    win::wide,
};

const MAX_DOCUMENT: usize = 16 * 1024 * 1024;

pub fn endpoint(text: &str) -> Result<Url> {
    let mut url = Url::parse(text.trim()).map_err(|_| "请输入完整的 WebDAV 地址")?;
    let loopback = is_loopback(&url);
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        return Err("WebDAV 地址需以 https:// 开头".into());
    }
    if url.host().is_none()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("请填写不含账号、查询参数或片段的 WebDAV 目录地址".into());
    }
    if !url.path().ends_with('/') {
        url.path_segments_mut()
            .map_err(|_| "无效的 WebDAV 目录地址")?
            .push("");
    }
    Ok(url)
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        Some(Host::Domain(domain)) => domain == "localhost",
        None => false,
    }
}

pub fn protect(text: &str) -> Result<Vec<u8>> {
    crypt(text.as_bytes(), true)
}

fn crypt(bytes: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    let status = unsafe {
        if encrypt {
            CryptProtectData(
                &input,
                null(),
                null(),
                null(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                null_mut(),
                null(),
                null(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    check(status, "读取 Windows 加密凭据")?;
    let data =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        LocalFree(output.pbData.cast());
    }
    Ok(data)
}

#[derive(Serialize, Deserialize)]
struct Document {
    version: u32,
    entries: Entries,
    #[serde(default)]
    semester: Option<Semester>,
}

pub struct Synced {
    pub entries: Entries,
    pub conflicts: usize,
    pub semester: Option<Semester>,
}

pub fn sync(
    settings: &DavSettings,
    verified: bool,
    base: &Entries,
    local: &Entries,
    semester: &Semester,
) -> Result<Synced> {
    let url = endpoint(&settings.url)?;
    let client = Client::new(&url, settings)?;
    if !verified {
        verify(&client)?;
    }
    for _ in 0..4 {
        let remote = client.request("GET", "course-clock.json", "", None)?;
        let (entries, remote_semester, condition) = match remote.status {
            200 => {
                let doc: Document = serde_json::from_slice(&remote.body)
                    .map_err(|_| "远端 course-clock.json 格式无效，已停止同步")?;
                if doc.version != 1 || doc.entries.values().any(|entry| !entry.validates()) {
                    return Err("远端数据版本或内容无效，已停止同步".into());
                }
                (
                    doc.entries,
                    doc.semester,
                    format!("If-Match: {}\r\n", remote.tag()?),
                )
            }
            404 if !base.is_empty() => {
                return Err(
                    "远端文件丢失，本地数据已保留。请核对 WebDAV 目录或恢复远端文件".into(),
                );
            }
            404 => (Entries::new(), None, "If-None-Match: *\r\n".to_string()),
            status => return Err(http_error("读取", status)),
        };
        let (merged, conflicts) = merge(base, local, &entries);
        if merged == entries && remote.status == 200 {
            return Ok(Synced {
                entries: merged,
                conflicts,
                semester: remote_semester,
            });
        }
        let doc = Document {
            version: 1,
            entries: merged,
            semester: Some(remote_semester.unwrap_or_else(|| semester.clone())),
        };
        let bytes = serde_json::to_vec(&doc).map_err(|error| error.to_string())?;
        if bytes.len() > MAX_DOCUMENT {
            return Err("同步文件超出 16 MB".into());
        }
        let response = client.request("PUT", "course-clock.json", &condition, Some(&bytes))?;
        match response.status {
            200 | 201 | 204 => {
                return Ok(Synced {
                    entries: doc.entries,
                    conflicts,
                    semester: doc.semester,
                });
            }
            412 => continue,
            status => return Err(http_error("写入", status)),
        }
    }
    Err("远端一直有新修改，本轮同步已延后".into())
}

fn verify(client: &Client) -> Result<()> {
    let probe = format!("course-clock-check-{}.json", Uuid::new_v4());
    let create = "If-None-Match: *\r\n";
    let response = client.request("PUT", &probe, create, Some(b"{}"))?;
    if !matches!(response.status, 200 | 201 | 204) {
        return Err(http_error("检查目录写权限", response.status));
    }
    let result = (|| {
        let current = client.request("GET", &probe, "", None)?;
        if current.status != 200 {
            return Err(http_error("验证同步条件", current.status));
        }
        let tag = current.tag()?;
        let invalid = "If-Match: \"course-clock-invalid-test-tag\"\r\n";
        if client
            .request("PUT", &probe, invalid, Some(b"{\"probe\":1}"))?
            .status
            != 412
            || client
                .request("PUT", &probe, create, Some(b"{\"probe\":2}"))?
                .status
                != 412
        {
            return Err("WebDAV 服务忽略条件写入，无法安全进行双机同步".into());
        }
        let valid = format!("If-Match: {tag}\r\n");
        let write = client.request("PUT", &probe, &valid, Some(b"{\"probe\":3}"))?;
        if !matches!(write.status, 200 | 201 | 204) {
            return Err(http_error("检查有效 ETag 写入", write.status));
        }
        Ok(())
    })();
    let cleanup = client.request("DELETE", &probe, "", None);
    result?;
    let status = cleanup?.status;
    if !matches!(status, 200 | 202 | 204 | 404) {
        return Err(http_error("删除同步测试文件", status));
    }
    Ok(())
}

struct Client {
    connection: Handle,
    _session: Handle,
    origin: Url,
    auth: String,
}

impl Client {
    fn new(url: &Url, settings: &DavSettings) -> Result<Self> {
        let agent = wide("CourseClock/0.1");
        let access = if is_loopback(url) {
            WINHTTP_ACCESS_TYPE_NO_PROXY
        } else {
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY
        };
        let session = Handle::new(
            unsafe { WinHttpOpen(agent.as_ptr(), access, null(), null(), 0) },
            "初始化 WinHTTP",
        )?;
        check(
            unsafe { WinHttpSetTimeouts(session.0, 5000, 5000, 5000, 6000) },
            "设置连接超时",
        )?;
        let hostname = url.host_str().ok_or("WebDAV 地址缺少主机名")?;
        let host = wide(hostname.trim_matches(['[', ']']));
        let port = url.port_or_known_default().ok_or("WebDAV 地址缺少端口")?;
        let connection = Handle::new(
            unsafe { WinHttpConnect(session.0, host.as_ptr(), port, 0) },
            "连接 WebDAV",
        )?;
        let password = crypt(&settings.password, false)?;
        let mut credentials = settings.username.as_bytes().to_vec();
        credentials.push(b':');
        credentials.extend(&password);
        let auth = if settings.username.is_empty() && password.is_empty() {
            String::new()
        } else {
            format!("Authorization: Basic {}\r\n", STANDARD.encode(credentials))
        };
        Ok(Self {
            connection,
            _session: session,
            origin: url.clone(),
            auth,
        })
    }

    fn request(
        &self,
        method: &str,
        name: &str,
        conditions: &str,
        body: Option<&[u8]>,
    ) -> Result<Response> {
        let url = self.origin.join(name).map_err(|error| error.to_string())?;
        let path = wide(url.path());
        let method_wide = wide(method);
        let flags = if url.scheme() == "https" {
            WINHTTP_FLAG_SECURE
        } else {
            0
        };
        let request = Handle::new(
            unsafe {
                WinHttpOpenRequest(
                    self.connection.0,
                    method_wide.as_ptr(),
                    path.as_ptr(),
                    null(),
                    null(),
                    null(),
                    flags,
                )
            },
            "建立 WebDAV 请求",
        )?;
        let disable = WINHTTP_DISABLE_REDIRECTS | WINHTTP_DISABLE_COOKIES;
        check(
            unsafe {
                WinHttpSetOption(
                    request.0,
                    WINHTTP_OPTION_DISABLE_FEATURE,
                    (&disable as *const u32).cast(),
                    4,
                )
            },
            "设置 WebDAV 请求选项",
        )?;
        let content_type = if body.is_some() {
            "Content-Type: application/json\r\n"
        } else {
            ""
        };
        let headers = wide(&format!(
            "{}Cache-Control: no-cache\r\n{content_type}{conditions}",
            self.auth
        ));
        let bytes = body.unwrap_or_default();
        check(
            unsafe {
                WinHttpSendRequest(
                    request.0,
                    headers.as_ptr(),
                    u32::MAX,
                    body.map_or(null(), |bytes| bytes.as_ptr().cast()),
                    bytes.len() as u32,
                    bytes.len() as u32,
                    0,
                )
            },
            "发送 WebDAV 请求",
        )?;
        check(
            unsafe { WinHttpReceiveResponse(request.0, null_mut()) },
            "读取 WebDAV 响应",
        )?;
        let mut status = 0_u32;
        let mut size = 4_u32;
        check(
            unsafe {
                WinHttpQueryHeaders(
                    request.0,
                    WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                    null(),
                    (&mut status as *mut u32).cast(),
                    &mut size,
                    null_mut(),
                )
            },
            "读取 HTTP 状态",
        )?;
        let mut response = Response {
            status,
            etag: None,
            body: Vec::new(),
        };
        if method == "GET" && status == 200 {
            let mut tag = [0_u16; 512];
            let mut size = size_of_val(&tag) as u32;
            if unsafe {
                WinHttpQueryHeaders(
                    request.0,
                    WINHTTP_QUERY_ETAG,
                    null(),
                    tag.as_mut_ptr().cast(),
                    &mut size,
                    null_mut(),
                )
            } != 0
            {
                let end = tag.iter().position(|ch| *ch == 0).unwrap_or(tag.len());
                response.etag = String::from_utf16(&tag[..end]).ok();
            }
            let mut buffer = [0_u8; 8 * 1024];
            loop {
                let mut read = 0;
                check(
                    unsafe {
                        WinHttpReadData(
                            request.0,
                            buffer.as_mut_ptr().cast(),
                            buffer.len() as u32,
                            &mut read,
                        )
                    },
                    "下载同步数据",
                )?;
                if read == 0 {
                    break;
                }
                if response.body.len() + read as usize > MAX_DOCUMENT {
                    return Err("远端同步文件超出 16 MB".into());
                }
                response.body.extend_from_slice(&buffer[..read as usize]);
            }
        }
        Ok(response)
    }
}

struct Response {
    status: u32,
    etag: Option<String>,
    body: Vec<u8>,
}

impl Response {
    fn tag(&self) -> Result<&str> {
        self.etag
            .as_deref()
            .map(str::trim)
            .filter(|tag| {
                tag.len() >= 2
                    && tag.starts_with('"')
                    && tag.ends_with('"')
                    && tag.as_bytes()[1..tag.len() - 1]
                        .iter()
                        .all(|byte| matches!(byte, 0x21 | 0x23..=0x7e))
            })
            .ok_or_else(|| "WebDAV 响应没有可用于条件写入的强 ETag".into())
    }
}

struct Handle(*mut c_void);
impl Handle {
    fn new(value: *mut c_void, action: &str) -> Result<Self> {
        if value.is_null() {
            Err(format!("{action}失败：{}", io::Error::last_os_error()))
        } else {
            Ok(Self(value))
        }
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            WinHttpCloseHandle(self.0);
        }
    }
}

fn check(ok: i32, action: &str) -> Result<()> {
    if ok == 0 {
        Err(format!("{action}失败：{}", io::Error::last_os_error()))
    } else {
        Ok(())
    }
}

fn http_error(action: &str, status: u32) -> String {
    let detail = match status {
        301 | 302 | 307 | 308 => "，请填写跳转后的 WebDAV 目录地址",
        401 | 403 => "，请核对应用密码和目录权限",
        404 | 409 => "，请检查目录是否已创建",
        _ => "",
    };
    format!("{action}失败 · HTTP {status}{detail}")
}
