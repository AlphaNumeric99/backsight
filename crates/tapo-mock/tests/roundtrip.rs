//! `tapo-camera` against the fake camera: login, API calls and the media stream.

use std::time::Duration;

use tapo_camera::stream::{MediaConfig, MediaSession, Quality, StreamPart, StreamRequest};
use tapo_camera::{Camera, CameraConfig, Error};
use tapo_mock::{MockCamera, MockOptions};

async fn start() -> MockCamera {
    MockCamera::start(MockOptions::new("s3cret")).await.unwrap()
}

fn config(mock: &MockCamera, password: &str) -> CameraConfig {
    let mut config = CameraConfig::new(mock.control_addr.ip().to_string(), password);
    config.port = mock.control_addr.port();
    config
}

#[tokio::test]
async fn login_and_read_the_api() {
    let mock = start().await;
    let camera = Camera::connect(config(&mock, "s3cret")).await.unwrap();
    assert!(camera.certificate().await.is_some());

    let info = camera.device_info().await.unwrap();
    assert_eq!(info["device_model"], "C200");
    let clock = camera.clock_status().await.unwrap();
    assert!(clock["seconds_from_1970"].as_i64().unwrap() > 1_700_000_000);

    let today = jiff::Zoned::now().date();
    let month_start = today.first_of_month().strftime("%Y%m%d").to_string();
    let month_end = today.last_of_month().strftime("%Y%m%d").to_string();
    let days = camera
        .days_with_recordings(&month_start, &month_end)
        .await
        .unwrap();
    assert!(days.as_array().is_some());

    let user_id = camera.user_id().await.unwrap();
    let yesterday = today.yesterday().unwrap().strftime("%Y%m%d").to_string();
    let recordings = camera.recordings_of_day(&yesterday, user_id).await.unwrap();
    assert!(!recordings.as_array().unwrap().is_empty());

    // A batch with an unsupported method: the others still succeed.
    let results = camera
        .execute_many(&[
            (
                "getLensMaskConfig",
                serde_json::json!({ "lens_mask": { "name": ["lens_mask_info"] } }),
            ),
            ("getNotAThing", serde_json::json!({})),
        ])
        .await
        .unwrap();
    assert!(results[0].is_ok());
    assert_eq!(results[1].as_ref().unwrap_err().camera_code(), Some(-40106));
}

#[tokio::test]
async fn wrong_password_never_costs_a_failed_login() {
    let mock = start().await;
    // More attempts than the camera's lockout threshold: the client spots the wrong
    // password from `device_confirm` and never sends a digest.
    for _ in 0..12 {
        let err = Camera::connect(config(&mock, "nope")).await.unwrap_err();
        assert!(matches!(err, Error::BadCredentials), "{err:?}");
    }
    Camera::connect(config(&mock, "s3cret")).await.unwrap();
}

#[tokio::test]
async fn pinned_certificate_is_enforced() {
    let mock = start().await;
    let first = Camera::connect(config(&mock, "s3cret")).await.unwrap();
    let pin = first.certificate().await.unwrap();

    // Same camera, same pin: fine.
    Camera::connect(config(&mock, "s3cret").with_certificate(Some(pin)))
        .await
        .unwrap();

    // A different camera answering at the pinned address is refused.
    let other = start().await;
    let err = Camera::connect(config(&other, "s3cret").with_certificate(Some(pin)))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::CertificateMismatch { .. }), "{err:?}");
}

fn media_config(mock: &MockCamera, password: &str) -> MediaConfig {
    let mut config = MediaConfig::new(mock.media_addr.ip().to_string(), password);
    config.port = mock.media_addr.port();
    config
}

#[tokio::test]
async fn live_stream_delivers_ts() {
    let mock = start().await;
    let mut session = MediaSession::connect(&media_config(&mock, "s3cret"))
        .await
        .unwrap();
    session
        .start(&StreamRequest::Live {
            quality: Quality::High,
            channel: 0,
        })
        .await
        .unwrap();

    let mut media_parts = 0;
    while media_parts < 5 {
        let part = tokio::time::timeout(Duration::from_secs(5), session.next_part())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let StreamPart::Media { data, .. } = part {
            assert_eq!(data[0], 0x47, "decrypted TS starts with a sync byte");
            assert_eq!(data.len() % 188, 0);
            media_parts += 1;
        }
    }
    assert_eq!(session.session_id(), Some("1"));
}

#[tokio::test]
async fn download_is_fast_and_finishes() {
    let mock = start().await;
    let mut config = media_config(&mock, "s3cret");
    config.window_size = Some(50);
    let mut session = MediaSession::connect(&config).await.unwrap();
    session
        .start(&StreamRequest::Download {
            client_id: 42,
            start: 1_000,
            end: 1_004,
            player_id: "t".into(),
        })
        .await
        .unwrap();

    let started = std::time::Instant::now();
    let mut bytes = 0;
    loop {
        let part = tokio::time::timeout(Duration::from_secs(5), session.next_part())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if part.is_finished() {
            break;
        }
        if let StreamPart::Media { data, .. } = part {
            bytes += data.len();
        }
    }
    // Four seconds of video at ~10× real time.
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    assert!(bytes > 200_000, "{bytes}");
}

#[tokio::test]
async fn media_rejects_wrong_password() {
    let mock = start().await;
    let err = MediaSession::connect(&media_config(&mock, "nope"))
        .await
        .err()
        .unwrap();
    assert!(matches!(err, Error::BadCredentials), "{err:?}");
}
