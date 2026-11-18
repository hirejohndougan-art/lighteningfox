//! `TextDecoder`'s decoding (blob.js), on encoding_rs: every WHATWG encoding (UTF-16 included,
//! which Emscripten's start-up code asks for), and fast, since WebAssembly apps decode strings
//! out of their memory all the time.

use boa_engine::object::builtins::{JsArray, JsUint8Array};
use boa_engine::{Context, JsNativeError, JsResult, JsString, JsValue, NativeFunction};

pub fn install(ctx: &mut Context) {
    let natives: &[(&str, usize, fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>)] =
        &[("__encodingName", 1, n_encoding_name), ("__textDecode", 5, n_text_decode)];
    for (name, len, f) in natives {
        let _ = ctx.register_global_callable(JsString::from(*name), *len, NativeFunction::from_fn_ptr(*f));
    }
}

fn encoding(label: &str) -> Option<&'static encoding_rs::Encoding> {
    encoding_rs::Encoding::for_label(label.trim().as_bytes()).filter(|e| *e != encoding_rs::REPLACEMENT)
}

fn label_arg(a: &[JsValue], ctx: &mut Context) -> JsResult<String> {
    Ok(a.first().cloned().unwrap_or_default().to_string(ctx)?.to_std_string_escaped())
}

/// __encodingName(label) -> the encoding's name ("utf-8", "utf-16le", "shift_jis"…) or null.
fn n_encoding_name(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let label = label_arg(a, ctx)?;
    Ok(match encoding(&label) {
        Some(e) => JsString::from(e.name().to_ascii_lowercase()).into(),
        None => JsValue::null(),
    })
}

/// How many of `bytes` can be decoded now when more may follow (`stream`): a UTF-8 or UTF-16
/// character cut at the end waits for the next chunk.
fn complete_prefix(e: &'static encoding_rs::Encoding, bytes: &[u8]) -> usize {
    let n = bytes.len();
    if e == encoding_rs::UTF_8 {
        // Back up over at most 3 continuation bytes to a lead byte; keep it if its sequence is cut.
        let mut i = n;
        while i > 0 && n - i < 3 && bytes[i - 1] & 0xc0 == 0x80 {
            i -= 1;
        }
        if i > 0 {
            let lead = bytes[i - 1];
            let need = if lead >= 0xf0 { 4 } else if lead >= 0xe0 { 3 } else if lead >= 0xc0 { 2 } else { 1 };
            if n - (i - 1) < need {
                return i - 1;
            }
        }
        n
    } else if e == encoding_rs::UTF_16LE || e == encoding_rs::UTF_16BE {
        let even = n & !1;
        if even >= 2 {
            let unit = if e == encoding_rs::UTF_16LE {
                u16::from_le_bytes([bytes[even - 2], bytes[even - 1]])
            } else {
                u16::from_be_bytes([bytes[even - 2], bytes[even - 1]])
            };
            if (0xd800..0xdc00).contains(&unit) {
                return even - 2;
            }
        }
        even
    } else {
        n
    }
}

/// __textDecode(encodingName, Uint8Array, fatal, stripBOM, stream) -> [text, bytes used].
/// Throws a TypeError for malformed input when `fatal`.
fn n_text_decode(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let label = label_arg(a, ctx)?;
    let Some(enc) = encoding(&label) else {
        return Err(JsNativeError::range().with_message(format!("Unsupported encoding: {label}")).into());
    };
    let bytes = match a.get(1).and_then(|v| v.as_object()).and_then(|o| JsUint8Array::from_object(o.clone()).ok()) {
        Some(arr) => arr.to_vec(ctx)?,
        None => Vec::new(),
    };
    let fatal = a.get(2).is_some_and(JsValue::to_boolean);
    let strip_bom = a.get(3).is_some_and(JsValue::to_boolean);
    let stream = a.get(4).is_some_and(JsValue::to_boolean);
    let used = if stream { complete_prefix(enc, &bytes) } else { bytes.len() };
    let src = &bytes[..used];
    let mut decoder = if strip_bom { enc.new_decoder_with_bom_removal() } else { enc.new_decoder_without_bom_handling() };
    let mut text = String::with_capacity(decoder.max_utf8_buffer_length(src.len()).unwrap_or(src.len() * 3));
    if fatal {
        let (result, _) = decoder.decode_to_string_without_replacement(src, &mut text, true);
        if !matches!(result, encoding_rs::DecoderResult::InputEmpty) {
            return Err(JsNativeError::typ().with_message(format!("The data is not valid {}.", enc.name())).into());
        }
    } else {
        let _ = decoder.decode_to_string(src, &mut text, true);
    }
    Ok(JsArray::from_iter([JsValue::from(JsString::from(text)), JsValue::from(used as f64)], ctx).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_keeps_cut_characters() {
        assert_eq!(complete_prefix(encoding_rs::UTF_8, "aé".as_bytes()), 3);
        assert_eq!(complete_prefix(encoding_rs::UTF_8, &"a€".as_bytes()[..3]), 1);
        assert_eq!(complete_prefix(encoding_rs::UTF_8, &"😀".as_bytes()[..3]), 0);
        assert_eq!(complete_prefix(encoding_rs::UTF_16LE, &[0x61, 0, 0x3d]), 2);
        assert_eq!(complete_prefix(encoding_rs::UTF_16LE, &[0x3d, 0xd8]), 0);
    }

    #[test]
    fn text_decoder_in_script() {
        let mut js = crate::js::Js::new("https://decode.example/");
        let r = js.run("const u = new Uint8Array([0x68, 0, 0x69, 0, 0x3d, 0xd8, 0, 0xde]); new TextDecoder('utf-16le').decode(u) === 'hi\\u{1f600}'");
        assert_eq!(r.unwrap(), "true");
        assert_eq!(js.run("new TextDecoder('UTF-16').encoding + ' ' + new TextDecoder(' Shift_JIS ').encoding").unwrap(), "\"utf-16le shift_jis\"");
        assert_eq!(js.run("new TextDecoder('shift_jis').decode(new Uint8Array([0x82, 0xa0]))").unwrap(), "\"あ\"");
        assert_eq!(js.run("try { new TextDecoder('nope') } catch (e) { e.name }").unwrap(), "\"RangeError\"");
        assert_eq!(js.run("try { new TextDecoder('utf-8', {fatal: true}).decode(new Uint8Array([0xff])) } catch (e) { e.name }").unwrap(), "\"TypeError\"");
        // A BOM goes unless ignoreBOM; a character split across stream chunks comes out whole.
        assert_eq!(js.run("new TextDecoder().decode(new Uint8Array([0xef, 0xbb, 0xbf, 0x41])) + new TextDecoder('utf-8', {ignoreBOM: true}).decode(new Uint8Array([0xef, 0xbb, 0xbf])).length").unwrap(), "\"A1\"");
        let r = js.run("const d = new TextDecoder(); const e = new TextEncoder().encode('€'); d.decode(e.subarray(0, 2), {stream: true}) + d.decode(e.subarray(2))");
        assert_eq!(r.unwrap(), "\"€\"");
    }
}
