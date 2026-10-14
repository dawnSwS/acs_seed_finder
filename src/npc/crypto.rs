use base64::{engine::general_purpose::STANDARD, Engine};
use std::{error::Error, fs, io, path::Path};

const DECRYPT_PASSWORD: &[u8] = b"no spoilers please";
const DECRYPT_SALT: &[u8] = b"XhSG9hLyZ7T~Ge3@";
const DECRYPT_IV: [u8; 16] = *b"~8YSi0Xv2@|{aDfb";

fn invalid_data(message: impl Into<Box<dyn Error + Send + Sync>>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// Matches Rfc2898DeriveBytes(password, salt, 1000).GetBytes(16).
fn derive_key() -> [u8; 16] {
    let mut key = [0; 16];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(DECRYPT_PASSWORD, DECRYPT_SALT, 1000, &mut key);
    key
}

/// The game uses AES-128-CBC with PaddingMode.None and zero-padded plaintext.
/// Padding damaged ciphertext before decrypting would fabricate corrupt data.
fn aes_cbc_decrypt(ciphertext: &[u8]) -> io::Result<Vec<u8>> {
    use aes::cipher::{generic_array::GenericArray, BlockDecrypt, KeyInit};
    if ciphertext.is_empty() || !ciphertext.len().is_multiple_of(16) {
        return Err(invalid_data(
            "encrypted data length must be a nonzero multiple of 16",
        ));
    }
    let cipher = aes::Aes128::new_from_slice(&derive_key()).map_err(invalid_data)?;
    let mut plaintext = ciphertext.to_vec();
    let mut previous = DECRYPT_IV;
    for chunk in plaintext.as_chunks_mut::<16>().0 {
        let mut block = GenericArray::clone_from_slice(chunk);
        let mut current = [0; 16];
        current.copy_from_slice(chunk);
        cipher.decrypt_block(&mut block);
        for ((output, decoded), prior) in chunk.iter_mut().zip(block.iter()).zip(previous) {
            *output = *decoded ^ prior;
        }
        previous = current;
    }
    while plaintext.last() == Some(&0) {
        plaintext.pop();
    }
    Ok(plaintext)
}

pub fn decode_and_decrypt(bytes: &[u8]) -> io::Result<String> {
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => std::borrow::Cow::Borrowed(text),
        Err(_) => encoding_rs::GBK
            .decode_without_bom_handling_and_without_replacement(bytes)
            .ok_or_else(|| invalid_data("invalid UTF-8 or GBK text"))?,
    };
    let trimmed = text.trim_start_matches(|c: char| c == '\u{feff}' || c.is_whitespace());
    let Some(encoded) = trimmed.strip_prefix("|E|") else {
        return Ok(text.into_owned());
    };
    let encoded: String = encoded
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    let ciphertext = STANDARD.decode(encoded).map_err(invalid_data)?;
    String::from_utf8(aes_cbc_decrypt(&ciphertext)?).map_err(invalid_data)
}

pub fn read_and_decrypt(path: &Path) -> io::Result<String> {
    decode_and_decrypt(&fs::read(path)?)
}
