use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub context: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInput {
    pub id: Option<String>,
    pub name: String,
    pub host: String,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub password: String,
    pub context: String,
}

/// Parse chuỗi host input: `host`, `user@host`, `user@host:port`, `host:port`.
pub fn parse_host_input(input: &str) -> (Option<String>, String, Option<u16>) {
    let input = input.trim();
    let (user, rest) = match input.split_once('@') {
        Some((u, r)) => {
            let u = u.trim();
            (if u.is_empty() { None } else { Some(u.to_string()) }, r)
        }
        None => (None, input),
    };
    // IPv6 dạng [::1]:22 — bỏ qua port khi có dấu ]
    let (host, port) = if rest.starts_with('[') {
        match rest.split_once(']') {
            Some((h, tail)) => {
                let port = tail
                    .strip_prefix(':')
                    .and_then(|p| p.parse::<u16>().ok());
                (h.trim_start_matches('[').to_string(), port)
            }
            None => (rest.to_string(), None),
        }
    } else if rest.matches(':').count() == 1 {
        match rest.split_once(':') {
            Some((h, p)) => (h.to_string(), p.parse::<u16>().ok()),
            None => (rest.to_string(), None),
        }
    } else {
        (rest.to_string(), None)
    };
    (user, host.trim().to_string(), port)
}

fn now_iso() -> String {
    // Không phụ thuộc crate thời gian: dùng SystemTime epoch → ISO8601 thủ công.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    epoch_secs_to_iso(secs)
}

fn epoch_secs_to_iso(secs: u64) -> String {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // civil_from_days (Howard Hinnant)
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

fn build_profile(input: ProfileInput) -> AppResult<Profile> {
    let (input_user, host, input_port) = parse_host_input(&input.host);
    if host.is_empty() {
        return Err(AppError::Other { detail: "SSH host không được để trống".into() });
    }
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::Other { detail: "Tên profile không được để trống".into() });
    }
    let username = if let Some(u) = &input.username {
        if u.trim().is_empty() {
            return Err(AppError::Other { detail: "Username không được để trống".into() });
        }
        u.trim().to_string()
    } else if let Some(u) = &input_user {
        u.clone()
    } else {
        return Err(AppError::Other {
            detail: "Thiếu username (ghi trong trường riêng hoặc dạng user@host)".into(),
        });
    };
    Ok(Profile {
        id: input.id.unwrap_or_else(|| Uuid::new_v4().to_string()),
        name,
        host,
        port: input.port.or(input_port).unwrap_or(22),
        username,
        password: input.password,
        context: input.context.trim().to_string(),
        created_at: now_iso(),
        updated_at: now_iso(),
    })
}

/// Thêm hoặc cập nhật (theo id). Tên profile phải duy nhất.
pub fn upsert(profiles: &mut Vec<Profile>, input: ProfileInput) -> AppResult<()> {
    let profile = build_profile(input)?;
    let id_exists = profiles.iter().any(|p| p.id == profile.id);
    if id_exists {
        if profiles
            .iter()
            .any(|p| p.id != profile.id && p.name.eq_ignore_ascii_case(&profile.name))
        {
            return Err(AppError::DuplicateName { name: profile.name });
        }
        if let Some(existing) = profiles.iter_mut().find(|p| p.id == profile.id) {
            existing.name = profile.name;
            existing.host = profile.host;
            existing.port = profile.port;
            existing.username = profile.username;
            existing.password = profile.password;
            existing.context = profile.context;
            existing.updated_at = now_iso();
        }
        return Ok(());
    }
    if profiles
        .iter()
        .any(|p| p.name.eq_ignore_ascii_case(&profile.name))
    {
        return Err(AppError::DuplicateName { name: profile.name });
    }
    profiles.push(profile);
    Ok(())
}

