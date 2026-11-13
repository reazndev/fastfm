//! Response bodies with a strong entity tag and compressed copies made on first demand.
//!
//! A cached body is compressed at most once per encoding: the first request that accepts gzip
//! or zstd pays for it, and every later one reuses the bytes.

use std::fmt::Write as _;
use std::hash::{BuildHasher, Hash, RandomState};
use std::io::Write as _;
use std::ops::Deref;
use std::sync::{Arc, LazyLock, OnceLock};

use bytes::Bytes;

/// Bodies shorter than this are always sent as they are: compression would barely help.
const MIN_COMPRESSED: usize = 1024;

/// Levels picked for a miss, which compresses on the request path: a 1000-track page (830 KB of
/// JSON) takes 2.6 ms at gzip level 1 (to 175 KB) where level 6 takes 8 ms (to 125 KB), and
/// 1.7 ms at zstd level 3 (to 77 KB). Most clients ask for gzip, so its level sets the cost.
const GZIP_LEVEL: u32 = 1;
const ZSTD_LEVEL: i32 = 3;

/// Keys every tag hash with a secret chosen at start. A tag then never survives a restart,
/// which keeps it correct when the database or the renderer changed in between.
static TAG_KEYS: LazyLock<RandomState> = LazyLock::new(RandomState::new);

/// A content coding the mirror can send.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Encoding {
    /// The bytes as rendered.
    Identity,
    /// `gzip`.
    Gzip,
    /// `zstd`.
    Zstd,
}

impl Encoding {
    /// The best coding an `Accept-Encoding` header allows: zstd, then gzip, then identity.
    /// Codings with `q=0` are refused; `*` stands for every coding not named.
    #[must_use]
    pub fn negotiate(accept: &str) -> Self {
        let quality = |coding: &str| {
            let named = accept_quality(accept, |token| {
                token.eq_ignore_ascii_case(coding)
                    || (coding == "gzip" && token.eq_ignore_ascii_case("x-gzip"))
            });
            named.or_else(|| accept_quality(accept, |token| token == "*"))
        };
        [(Self::Zstd, "zstd"), (Self::Gzip, "gzip")]
            .into_iter()
            .map(|(encoding, name)| (encoding, quality(name).unwrap_or(0)))
            .filter(|(_, q)| *q > 0)
            // `max_by_key` keeps the last maximum, so reverse to prefer zstd on a tie.
            .rev()
            .max_by_key(|(_, q)| *q)
            .map_or(Self::Identity, |(encoding, _)| encoding)
    }

    /// The `Content-Encoding` value, `None` for identity.
    #[must_use]
    pub fn header(self) -> Option<&'static str> {
        match self {
            Self::Identity => None,
            Self::Gzip => Some("gzip"),
            Self::Zstd => Some("zstd"),
        }
    }
}

/// The quality, in thousandths, of the first coding in `accept` that `matches`.
fn accept_quality(accept: &str, matches: impl Fn(&str) -> bool) -> Option<u16> {
    accept.split(',').find_map(|item| {
        let mut parts = item.split(';');
        let token = parts.next()?.trim();
        if !matches(token) {
            return None;
        }
        let q = parts
            .filter_map(|param| param.trim().strip_prefix("q="))
            .find_map(|q| q.trim().parse::<f32>().ok())
            .unwrap_or(1.0);
        // q is between 0 and 1 with at most three decimals (RFC 9110, 12.4.2).
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped to 0..=1000 first"
        )]
        let millis = (q.clamp(0.0, 1.0) * 1000.0).round() as u16;
        Some(millis)
    })
}

/// A response body, its strong entity tag, and its compressed copies. Cheap to clone; clones
/// share the copies.
#[derive(Clone)]
pub struct Body {
    inner: Arc<Inner>,
}

struct Inner {
    bytes: Bytes,
    tag: u64,
    gzip: OnceLock<Option<Bytes>>,
    zstd: OnceLock<Option<Bytes>>,
}

