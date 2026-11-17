//! Natives behind `crypto` (src/blob.js): secure random bytes and SHA digests. Bytes cross
//! into JavaScript as strings of char codes 0-255.

use boa_engine::{Context, JsNativeError, JsResult, JsString, JsValue};

fn bytes_to_js(b: &[u8]) -> JsValue {
    JsValue::from(JsString::from(b.iter().map(|&c| u16::from(c)).collect::<Vec<u16>>().as_slice()))
}

/// __randomBytes(n) -> n random bytes from the operating system (n <= 65536).
pub fn n_random_bytes(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let n = a.first().cloned().unwrap_or_default().to_number(ctx)?;
    let n = if n.is_finite() && n > 0.0 { (n as usize).min(65536) } else { 0 };
    let mut buf = vec![0u8; n];
    getrandom::getrandom(&mut buf).map_err(|e| JsNativeError::error().with_message(format!("no random numbers: {e}")))?;
    Ok(bytes_to_js(&buf))
}

/// __digest(algorithm, bytes) -> the digest's bytes, or undefined for an unknown algorithm.
pub fn n_digest(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    use sha2::Digest;
    let alg = a.first().cloned().unwrap_or_default().to_string(ctx)?.to_std_string_escaped().to_ascii_uppercase();
    let data: Vec<u8> = a.get(1).cloned().unwrap_or_default().to_string(ctx)?.iter().map(|c| c as u8).collect();
    let out = match alg.as_str() {
        "SHA-1" => sha1::Sha1::digest(&data).to_vec(),
        "SHA-256" => sha2::Sha256::digest(&data).to_vec(),
        "SHA-384" => sha2::Sha384::digest(&data).to_vec(),
        "SHA-512" => sha2::Sha512::digest(&data).to_vec(),
        _ => return Ok(JsValue::undefined()),
    };
    Ok(bytes_to_js(&out))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn digests_and_randomness() {
        let mut ctx = Context::default();
        let d = n_digest(&JsValue::undefined(), &[JsValue::from(JsString::from("SHA-256")), JsValue::from(JsString::from("abc"))], &mut ctx).unwrap();
        let hex: String = d.to_string(&mut ctx).unwrap().iter().map(|c| format!("{:02x}", c)).collect();
        assert_eq!(hex, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let r = n_random_bytes(&JsValue::undefined(), &[JsValue::from(32)], &mut ctx).unwrap();
        assert_eq!(r.to_string(&mut ctx).unwrap().len(), 32);
    }
}
