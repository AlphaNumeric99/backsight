//! Typed wrappers around the camera's control API methods.
//!
//! Method names and parameters follow pytapo (MIT). The responses are returned as JSON
//! for now; typed models follow once they are confirmed against real cameras.

use serde_json::{Value, json};

use crate::client::Camera;
use crate::error::{Error, Result};

impl Camera {
    /// `getDeviceInfo` → `device_info.basic_info`: model, name, firmware, MAC…
    pub async fn device_info(&self) -> Result<Value> {
        let result = self
            .execute(
                "getDeviceInfo",
                json!({ "device_info": { "name": ["basic_info"] } }),
            )
            .await?;
        pick(result, "/device_info/basic_info")
    }

    /// `getClockStatus` → the camera clock (`seconds_from_1970`, `local_time`).
    pub async fn clock_status(&self) -> Result<Value> {
        let result = self
            .execute(
                "getClockStatus",
                json!({ "system": { "name": "clock_status" } }),
            )
            .await?;
        pick(result, "/system/clock_status")
    }

    /// `getTimezone` → the camera's time zone settings.
    pub async fn timezone(&self) -> Result<Value> {
        let result = self
            .execute("getTimezone", json!({ "system": { "name": ["basic"] } }))
            .await?;
        pick(result, "/system/basic")
    }

    /// `getDstRule` → daylight saving settings.
    pub async fn dst_rule(&self) -> Result<Value> {
        let result = self
            .execute("getDstRule", json!({ "system": { "name": "dst" } }))
            .await?;
        pick(result, "/system/dst")
    }

    /// `getSdCardStatus` → one entry per storage device.
    pub async fn sd_cards(&self) -> Result<Value> {
        let result = self
            .execute(
                "getSdCardStatus",
                json!({ "harddisk_manage": { "table": ["hd_info"] } }),
            )
            .await?;
        pick(result, "/harddisk_manage/hd_info")
    }

    /// `getAudioConfig` → speaker, microphone (incl. sample rate) and record-audio settings.
    pub async fn audio_config(&self) -> Result<Value> {
        let result = self
            .execute(
                "getAudioConfig",
                json!({ "method": "get", "audio_config": { "name": ["speaker", "microphone", "record_audio"] } }),
            )
            .await?;
        pick(result, "/audio_config")
    }

    /// `getLensMaskConfig` → privacy mode state.
    pub async fn privacy_mode(&self) -> Result<Value> {
        let result = self
            .execute(
                "getLensMaskConfig",
                json!({ "lens_mask": { "name": ["lens_mask_info"] } }),
            )
            .await?;
        pick(result, "/lens_mask/lens_mask_info")
    }

    /// `getVideoCapability` → supported resolutions/codecs for the main and minor streams.
    pub async fn video_capability(&self) -> Result<Value> {
        self.execute(
            "getVideoCapability",
            json!({ "video_capability": { "name": ["main", "minor"] } }),
        )
        .await
    }

    /// `getUserID` → the id that playback searches and SD streaming need.
    pub async fn user_id(&self) -> Result<u64> {
        let result = self
            .execute("getUserID", json!({ "system": { "get_user_id": "null" } }))
            .await?;
        result
            .get("user_id")
            .and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok()))
            .ok_or_else(|| Error::protocol(format!("getUserID without user_id: {result}")))
    }

    /// `searchDateWithVideo` → which days between `start` and `end` (`YYYYMMDD`,
    /// camera-local) have recordings.
    pub async fn days_with_recordings(&self, start: &str, end: &str) -> Result<Value> {
        let result = self
            .execute(
                "searchDateWithVideo",
                json!({ "playback": { "search_year_utility": { "channel": [0], "start_date": start, "end_date": end } } }),
            )
            .await?;
        pick(result, "/playback/search_results")
    }

    /// `searchVideoOfDay` → the recordings of one day (`YYYYMMDD`, camera-local).
    pub async fn recordings_of_day(&self, date: &str, user_id: u64) -> Result<Value> {
        let result = self
            .execute(
                "searchVideoOfDay",
                json!({ "playback": { "search_video_utility": {
                    "channel": 0, "date": date, "id": user_id,
                    "start_index": 0, "end_index": 999_999_999u64,
                } } }),
            )
            .await?;
        pick(result, "/playback/search_video_results")
    }

    /// `searchDetectionList` → detection events between two camera-clock timestamps.
    pub async fn detection_events(&self, start_time: i64, end_time: i64) -> Result<Value> {
        let result = self
            .execute(
                "searchDetectionList",
                json!({ "playback": { "search_detection_list": {
                    "channel": 0, "start_time": start_time, "end_time": end_time,
                    "start_index": 0, "end_index": 999,
                } } }),
            )
            .await?;
        pick(result, "/playback/search_detection_list")
    }
}

fn pick(value: Value, pointer: &str) -> Result<Value> {
    value
        .pointer(pointer)
        .cloned()
        .ok_or_else(|| Error::protocol(format!("expected {pointer} in {value}")))
}
