use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use rand::rngs::OsRng;
use rand::RngCore;
use zeroize::Zeroizing;

use crate::error::{AppError, AppResult};

/// magic | version u8 | kdf u8 | [salt 16B nếu kdf=1] | nonce 12B | ciphertext
pub const MAGIC: &[u8] = b"SSHLZY";
pub const FORMAT_VERSION: u8 = 1;
pub const KDF_KEYRING: u8 = 0;
pub const KDF_ARGON2ID: u8 = 1;

const KEYRING_SERVICE: &str = "ssh-ui-lazy";
const KEYRING_ACCOUNT: &str = "master-key";

/// Argon2id: m=64MiB, t=3, p=4 (mục 4 PLAN)
const ARGON2_M_KIB: u32 = 64 * 1024;
const ARGON2_T: u32 = 3;
const ARGON2_P: u32 = 4;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;

pub struct FileHeader {
    pub kdf: u8,
    pub salt: Option<[u8; SALT_LEN]>,
    /// số byte của phần header (magic + version + kdf + salt) — dùng làm AAD
    pub header_len: usize,
}

pub fn parse_header(data: &[u8]) -> AppResult<FileHeader> {
    if data.len() < MAGIC.len() + 3 {
        return Err(AppError::Crypto {
            detail: "File dữ liệu quá ngắn hoặc hỏng".into(),
        });
    }
    if &data[..MAGIC.len()] != MAGIC {
        return Err(AppError::Crypto {
            detail: "File dữ liệu không đúng định dạng (magic mismatch)".into(),
        });
    }
    let version = data[MAGIC.len()];
    if version != FORMAT_VERSION {
        return Err(AppError::Crypto {
            detail: format!("Phiên bản file dữ liệu {version} không được hỗ trợ"),
        });
    }
    let kdf = data[MAGIC.len() + 1];
    match kdf {
        KDF_KEYRING => Ok(FileHeader { kdf, salt: None, header_len: MAGIC.len() + 2 }),
        KDF_ARGON2ID => {
            let need = MAGIC.len() + 2 + SALT_LEN;
            if data.len() < need {
                return Err(AppError::Crypto {
                    detail: "File dữ liệu hỏng (thiếu salt)".into(),
                });
            }
            let mut salt = [0u8; SALT_LEN];
            salt.copy_from_slice(&data[MAGIC.len() + 2..need]);
            Ok(FileHeader { kdf, salt: Some(salt), header_len: need })
        }
        other => Err(AppError::Crypto {
            detail: format!("KDF không xác định: {other}"),
        }),
    }
}

pub fn encrypt_to_bytes(
    key: &[u8; KEY_LEN],
    kdf: u8,
    salt: Option<&[u8; SALT_LEN]>,
    plaintext: &[u8],
) -> AppResult<Vec<u8>> {
    let mut header = Vec::with_capacity(MAGIC.len() + 2 + SALT_LEN);
    header.extend_from_slice(MAGIC);
    header.push(FORMAT_VERSION);
    header.push(kdf);
    if kdf == KDF_ARGON2ID {
        let salt = salt.ok_or_else(|| AppError::Crypto {
            detail: "KDF argon2id cần salt".into(),
        })?;
        header.extend_from_slice(salt);
    }

    let mut nonce_bytes = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from(nonce_bytes);

    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| AppError::Crypto { detail: e.to_string() })?;
    let ciphertext = cipher
        .encrypt(&nonce, aes_gcm::aead::Payload { msg: plaintext, aad: &header })
        .map_err(|_| AppError::Crypto {
            detail: "Mã hóa thất bại".into(),
        })?;

    let mut out = header;
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// AAD = header (magic + version + kdf + salt). Sai key/AAD → lỗi.
pub fn decrypt_from_bytes(data: &[u8], key: &[u8; KEY_LEN]) -> AppResult<Vec<u8>> {
    let header = parse_header(data)?;
    if data.len() < header.header_len + NONCE_LEN + 16 {
        return Err(AppError::Crypto {
            detail: "File dữ liệu hỏng (thiếu nonce/ciphertext)".into(),
        });
    }
    let nonce_bytes = &data[header.header_len..header.header_len + NONCE_LEN];
    let ciphertext = &data[header.header_len + NONCE_LEN..];
    let mut nonce_arr = [0u8; NONCE_LEN];
    nonce_arr.copy_from_slice(nonce_bytes);
    let nonce = Nonce::from(nonce_arr);

    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| AppError::Crypto { detail: e.to_string() })?;
    cipher
        .decrypt(&nonce, aes_gcm::aead::Payload { msg: ciphertext, aad: &data[..header.header_len] })
        .map_err(|_| AppError::UnlockFailed)
}

pub fn derive_argon2id(passphrase: &str, salt: &[u8; SALT_LEN]) -> AppResult<Zeroizing<[u8; KEY_LEN]>> {
    let params = Params::new(ARGON2_M_KIB, ARGON2_T, ARGON2_P, Some(KEY_LEN))
        .map_err(|e| AppError::Crypto { detail: e.to_string() })?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = Zeroizing::new([0u8; KEY_LEN]);
    argon
        .hash_password_into(passphrase.as_bytes(), salt, out.as_mut())
        .map_err(|e| AppError::Crypto { detail: e.to_string() })?;
    Ok(out)
}

