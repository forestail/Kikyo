use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 配列定義名ごとの全期間統計レコード
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AnalyticsRecord {
    /// 配列定義名
    pub layout_name: String,
    /// 記録開始日（"2026-04-18" 形式）
    pub start_date: String,
    /// 直近の記録更新日
    pub last_updated_date: String,
    /// 物理キー押下数（Down イベントのみカウント）
    pub physical_keystrokes: u64,
    /// 出力仮想キー数（Inject で出力されたキーストローク/Unicode の合計）
    pub output_virtual_keys: u64,
    /// キー名→押下回数（ヒートマップ用）
    #[serde(default)]
    pub key_counts: HashMap<String, u64>,
}

/// 永続化用のデータ構造全体
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AnalyticsData {
    /// 集計が有効かどうか
    #[serde(default)]
    pub enabled: bool,
    /// 配列定義名ごとのレコード
    #[serde(default)]
    pub records: Vec<AnalyticsRecord>,
}

/// インメモリの統計カウンタ。
/// Engine 内に保持し、process_key の結果に応じてインクリメントする。
#[derive(Debug, Clone, Default)]
pub struct AnalyticsCollector {
    /// 集計が有効かどうか
    enabled: bool,
    /// 現在の配列定義名
    current_layout_name: String,
    /// 配列定義名ごとのレコード
    records: HashMap<String, AnalyticsRecord>,
    /// ダーティフラグ（永続化が必要かどうか）
    dirty: bool,
}

