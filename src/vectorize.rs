// src/vectorize.rs
// Converts a transaction payload into a 14-dimensional normalized vector.
// All constants match normalization.json exactly.

const MAX_AMOUNT: f32 = 10_000.0;
const MAX_INSTALLMENTS: f32 = 12.0;
const AMOUNT_VS_AVG_RATIO: f32 = 10.0;
const MAX_MINUTES: f32 = 1_440.0;
const MAX_KM: f32 = 1_000.0;
const MAX_TX_COUNT_24H: f32 = 20.0;
const MAX_MERCHANT_AVG_AMOUNT: f32 = 10_000.0;

#[inline(always)]
fn clamp01(x: f32) -> f32 {
    x.min(1.0).max(0.0)
}

#[inline]
pub fn vectorize_payload(
    amount: f32,
    installments: f32,
    customer_avg_amount: f32,
    requested_at: &str,
    last_tx_timestamp: Option<&str>,
    last_tx_km: Option<f32>,
    km_from_home: f32,
    tx_count_24h: f32,
    is_online: bool,
    card_present: bool,
    merchant_id: &str,
    known_merchants: &[&str],
    mcc: &str,
    merchant_avg_amount: f32,
) -> [f32; 14] {
    let (hour, dow) = parse_timestamp_hour_dow(requested_at);

    let minutes_since = match last_tx_timestamp {
        Some(ts) => clamp01(minutes_between(ts, requested_at) / MAX_MINUTES),
        None => -1.0,
    };

    let km_from_last = match last_tx_km {
        Some(km) => clamp01(km / MAX_KM),
        None => -1.0,
    };

    let unknown_merchant = if known_merchants.contains(&merchant_id) { 0.0_f32 } else { 1.0_f32 };

    let avg = customer_avg_amount.max(0.001);

    [
        clamp01(amount / MAX_AMOUNT),                              // 0
        clamp01(installments / MAX_INSTALLMENTS),                  // 1
        clamp01((amount / avg) / AMOUNT_VS_AVG_RATIO),             // 2
        hour as f32 / 23.0,                                        // 3
        dow as f32 / 6.0,                                          // 4
        minutes_since,                                             // 5
        km_from_last,                                              // 6
        clamp01(km_from_home / MAX_KM),                            // 7
        clamp01(tx_count_24h / MAX_TX_COUNT_24H),                  // 8
        if is_online { 1.0 } else { 0.0 },                        // 9
        if card_present { 1.0 } else { 0.0 },                     // 10
        unknown_merchant,                                          // 11
        mcc_risk(mcc),                                             // 12
        clamp01(merchant_avg_amount / MAX_MERCHANT_AVG_AMOUNT),    // 13
    ]
}

// ─── MCC risk (hardcoded from mcc_risk.json) ──────────────────────────────────
fn mcc_risk(mcc: &str) -> f32 {
    match mcc {
        "5411" => 0.15,
        "5812" => 0.30,
        "5912" => 0.20,
        "5944" => 0.45,
        "7801" => 0.80,
        "7802" => 0.75,
        "7995" => 0.85,
        "4511" => 0.35,
        "5311" => 0.25,
        "5999" => 0.50,
        _ => 0.50,
    }
}

// ─── Timestamp parsing (no-alloc, no dependencies) ───────────────────────────
// Input: "2026-03-11T18:45:53Z"  (ISO 8601 UTC)
fn parse_timestamp_hour_dow(ts: &str) -> (u8, u8) {
    let b = ts.as_bytes();
    if b.len() < 19 {
        return (0, 0);
    }
    let year  = dec4(&b[0..4]) as i32;
    let month = dec2(&b[5..7]) as i32;
    let day   = dec2(&b[8..10]) as i32;
    let hour  = dec2(&b[11..13]) as u8;

    // Tomohiko Sakamoto's day-of-week (0=Sun..6=Sat)
    static T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 3 { year - 1 } else { year };
    let dow_sun = ((y + y/4 - y/100 + y/400 + T[(month-1) as usize] + day) % 7) as u8;
    // Convert to Mon=0..Sun=6
    let dow_mon = if dow_sun == 0 { 6 } else { dow_sun - 1 };

    (hour, dow_mon)
}

// Parse minute-precision epoch from ISO timestamp for delta computation
fn parse_epoch_minutes(ts: &str) -> i64 {
    let b = ts.as_bytes();
    if b.len() < 16 { return 0; }
    let year  = dec4(&b[0..4]) as i64;
    let month = dec2(&b[5..7]) as i64;
    let day   = dec2(&b[8..10]) as i64;
    let hour  = dec2(&b[11..13]) as i64;
    let min   = dec2(&b[14..16]) as i64;
    civil_to_days(year, month, day) * 1440 + hour * 60 + min
}

fn minutes_between(prev: &str, curr: &str) -> f32 {
    let delta = parse_epoch_minutes(curr) - parse_epoch_minutes(prev);
    if delta > 0 { delta as f32 } else { 0.0 }
}

// Rata Die days from civil date
fn civil_to_days(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe
}

#[inline(always)]
fn dec2(b: &[u8]) -> u32 {
    (b[0] - b'0') as u32 * 10 + (b[1] - b'0') as u32
}

#[inline(always)]
fn dec4(b: &[u8]) -> u32 {
    (b[0] - b'0') as u32 * 1000
        + (b[1] - b'0') as u32 * 100
        + (b[2] - b'0') as u32 * 10
        + (b[3] - b'0') as u32
}