fn entry() -> AppResult<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
        .map_err(|e| AppError::Keyring { detail: e.to_string() })
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn from_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok())
        .collect()
}

/// Đọc key 32B từ OS keyring; chưa có thì tạo mới và lưu.
pub fn get_or_create_keyring_key() -> AppResult<Zeroizing<[u8; KEY_LEN]>> {
    let entry = entry()?;
    if let Ok(stored) = entry.get_password() {
        if let Some(bytes) = from_hex(&stored) {
            if bytes.len() == KEY_LEN {
                let mut key = Zeroizing::new([0u8; KEY_LEN]);
                key.copy_from_slice(&bytes);
                return Ok(key);
            }
        }
    }
    let key = random_key();
    entry
        .set_password(&to_hex(key.as_ref()))
        .map_err(|e| AppError::Keyring { detail: e.to_string() })?;
    Ok(key)
}

pub fn set_keyring_key(key: &[u8; KEY_LEN]) -> AppResult<()> {
    let e = entry()?;
    e.set_password(&to_hex(key))
        .map_err(|err| AppError::Keyring { detail: err.to_string() })
}

pub fn delete_keyring_key() -> AppResult<()> {
    let entry = entry()?;
    // Không coi việc thiếu entry là lỗi.
    let _ = entry.delete_credential();
    Ok(())
}

pub fn random_key() -> Zeroizing<[u8; KEY_LEN]> {
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    OsRng.fill_bytes(key.as_mut());
    key
}

pub fn random_salt() -> [u8; SALT_LEN] {
    let mut salt = [0u8; SALT_LEN];
    OsRng.fill_bytes(&mut salt);
    salt
}

pub fn random_suffix(len: usize) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut buf = vec![0u8; len];
    OsRng.fill_bytes(&mut buf);
    buf.iter().map(|b| HEX[(b & 0x0f) as usize] as char).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn roundtrip_keyring_mode() {
        let key = random_key();
        let plaintext = json!([{ "name": "staging-126", "password": "p@ss'w!rd" }])
            .to_string()
            .into_bytes();
        let file = encrypt_to_bytes(&key, KDF_KEYRING, None, &plaintext).unwrap();
        let decrypted = decrypt_from_bytes(&file, &key).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn roundtrip_argon2_mode() {
        let salt = random_salt();
        let key = derive_argon2id("chuỗi pass phr@se !#", &salt).unwrap();
        let plaintext = b"[]".to_vec();
        let file = encrypt_to_bytes(&key, KDF_ARGON2ID, Some(&salt), &plaintext).unwrap();
        assert_eq!(decrypt_from_bytes(&file, &key).unwrap(), plaintext);

        // sai passphrase → fail
        let wrong = derive_argon2id("sai", &salt).unwrap();
        assert!(decrypt_from_bytes(&file, &wrong).is_err());
    }

    #[test]
    fn wrong_key_fails() {
        let file = encrypt_to_bytes(&random_key(), KDF_KEYRING, None, b"data").unwrap();
        assert!(decrypt_from_bytes(&file, &random_key()).is_err());
    }

    #[test]
    fn aad_tamper_fails() {
        let key = random_key();
        let mut file = encrypt_to_bytes(&key, KDF_ARGON2ID, Some(&random_salt()), b"data").unwrap();
        // sửa 1 byte trong header (AAD) → decrypt phải fail
        file[6] ^= 0x01;
        assert!(decrypt_from_bytes(&file, &key).is_err());
    }

    #[test]
    fn ciphertext_tamper_fails() {
        let key = random_key();
        let mut file = encrypt_to_bytes(&key, KDF_KEYRING, None, b"data").unwrap();
        let last = file.len() - 1;
        file[last] ^= 0xFF;
        assert!(decrypt_from_bytes(&file, &key).is_err());
    }

    #[test]
    fn header_parsing() {
        let salt = random_salt();
        let key = random_key();
        let file = encrypt_to_bytes(&key, KDF_ARGON2ID, Some(&salt), b"x").unwrap();
        let h = parse_header(&file).unwrap();
        assert_eq!(h.kdf, KDF_ARGON2ID);
        assert_eq!(h.salt.unwrap(), salt);

        let file2 = encrypt_to_bytes(&key, KDF_KEYRING, None, b"x").unwrap();
        let h2 = parse_header(&file2).unwrap();
        assert_eq!(h2.kdf, KDF_KEYRING);
        assert!(h2.salt.is_none());

        assert!(parse_header(b"NOPE01").is_err());
        assert!(parse_header(b"SSHLZY\x09\x00").is_err()); // version lạ
        // version mới hơn → lỗi migrate cần làm
        let mut file3 = encrypt_to_bytes(&key, KDF_KEYRING, None, b"x").unwrap();
        file3[6] = FORMAT_VERSION + 1;
        assert!(decrypt_from_bytes(&file3, &key).is_err());
    }

    #[test]
    fn hex_roundtrip() {
        let bytes = random_key();
        assert_eq!(from_hex(&to_hex(bytes.as_ref())).unwrap(), bytes.to_vec());
    }
}
