//! Small portable helpers shared by the semantic modules: SHA-256 (text
//! hashes and session keys), local calendar math from a UTC offset (Mac
//! `Calendar.current`), and string clipping.

/// SHA-256 digest of `data` (FIPS 180-4). Self-contained so the core needs no
/// crypto dependency for content hashing.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for block in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, v) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(v);
        }
    }
    let mut out = [0u8; 32];
    for (i, v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

/// Lowercase hex of the first `bytes` bytes of the SHA-256 digest.
pub fn sha256_hex(text: &str, bytes: usize) -> String {
    sha256(text.as_bytes())
        .iter()
        .take(bytes.min(32))
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// djb2 string hash (Mac `stableSemanticHash` / the canvas `stableHash`).
pub fn djb2(value: &str) -> u64 {
    let mut hash: u64 = 5381;
    for c in value.chars() {
        hash = (hash << 5).wrapping_add(hash).wrapping_add(c as u64);
    }
    hash
}

/// Clip to `limit` characters, ending with `…` when cut.
pub fn clipped(text: &str, limit: usize) -> String {
    let cleaned = text.trim();
    if cleaned.chars().count() <= limit {
        return cleaned.to_string();
    }
    let head: String = cleaned.chars().take(limit.saturating_sub(1)).collect();
    format!("{}…", head.trim_end())
}

/// Clip to `limit` characters, ending with `...` when cut (Mac `excerpt`).
pub fn excerpt(text: &str, limit: usize) -> String {
    let cleaned = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.chars().count() <= limit {
        return cleaned;
    }
    let head: String = cleaned.chars().take(limit.saturating_sub(3)).collect();
    format!("{}...", head.trim_end())
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const WEEKDAYS: [&str; 7] = [
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
];

/// "Now" plus the user's UTC offset, so day and hour buckets follow the local
/// calendar the way the Mac's `Calendar.current` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clock {
    /// Unix seconds.
    pub now: i64,
    /// Seconds east of UTC (e.g. +3600 for CET).
    pub tz_offset_secs: i64,
}

impl Clock {
    pub fn new(now: i64, tz_offset_minutes: i64) -> Self {
        Self {
            now,
            tz_offset_secs: tz_offset_minutes.clamp(-16 * 60, 16 * 60) * 60,
        }
    }

    pub fn utc(now: i64) -> Self {
        Self::new(now, 0)
    }

    /// Local day number (days since 1970-01-01 local).
    pub fn day(&self, ts: i64) -> i64 {
        (ts + self.tz_offset_secs).div_euclid(86_400)
    }

    /// Unix time of local midnight starting the day containing `ts`.
    pub fn start_of_day(&self, ts: i64) -> i64 {
        self.day(ts) * 86_400 - self.tz_offset_secs
    }

    /// Local hour 0–23.
    pub fn hour(&self, ts: i64) -> u32 {
        ((ts + self.tz_offset_secs).rem_euclid(86_400) / 3600) as u32
    }

    /// "Monday" …
    pub fn weekday_name(&self, ts: i64) -> &'static str {
        WEEKDAYS[self.day(ts).rem_euclid(7) as usize]
    }

    /// (year, month 1–12, day 1–31) in local time.
    pub fn civil(&self, ts: i64) -> (i64, u32, u32) {
        civil_from_days(self.day(ts))
    }

    /// "Mar 4".
    pub fn month_day(&self, ts: i64) -> String {
        let (_, m, d) = self.civil(ts);
        format!("{} {}", MONTHS[(m - 1) as usize], d)
    }

    /// "Mar 4, 2026, 14:05" (prompt/evidence formatting).
    pub fn medium(&self, ts: i64) -> String {
        let (y, m, d) = self.civil(ts);
        let secs = (ts + self.tz_offset_secs).rem_euclid(86_400);
        format!(
            "{} {}, {}, {:02}:{:02}",
            MONTHS[(m - 1) as usize],
            d,
            y,
            secs / 3600,
            (secs % 3600) / 60
        )
    }
}

/// Howard Hinnant's days → civil date.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_known_vectors() {
        assert_eq!(
            sha256_hex("", 32),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex("abc", 32),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let long = "a".repeat(1000);
        assert_eq!(sha256_hex(&long, 4).len(), 8);
    }

    #[test]
    fn clock_buckets_follow_offset() {
        // 2024-03-04 23:30 UTC is Tuesday 01:30 at UTC+2.
        let ts = 1_709_595_000;
        let utc = Clock::utc(ts);
        assert_eq!(utc.hour(ts), 23);
        assert_eq!(utc.weekday_name(ts), "Monday");
        assert_eq!(utc.month_day(ts), "Mar 4");
        let plus2 = Clock::new(ts, 120);
        assert_eq!(plus2.hour(ts), 1);
        assert_eq!(plus2.weekday_name(ts), "Tuesday");
        assert_eq!(plus2.day(ts), utc.day(ts) + 1);
        assert_eq!(plus2.start_of_day(ts) % 3600, 0);
    }

    #[test]
    fn clipping() {
        assert_eq!(clipped("hello world", 20), "hello world");
        assert_eq!(clipped("hello world", 6), "hello…");
        assert_eq!(excerpt("a  b\nc", 10), "a b c");
        assert!(excerpt(&"x".repeat(50), 10).ends_with("..."));
    }
}