impl Body {
    /// A body whose tag is derived from `identity`, such as a cache key that names the content
    /// exactly. Equal identities must mean equal bytes.
    #[must_use]
    pub fn tagged(bytes: Bytes, identity: &impl Hash) -> Self {
        Self::with_tag(bytes, TAG_KEYS.hash_one(identity))
    }

    /// A body tagged by a hash of its bytes.
    #[must_use]
    pub fn hashed(bytes: Bytes) -> Self {
        let tag = TAG_KEYS.hash_one(bytes.as_ref());
        Self::with_tag(bytes, tag)
    }

    fn with_tag(bytes: Bytes, tag: u64) -> Self {
        Self {
            inner: Arc::new(Inner {
                bytes,
                tag,
                gzip: OnceLock::new(),
                zstd: OnceLock::new(),
            }),
        }
    }

    /// The uncompressed bytes.
    #[must_use]
    pub fn bytes(&self) -> &Bytes {
        &self.inner.bytes
    }

    /// The bytes to send for `encoding`, and the coding they are in. A body that is too small
    /// or does not shrink is sent as it is.
    #[must_use]
    pub fn encoded(&self, encoding: Encoding) -> (Bytes, Encoding) {
        let copy = match encoding {
            Encoding::Identity => &None,
            Encoding::Gzip => self.inner.gzip.get_or_init(|| self.compress(gzip)),
            Encoding::Zstd => self.inner.zstd.get_or_init(|| self.compress(zstd)),
        };
        match copy {
            Some(bytes) => (bytes.clone(), encoding),
            None => (self.inner.bytes.clone(), Encoding::Identity),
        }
    }

    fn compress(&self, codec: fn(&[u8]) -> Option<Vec<u8>>) -> Option<Bytes> {
        let bytes = &self.inner.bytes;
        if bytes.len() < MIN_COMPRESSED {
            return None;
        }
        codec(bytes)
            .filter(|out| out.len() < bytes.len())
            .map(Bytes::from)
    }

    /// The `ETag` of the copy in `encoding`: a strong tag, different for each coding as
    /// RFC 9110 asks of strong validators.
    #[must_use]
    pub fn etag(&self, encoding: Encoding) -> String {
        let mut tag = format!("\"{:016x}", self.inner.tag);
        if let Some(coding) = encoding.header() {
            let _ = write!(tag, "-{coding}");
        }
        tag.push('"');
        tag
    }

    /// Whether an `If-None-Match` header names this body, in any coding. Uses the weak
    /// comparison RFC 9110 prescribes for `If-None-Match`.
    #[must_use]
    pub fn matches(&self, if_none_match: &str) -> bool {
        let tag = format!("{:016x}", self.inner.tag);
        if_none_match.split(',').map(str::trim).any(|candidate| {
            let candidate = candidate.strip_prefix("W/").unwrap_or(candidate);
            let opaque = candidate.trim_matches('"');
            candidate == "*"
                || opaque == tag
                || opaque
                    .strip_prefix(tag.as_str())
                    .is_some_and(|rest| matches!(rest, "-gzip" | "-zstd"))
        })
    }

    /// The bytes the body is expected to take in a cache: the identity plus room for its
    /// compressed copies, which are made later and about a tenth of the size each.
    #[must_use]
    pub fn weight(&self) -> u32 {
        let len = self.inner.bytes.len();
        u32::try_from(len + len / 4 + 128).unwrap_or(u32::MAX)
    }
}

fn gzip(bytes: &[u8]) -> Option<Vec<u8>> {
    let out = Vec::with_capacity(bytes.len() / 4);
    let mut encoder = flate2::write::GzEncoder::new(out, flate2::Compression::new(GZIP_LEVEL));
    encoder.write_all(bytes).ok()?;
    encoder.finish().ok()
}

fn zstd(bytes: &[u8]) -> Option<Vec<u8>> {
    zstd::bulk::compress(bytes, ZSTD_LEVEL).ok()
}

impl Deref for Body {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.inner.bytes
    }
}

impl PartialEq for Body {
    fn eq(&self, other: &Self) -> bool {
        self.inner.bytes == other.inner.bytes
    }
}

impl Eq for Body {}

