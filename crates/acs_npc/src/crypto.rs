use std::{fs, path::Path};

/// |E| 前缀文件的解密密码和盐值（与 C# GFileUtil.cs 一致）
const DECRYPT_PASSWORD: &[u8] = b"no spoilers please";
const DECRYPT_SALT: &[u8] = b"XhSG9hLyZ7T~Ge3@";
const DECRYPT_IV: &[u8] = b"~8YSi0Xv2@|{aDfb";

/// 通过 PBKDF2-HMAC-SHA1 从密码派生 AES-128 密钥
/// 与 C# Rfc2898DeriveBytes(password, salt, 1000) 一致
fn derive_key(password: &[u8], salt: &[u8], iterations: u32) -> [u8; 16] {
    let mut key = [0u8; 16];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password, salt, iterations, &mut key);
    key
}

/// AES-128-CBC 解密（无填充，手动去零）
fn aes_cbc_decrypt(ciphertext: &[u8], key: &[u8; 16], iv: &[u8; 16]) -> Vec<u8> {
    use aes::cipher::{generic_array::GenericArray, BlockDecrypt, KeyInit};

    let cipher = aes::Aes128::new_from_slice(key).expect("Invalid AES key");
    let mut buf = ciphertext.to_vec();

    // PKCS7 或 Zero 填充：确保长度是 16 的倍数
    if buf.len() % 16 != 0 {
        let rem = 16 - (buf.len() % 16);
        buf.extend(vec![0; rem]);
    }

    // CBC 模式解密
    let mut prev_block = *iv;
    for chunk in buf.chunks_exact_mut(16) {
        let mut block = GenericArray::clone_from_slice(chunk);
        cipher.decrypt_block(&mut block);
        // CBC: plaintext = decrypt(ciphertext) XOR previous_ciphertext
        for i in 0..16 {
            let orig_ct = chunk[i];
            chunk[i] = block[i] ^ prev_block[i];
            prev_block[i] = orig_ct;
        }
    }

    buf
}

pub fn read_and_decrypt(p: &Path) -> std::io::Result<String> {
    let bytes = fs::read(p)?;
    let content = String::from_utf8_lossy(&bytes).into_owned();
    let trimmed = content.trim_start_matches(|c: char| c == '\u{FEFF}' || c.is_whitespace());

    if trimmed.starts_with("|E|") {
        let b64_str: String = trimmed[3..]
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        use base64::{engine::general_purpose::STANDARD, Engine as _};

        if let Ok(cipher_bytes) = STANDARD.decode(&b64_str) {
            // PBKDF2 派生密钥（1000 次迭代，与 .NET Rfc2898DeriveBytes 默认一致）
            let key = derive_key(DECRYPT_PASSWORD, DECRYPT_SALT, 1000);
            let iv: [u8; 16] = DECRYPT_IV.try_into().unwrap();

            // AES-128-CBC 解密
            let decrypted = aes_cbc_decrypt(&cipher_bytes, &key, &iv);

            // 转换为字符串，去除尾部零字节
            let s = String::from_utf8_lossy(&decrypted).into_owned();
            let s_trimmed = s.trim_end_matches('\0').to_string();
            let start_trimmed =
                s_trimmed.trim_start_matches(|c: char| c == '\u{FEFF}' || c.is_whitespace());

            // 验证解密结果是有效 XML
            if start_trimmed.starts_with('<') {
                return Ok(s_trimmed);
            }
        }
    }

    Ok(content)
}
