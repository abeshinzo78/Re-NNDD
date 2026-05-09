//! HLS の `EXT-X-KEY:METHOD=AES-128` 用 復号。
//!
//! RFC 8216 §5.2:
//! - 鍵: 16 byte raw bytes (鍵 URI からそのまま取得)
//! - IV: EXT-X-KEY の `IV` 属性が無ければ media sequence number を 128bit BE で
//!   詰めたものを使う（niconico Domand は IV を毎回明示してくるが念のため対応）
//! - パディング: PKCS#7
//!
//! `aes` + `cbc` クレートで素直にやる。鍵 URI ごとの値を呼び出し側でキャッシュ
//! して、毎回 fetch しないようにすること。

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};

use crate::error::ApiError;

type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

/// 16 byte key + 16 byte IV で `ciphertext` を復号して `Vec<u8>` を返す。
///
/// ciphertext.len() は 16 の倍数でなければならない (HLS 規約)。最後のブロック
/// は PKCS#7 padding を取り除いて返す。
pub fn decrypt_aes_128_cbc(
    key: &[u8; 16],
    iv: &[u8; 16],
    ciphertext: &[u8],
) -> Result<Vec<u8>, ApiError> {
    if ciphertext.is_empty() {
        return Ok(Vec::new());
    }
    if !ciphertext.len().is_multiple_of(16) {
        return Err(ApiError::Downloader(format!(
            "AES-128-CBC ciphertext length {} is not a multiple of 16",
            ciphertext.len()
        )));
    }
    // cipher 0.4 の cbc::Decryptor は in-place 復号 (decrypt_padded_mut) で
    // padding 切り落とし後の slice を返す。所有権を持って使いたいので一旦
    // 入力をコピーしてから復号、結果の長さに truncate する。
    let mut buffer = ciphertext.to_vec();
    let cipher = Aes128CbcDec::new(key.into(), iv.into());
    let plain_len = cipher
        .decrypt_padded_mut::<Pkcs7>(&mut buffer)
        .map_err(|e| ApiError::Downloader(format!("AES-128-CBC decrypt failed: {e}")))?
        .len();
    buffer.truncate(plain_len);
    Ok(buffer)
}

/// EXT-X-KEY の `IV` 属性が省略されたときに使う既定 IV。
/// media sequence number を 128bit BE に詰めるだけ。
pub fn iv_from_media_sequence(seq: u64) -> [u8; 16] {
    let mut iv = [0u8; 16];
    iv[8..16].copy_from_slice(&seq.to_be_bytes());
    iv
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut};

    type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;

    fn encrypt(key: &[u8; 16], iv: &[u8; 16], plaintext: &[u8]) -> Vec<u8> {
        let block_size = 16usize;
        let mut buf = vec![0u8; plaintext.len() + block_size];
        let ciphertext = Aes128CbcEnc::new(key.into(), iv.into())
            .encrypt_padded_b2b_mut::<Pkcs7>(plaintext, &mut buf)
            .expect("encrypt fits in buffer");
        let len = ciphertext.len();
        buf.truncate(len);
        buf
    }

    #[test]
    fn round_trips_plaintext() {
        let key = [0x42u8; 16];
        let iv = [0xAAu8; 16];
        let plain = b"This is a test plaintext that crosses block boundaries.".to_vec();
        let cipher = encrypt(&key, &iv, &plain);
        let back = decrypt_aes_128_cbc(&key, &iv, &cipher).unwrap();
        assert_eq!(back, plain);
    }

    #[test]
    fn empty_input_yields_empty_output() {
        let key = [0u8; 16];
        let iv = [0u8; 16];
        assert!(decrypt_aes_128_cbc(&key, &iv, &[]).unwrap().is_empty());
    }

    #[test]
    fn rejects_non_16_byte_aligned_ciphertext() {
        let key = [0u8; 16];
        let iv = [0u8; 16];
        let bad = vec![0u8; 17];
        let err = decrypt_aes_128_cbc(&key, &iv, &bad).unwrap_err();
        assert!(matches!(err, ApiError::Downloader(_)));
    }

    #[test]
    fn iv_from_media_sequence_packs_be() {
        let iv = iv_from_media_sequence(0x0102030405060708);
        assert_eq!(iv[..8], [0u8; 8]);
        assert_eq!(iv[8..], [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
    }
}