impl std::fmt::Debug for Body {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Body")
            .field("len", &self.inner.bytes.len())
            .field("tag", &format_args!("{:016x}", self.inner.tag))
            .finish_non_exhaustive()
    }
}

impl From<Bytes> for Body {
    fn from(bytes: Bytes) -> Self {
        Self::hashed(bytes)
    }
}

impl From<Vec<u8>> for Body {
    fn from(bytes: Vec<u8>) -> Self {
        Self::hashed(Bytes::from(bytes))
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;

    use super::*;

    fn big() -> Bytes {
        Bytes::from("{\"track\":\"Lucid Dreams\"},".repeat(200))
    }

    #[test]
    fn negotiation_prefers_zstd_then_gzip_and_honours_q() {
        assert_eq!(
            Encoding::negotiate("gzip, deflate, br, zstd"),
            Encoding::Zstd
        );
        assert_eq!(Encoding::negotiate("gzip, deflate"), Encoding::Gzip);
        assert_eq!(Encoding::negotiate("zstd;q=0.5, gzip"), Encoding::Gzip);
        assert_eq!(
            Encoding::negotiate("gzip;q=0, zstd;q=0"),
            Encoding::Identity
        );
        assert_eq!(Encoding::negotiate("*"), Encoding::Zstd);
        assert_eq!(Encoding::negotiate("*;q=0, gzip"), Encoding::Gzip);
        assert_eq!(Encoding::negotiate("identity"), Encoding::Identity);
        assert_eq!(Encoding::negotiate("br"), Encoding::Identity);
        assert_eq!(Encoding::negotiate("X-GZIP"), Encoding::Gzip);
        assert_eq!(Encoding::negotiate(""), Encoding::Identity);
    }

    #[test]
    fn compressed_copies_decode_to_the_body_and_are_made_once() {
        let body = Body::hashed(big());
        let (gz, coding) = body.encoded(Encoding::Gzip);
        assert_eq!(coding, Encoding::Gzip);
        let mut decoded = Vec::new();
        flate2::read::GzDecoder::new(gz.as_ref())
            .read_to_end(&mut decoded)
            .unwrap();
        assert_eq!(decoded, big());
        let (zst, coding) = body.encoded(Encoding::Zstd);
        assert_eq!(coding, Encoding::Zstd);
        assert_eq!(zstd::decode_all(zst.as_ref()).unwrap(), big());
        let (again, _) = body.clone().encoded(Encoding::Gzip);
        assert_eq!(
            again.as_ptr(),
            gz.as_ptr(),
            "the copy is shared, not redone"
        );
    }

    #[test]
    fn small_bodies_stay_uncompressed() {
        let body = Body::hashed(Bytes::from_static(b"{\"error\":6}"));
        assert_eq!(body.encoded(Encoding::Zstd).1, Encoding::Identity);
    }

    #[test]
    fn etags_are_strong_per_coding_and_match_any_coding() {
        let body = Body::tagged(big(), &("user.getInfo", 3_u64));
        let plain = body.etag(Encoding::Identity);
        let gz = body.etag(Encoding::Gzip);
        assert!(plain.starts_with('"') && plain.ends_with('"') && !plain.starts_with("W/"));
        assert_ne!(plain, gz);
        assert!(body.matches(&plain));
        assert!(body.matches(&format!("\"other\", W/{gz}")));
        assert!(body.matches("*"));
        assert!(!body.matches("\"0000000000000000\""));
        let same = Body::tagged(Bytes::new(), &("user.getInfo", 3_u64));
        assert_eq!(same.etag(Encoding::Identity), plain);
        let next = Body::tagged(Bytes::new(), &("user.getInfo", 4_u64));
        assert_ne!(next.etag(Encoding::Identity), plain);
    }

    #[test]
    fn hashed_tags_follow_the_bytes() {
        assert_eq!(
            Body::hashed(big()).etag(Encoding::Identity),
            Body::hashed(big()).etag(Encoding::Identity)
        );
        assert_ne!(
            Body::hashed(big()).etag(Encoding::Identity),
            Body::hashed(Bytes::from_static(b"x")).etag(Encoding::Identity)
        );
    }
}