pub fn delete(profiles: &mut Vec<Profile>, id: &str) -> AppResult<()> {
    let len_before = profiles.len();
    profiles.retain(|p| p.id != id);
    if profiles.len() == len_before {
        return Err(AppError::ProfileNotFound);
    }
    Ok(())
}

/// Nhân bản: id mới, tên "<tên> (copy)" (đảm bảo duy nhất).
pub fn duplicate(profiles: &mut Vec<Profile>, id: &str) -> AppResult<String> {
    let source = profiles
        .iter()
        .find(|p| p.id == id)
        .ok_or(AppError::ProfileNotFound)?
        .clone();
    let mut n = 1usize;
    let new_name = loop {
        let candidate = if n == 1 {
            format!("{} (copy)", source.name)
        } else {
            format!("{} (copy {})", source.name, n)
        };
        if !profiles.iter().any(|p| p.name.eq_ignore_ascii_case(&candidate)) {
            break candidate;
        }
        n += 1;
    };
    let mut copy = source;
    copy.id = Uuid::new_v4().to_string();
    copy.name = new_name;
    copy.created_at = now_iso();
    copy.updated_at = copy.created_at.clone();
    let new_id = copy.id.clone();
    profiles.push(copy);
    Ok(new_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str, host: &str) -> ProfileInput {
        ProfileInput {
            id: None,
            name: name.into(),
            host: host.into(),
            port: None,
            username: None,
            password: "pw".into(),
            context: "ctx".into(),
        }
    }

    #[test]
    fn host_parsing() {
        assert_eq!(
            parse_host_input("10.110.30.126"),
            (None, "10.110.30.126".into(), None)
        );
        assert_eq!(
            parse_host_input("abc@10.110.30.126"),
            (Some("abc".into()), "10.110.30.126".into(), None)
        );
        assert_eq!(
            parse_host_input("abc@10.110.30.126:2222"),
            (Some("abc".into()), "10.110.30.126".into(), Some(2222))
        );
        assert_eq!(
            parse_host_input("host.example.com:2200"),
            (None, "host.example.com".into(), Some(2200))
        );
        // IPv6 không có port
        assert_eq!(
            parse_host_input("[::1]"),
            (None, "::1".into(), None)
        );
    }

    #[test]
    fn upsert_and_unique_name() {
        let mut profiles = vec![];
        upsert(&mut profiles, input("staging-126", "abc@1.2.3.4")).unwrap();
        assert_eq!(profiles[0].username, "abc");
        assert_eq!(profiles[0].port, 22);

        // trùng tên → lỗi
        assert!(upsert(&mut profiles, input("STAGING-126", "1.2.3.5")).is_err());

        // sửa giữ nguyên tên theo id → ok
        let id = profiles[0].id.clone();
        let mut i = input("staging-126", "abc@1.2.3.4:2200");
        i.id = Some(id.clone());
        upsert(&mut profiles, i).unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].port, 2200);
    }

    #[test]
    fn delete_and_duplicate() {
        let mut profiles = vec![];
        upsert(&mut profiles, input("a", "u@h")).unwrap();
        upsert(&mut profiles, input("b", "u@h")).unwrap();
        let id = profiles[0].id.clone();

        let new_id = duplicate(&mut profiles, &id).unwrap();
        assert_eq!(profiles.len(), 3);
        assert_eq!(profiles[2].name, "a (copy)");
        assert_ne!(profiles[2].id, id);
        assert_eq!(profiles[2].password, "pw");

        delete(&mut profiles, &new_id).unwrap();
        assert!(delete(&mut profiles, &new_id).is_err());
        assert_eq!(profiles.len(), 2);
    }

    #[test]
    fn iso_timestamp() {
        // 2026-11-20T00:00:00Z = 1795132800
        assert_eq!(epoch_secs_to_iso(1_795_132_800), "2026-11-20T00:00:00Z");
        assert_eq!(epoch_secs_to_iso(0), "1970-01-01T00:00:00Z");
    }
}
