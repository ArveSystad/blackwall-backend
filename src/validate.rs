use axum::http::HeaderMap;
use axum::http::header::IF_MATCH;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use crate::error::ApiError;

/// Checks `syncId` against `^[0-9a-f]{64}$`.
pub fn sync_id(id: &str) -> Result<(), ApiError> {
    let ok = id.len() == 64 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    if ok {
        Ok(())
    } else {
        Err(ApiError::BadRequest)
    }
}

/// Parses `If-Match` as a plain decimal version. `None` when the header is
/// absent; any other shape (repeated, quoted, signed, too large) is a 400.
pub fn if_match(headers: &HeaderMap) -> Result<Option<i64>, ApiError> {
    let mut values = headers.get_all(IF_MATCH).iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(ApiError::BadRequest);
    }
    let bytes = value.as_bytes();
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return Err(ApiError::BadRequest);
    }
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .map(Some)
        .ok_or(ApiError::BadRequest)
}

/// Decodes a base64url blob without padding.
pub fn decode_blob(blob: &str) -> Result<Vec<u8>, ApiError> {
    URL_SAFE_NO_PAD
        .decode(blob)
        .map_err(|_| ApiError::BadRequest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn sync_id_accepts_only_lowercase_hex_of_length_64() {
        assert!(sync_id(&"a1".repeat(32)).is_ok());
        assert!(sync_id(&"A1".repeat(32)).is_err());
        assert!(sync_id(&"a".repeat(63)).is_err());
        assert!(sync_id(&"a".repeat(65)).is_err());
        assert!(sync_id(&format!("{}g", "a".repeat(63))).is_err());
    }

    #[test]
    fn if_match_parses_plain_decimals_only() {
        let parse = |v: &str| {
            let mut h = HeaderMap::new();
            h.insert(IF_MATCH, HeaderValue::from_str(v).unwrap());
            if_match(&h)
        };
        assert_eq!(if_match(&HeaderMap::new()), Ok(None));
        assert_eq!(parse("7"), Ok(Some(7)));
        for bad in [
            "",
            "abc",
            "\"1\"",
            "-1",
            "+1",
            "1.0",
            " 1",
            "9223372036854775808",
        ] {
            assert_eq!(parse(bad), Err(ApiError::BadRequest), "{bad:?}");
        }
    }

    #[test]
    fn if_match_rejects_repeated_header() {
        let mut h = HeaderMap::new();
        h.append(IF_MATCH, HeaderValue::from_static("1"));
        h.append(IF_MATCH, HeaderValue::from_static("1"));
        assert_eq!(if_match(&h), Err(ApiError::BadRequest));
    }

    #[test]
    fn decode_blob_is_strict_base64url() {
        assert_eq!(decode_blob(""), Ok(vec![]));
        assert_eq!(decode_blob("-_8"), Ok(vec![0xfb, 0xff]));
        assert!(decode_blob("-_8=").is_err());
        assert!(decode_blob("+/8").is_err());
        assert!(decode_blob("a b").is_err());
    }
}
