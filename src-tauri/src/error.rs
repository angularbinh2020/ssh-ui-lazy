use serde::{ser::SerializeStruct, Serialize, Serializer};

/// Lỗi phân loại — message tiếng Việt, serialize thành object cho frontend.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Sai tên đăng nhập hoặc mật khẩu SSH")]
    WrongPassword,
    #[error("Không thể kết nối tới {host}:{port}: {detail}")]
    HostUnreachable { host: String, port: u16, detail: String },
    #[error("Quá thời gian chờ: {stage}")]
    Timeout { stage: String },
    #[error("Fingerprint máy chủ đã thay đổi! Có thể có kẻ nghe lén (MITM). Cũ: {expected}, mới: {actual}")]
    FingerprintChanged { expected: String, actual: String },
    #[error("Máy chủ chưa được tin cậy. Fingerprint: {fingerprint}")]
    HostUnknown { fingerprint: String },
    #[error("Không có kubectl trên máy chủ")]
    NoKubectl,
    #[error("Context '{context}' không tồn tại trên máy chủ")]
    ContextNotFound { context: String },
    #[error("Pod '{pod}' không tồn tại trong namespace '{ns}'")]
    PodNotFound { pod: String, ns: String },
    #[error("Mất kết nối SSH")]
    ConnectionLost,
    #[error("Không thể chuyển sang gitlab-runner: {detail}")]
    SuFailed { detail: String },
    #[error("Lỗi sudo: {detail}")]
    SudoFailed { detail: String },
    #[error("Lỗi SSH: {detail}")]
    Ssh { detail: String },
    #[error("Lỗi mã hóa dữ liệu: {detail}")]
    Crypto { detail: String },
    #[error("Passphrase không đúng hoặc dữ liệu đã hỏng")]
    UnlockFailed,
    #[error("Lỗi đọc/ghi Windows Credential Manager: {detail}")]
    Keyring { detail: String },
    #[error("Tên profile '{name}' đã tồn tại")]
    DuplicateName { name: String },
    #[error("Không tìm thấy profile")]
    ProfileNotFound,
    #[error("Chưa kết nối. Hãy bấm Kết nối trước.")]
    NotConnected,
    #[error("Tab không tồn tại hoặc đã đóng")]
    TabNotFound,
    #[error("Lỗi hệ thống: {detail}")]
    Io { detail: String },
    #[error("{detail}")]
    Other { detail: String },
}

impl AppError {
    pub fn code(&self) -> &'static str {
        match self {
            AppError::WrongPassword => "WRONG_PASSWORD",
            AppError::HostUnreachable { .. } => "HOST_UNREACHABLE",
            AppError::Timeout { .. } => "TIMEOUT",
            AppError::FingerprintChanged { .. } => "FINGERPRINT_CHANGED",
            AppError::HostUnknown { .. } => "HOST_UNKNOWN",
            AppError::NoKubectl => "NO_KUBECTL",
            AppError::ContextNotFound { .. } => "CONTEXT_NOT_FOUND",
            AppError::PodNotFound { .. } => "POD_NOT_FOUND",
            AppError::ConnectionLost => "CONNECTION_LOST",
            AppError::SuFailed { .. } => "SU_FAILED",
            AppError::SudoFailed { .. } => "SUDO_FAILED",
            AppError::Ssh { .. } => "SSH",
            AppError::Crypto { .. } => "CRYPTO",
            AppError::UnlockFailed => "UNLOCK_FAILED",
            AppError::Keyring { .. } => "KEYRING",
            AppError::DuplicateName { .. } => "DUPLICATE_NAME",
            AppError::ProfileNotFound => "PROFILE_NOT_FOUND",
            AppError::NotConnected => "NOT_CONNECTED",
            AppError::TabNotFound => "TAB_NOT_FOUND",
            AppError::Io { .. } => "IO",
            AppError::Other { .. } => "OTHER",
        }
    }

    pub fn fingerprint(&self) -> Option<String> {
        match self {
            AppError::HostUnknown { fingerprint } | AppError::FingerprintChanged { actual: fingerprint, .. } => {
                Some(fingerprint.clone())
            }
            _ => None,
        }
    }

    pub fn expected_fingerprint(&self) -> Option<String> {
        match self {
            AppError::FingerprintChanged { expected, .. } => Some(expected.clone()),
            _ => None,
        }
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut st = serializer.serialize_struct("AppError", 5)?;
        st.serialize_field("code", self.code())?;
        st.serialize_field("message", &self.to_string())?;
        st.serialize_field("fingerprint", &self.fingerprint())?;
        st.serialize_field("expectedFingerprint", &self.expected_fingerprint())?;
        st.serialize_field(
            "extra",
            &match self {
                AppError::HostUnreachable { host, port, .. } => Some(format!("{host}:{port}")),
                AppError::ContextNotFound { context } => Some(context.clone()),
                AppError::PodNotFound { pod, ns } => Some(format!("{ns}/{pod}")),
                _ => None,
            },
        )?;
        st.end()
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Io { detail: e.to_string() }
    }
}

pub type AppResult<T> = Result<T, AppError>;