impl AnalyticsCollector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn set_layout_name(&mut self, name: String) {
        self.current_layout_name = name;
    }

    pub fn layout_name(&self) -> &str {
        &self.current_layout_name
    }

    /// 永続化データからインメモリ状態を復元する
    pub fn load_from_data(&mut self, data: &AnalyticsData) {
        self.enabled = data.enabled;
        self.records.clear();

        for record in &data.records {
            self.records.insert(record.layout_name.clone(), record.clone());
        }
        self.dirty = false;
    }

    /// すべてのレコードを AnalyticsData として返す
    pub fn to_data(&self) -> AnalyticsData {
        let records = self.records.values().cloned().collect();
        AnalyticsData {
            enabled: self.enabled,
            records,
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn clear_dirty(&mut self) {
        self.dirty = false;
    }

    /// 物理キー押下を記録する（KeyDown 時のみ呼ぶこと）
    pub fn record_physical_keystroke(&mut self, key_name: Option<&str>) {
        if !self.enabled {
            return;
        }
        let record = self.get_or_create_current_record();
        record.physical_keystrokes += 1;
        if let Some(name) = key_name {
            *record.key_counts.entry(name.to_string()).or_insert(0) += 1;
        }
        self.dirty = true;
    }

    /// 出力仮想キー数を記録する（Inject 時）
    pub fn record_output_virtual_keys(&mut self, count: u64) {
        if !self.enabled || count == 0 {
            return;
        }
        let record = self.get_or_create_current_record();
        record.output_virtual_keys += count;
        self.dirty = true;
    }

    /// 全データをクリアする
    pub fn clear_all(&mut self) {
        self.records.clear();
        self.dirty = true;
    }

    fn get_or_create_current_record(&mut self) -> &mut AnalyticsRecord {
        let layout_name = self.current_layout_name.clone();
        let today = today_date_string();
        let record = self.records.entry(layout_name.clone()).or_insert_with(|| AnalyticsRecord {
            layout_name,
            start_date: today.clone(),
            last_updated_date: today.clone(),
            physical_keystrokes: 0,
            output_virtual_keys: 0,
            key_counts: HashMap::new(),
        });
        record.last_updated_date = today;
        record
    }
}

pub fn today_date_string() -> String {
    // Use chrono-free approach: SystemTime -> days since epoch
    let now = std::time::SystemTime::now();
    let since_epoch = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();

    // ローカルタイムゾーンのオフセットを考慮
    // Rust の std だけでは正確なローカルオフセットが取れないため、
    // UTC + 9 (JST) を仮定してもよいが、汎用性のため UTC で管理する
    // （UI 側で表示時に調整可能）
    let total_secs = since_epoch.as_secs();

    // UTC オフセットを推定（Windows では _get_timezone API を使うのが理想だが、
    // 簡易的に環境変数やシステムAPIなしで UTC ベースにする）
    // 実用上、ローカル日付で集計したいので local_offset_secs を加算
    let local_offset_secs = local_utc_offset_seconds();
    let local_secs = total_secs as i64 + local_offset_secs;

    let days = local_secs / 86400;
    // 1970-01-01 からの日数を年月日に変換
    let (year, month, day) = days_to_ymd(days);
    format!("{:04}-{:02}-{:02}", year, month, day)
}

/// ローカルタイムゾーンの UTC オフセット（秒）を取得する。
/// 取得できなければ 0（UTC）を返す。
fn local_utc_offset_seconds() -> i64 {
    #[cfg(target_os = "windows")]
    {
        windows_local_offset()
    }
    #[cfg(not(target_os = "windows"))]
    {
        unix_local_offset()
    }
}

#[cfg(target_os = "windows")]
fn windows_local_offset() -> i64 {
    use std::mem::MaybeUninit;
    // Windows: GetTimeZoneInformation
    #[repr(C)]
    struct TimeZoneInformation {
        bias: i32,
        _standard_name: [u16; 32],
        _standard_date: [u16; 8],
        _standard_bias: i32,
        _daylight_name: [u16; 32],
        _daylight_date: [u16; 8],
        _daylight_bias: i32,
    }

    extern "system" {
        fn GetTimeZoneInformation(tzi: *mut TimeZoneInformation) -> u32;
    }

    unsafe {
        let mut tzi = MaybeUninit::<TimeZoneInformation>::zeroed().assume_init();
        let result = GetTimeZoneInformation(&mut tzi);
        // Bias is in minutes, west of UTC (so JST = -540).
        // result: 0=Unknown, 1=Standard, 2=Daylight
        let total_bias = if result == 2 {
            tzi.bias + tzi._daylight_bias
        } else {
            tzi.bias
        };
        (-total_bias as i64) * 60
    }
}

#[cfg(not(target_os = "windows"))]
fn unix_local_offset() -> i64 {
    // macOS/Linux: use libc::localtime_r
    use std::mem::MaybeUninit;
    extern "C" {
        fn time(tloc: *mut i64) -> i64;
        fn localtime_r(timep: *const i64, result: *mut libc_tm) -> *mut libc_tm;
    }
    #[repr(C)]
    struct libc_tm {
        tm_sec: i32,
        tm_min: i32,
        tm_hour: i32,
        tm_mday: i32,
        tm_mon: i32,
        tm_year: i32,
        tm_wday: i32,
        tm_yday: i32,
        tm_isdst: i32,
        tm_gmtoff: i64,
        _tm_zone: *const i8,
    }

    unsafe {
        let mut t: i64 = 0;
        time(&mut t);
        let mut tm = MaybeUninit::<libc_tm>::zeroed().assume_init();
        localtime_r(&t, &mut tm);
        tm.tm_gmtoff
    }
}

/// 1970-01-01 からの通算日数を (year, month, day) に変換する。
/// 負の日数（1970以前）も扱える。
fn days_to_ymd(days: i64) -> (i32, u32, u32) {
    // Civil calendar algorithm from Howard Hinnant
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = (yoe as i64 + era * 400) as i32;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Inject イベントリストから出力仮想キー数をカウントするヘルパー
/// keyboard_map を使い、キーボードマトリクス上の文字キー（数字行・QWERTY行・ASDF行・ZXCV行）
/// のスキャンコードのみをカウントする。Unicode / DirectString はすべてカウントする。
pub fn count_output_virtual_keys(
    events: &[crate::types::InputEvent],
    keyboard_map: &crate::keyboard_map::KeyboardMap,
) -> u64 {
    let mut count: u64 = 0;
    for event in events {
        match event {
            crate::types::InputEvent::Scancode(sc, _ext, up) => {
                if !up {
                    // Only count character keys (those with a row/col in the keyboard matrix).
                    // This excludes Space, Enter, Backspace, Shift, etc.
                    let sc_key = crate::types::ScKey::new(*sc, false);
                    if keyboard_map.sc_to_rc.contains_key(&sc_key) {
                        count += 1;
                    }
                }
            }
            crate::types::InputEvent::Unicode(_, up) => {
                if !up {
                    count += 1;
                }
            }
            crate::types::InputEvent::DirectString(s) => {
                count += s.chars().count() as u64;
            }
            _ => {}
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_days_to_ymd_epoch() {
        assert_eq!(days_to_ymd(0), (1970, 1, 1));
    }

    #[test]
    fn test_days_to_ymd_known_date() {
        // 2026-04-18 = 20561 days since 1970-01-01
        assert_eq!(days_to_ymd(20561), (2026, 4, 18));
    }

    #[test]
    fn test_collector_disabled_no_record() {
        let mut c = AnalyticsCollector::new();
        c.set_enabled(false);
        c.set_layout_name("test".to_string());
        c.record_physical_keystroke(Some("a"));
        c.record_output_virtual_keys(3);
        let data = c.to_data();
        assert!(data.records.is_empty());
    }

    #[test]
    fn test_collector_enabled_records() {
        let mut c = AnalyticsCollector::new();
        c.set_enabled(true);
        c.set_layout_name("test_layout".to_string());
        c.record_physical_keystroke(Some("a"));
        c.record_physical_keystroke(Some("a"));
        c.record_physical_keystroke(Some("b"));
        c.record_output_virtual_keys(5);
        let data = c.to_data();
        assert_eq!(data.records.len(), 1);
        let r = &data.records[0];
        assert_eq!(r.physical_keystrokes, 3);
        assert_eq!(r.output_virtual_keys, 5);
        assert_eq!(r.key_counts.get("a"), Some(&2));
        assert_eq!(r.key_counts.get("b"), Some(&1));
    }

    #[test]
    fn test_count_output_virtual_keys() {
        use crate::types::InputEvent;
        let map = crate::keyboard_map::new_jis_106();
        let events = vec![
            InputEvent::Scancode(0x1E, false, false), // 'a' down (character key)
            InputEvent::Scancode(0x1E, false, true),  // 'a' up
            InputEvent::Unicode('あ', false),          // unicode down
            InputEvent::Unicode('あ', true),           // unicode up
            InputEvent::DirectString("こんにちは".to_string()),
        ];
        assert_eq!(count_output_virtual_keys(&events, &map), 7); // 1 + 1 + 5
    }

    #[test]
    fn test_count_output_virtual_keys_excludes_non_character_keys() {
        use crate::types::InputEvent;
        let map = crate::keyboard_map::new_jis_106();
        let events = vec![
            InputEvent::Scancode(0x14, false, false), // 't' down (character key)
            InputEvent::Scancode(0x14, false, true),  // 't' up
            InputEvent::Scancode(0x2A, false, false), // Left Shift down (modifier - NOT in sc_to_rc)
            InputEvent::Scancode(0x1E, false, false), // 'a' down (character key)
            InputEvent::Scancode(0x1E, false, true),  // 'a' up
            InputEvent::Scancode(0x2A, false, true),  // Left Shift up
            InputEvent::Scancode(0x39, false, false), // Space down (NOT in sc_to_rc)
            InputEvent::Scancode(0x39, false, true),  // Space up
            InputEvent::Scancode(0x1C, false, false), // Enter down (NOT in sc_to_rc)
            InputEvent::Scancode(0x1C, false, true),  // Enter up
        ];
        // Only 't' and 'a' are character keys = 2
        assert_eq!(count_output_virtual_keys(&events, &map), 2);
    }

    #[test]
    fn test_load_from_data() {
        let data = AnalyticsData {
            enabled: true,
            records: vec![
                AnalyticsRecord {
                    start_date: "2020-01-01".to_string(),
                    last_updated_date: "2020-01-01".to_string(),
                    layout_name: "old".to_string(),
                    physical_keystrokes: 100,
                    output_virtual_keys: 200,
                    key_counts: HashMap::new(),
                },
            ],
        };
        let mut c = AnalyticsCollector::new();
        c.load_from_data(&data);
        assert!(c.is_enabled());
        let out = c.to_data();
        assert_eq!(out.records.len(), 1);
        assert_eq!(out.records[0].start_date, "2020-01-01");
    }
}
