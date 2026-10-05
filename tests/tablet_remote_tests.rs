use std::sync::Arc;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt; // for `oneshot`

use deskdsp_control::audio::AudioMeters;
use deskdsp_control::remote::{LiveMetersState, OutputLevelsState, PreampChannelState, RemoteMessage, TabletRemoteServer};

#[tokio::test]
async fn test_tablet_remote_serves_html_touch_ui() {
    let meters = Arc::new(AudioMeters::default());
    let server = TabletRemoteServer::new(None, meters, 8080);
    let app = server.router();

    let req = Request::builder()
        .uri("/")
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let html = String::from_utf8(body_bytes.to_vec()).unwrap();

    // Verify key touch elements and styles are present
    assert!(html.contains("DeskDSP Remote"));
    assert!(html.contains("Zen Go Synergy Core"));
    assert!(html.contains("ch1Fader"));
    assert!(html.contains("ch2Fader"));
    assert!(html.contains("monFader"));
    assert!(html.contains("hp1Fader"));
    assert!(html.contains("hp2Fader"));
    assert!(html.contains("ch1PhantomBtn"));
    assert!(html.contains("monitorMuteBtn"));
    assert!(html.contains("pointerdown"));
}

#[tokio::test]
async fn test_tablet_remote_api_status_and_meters_endpoints() {
    let meters = Arc::new(AudioMeters::default());
    // Store mock meter values (atomic bits)
    AudioMeters::store_f32(&meters.in_l_peak, 0.5);
    AudioMeters::store_f32(&meters.out_l_peak, 0.707);
    AudioMeters::store_f32(&meters.comp_gr_db, 3.5);
    AudioMeters::store_f32(&meters.master_limiter_gr_db, 1.2);
    AudioMeters::store_f32(&meters.integrated_lufs, -14.1);

    let server = TabletRemoteServer::new(None, meters, 8080);

    // 1. Test /api/status
    let status_req = Request::builder()
        .uri("/api/status")
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let status_res = server.router().oneshot(status_req).await.unwrap();
    assert_eq!(status_res.status(), StatusCode::OK);

    let body = axum::body::to_bytes(status_res.into_body(), usize::MAX).await.unwrap();
    let status_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(status_json["status"], "online");
    assert_eq!(status_json["sample_rate"], 48000);

    // 2. Test /api/meters
    let meters_req = Request::builder()
        .uri("/api/meters")
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let meters_res = server.router().oneshot(meters_req).await.unwrap();
    assert_eq!(meters_res.status(), StatusCode::OK);

    let body = axum::body::to_bytes(meters_res.into_body(), usize::MAX).await.unwrap();
    let meters_json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert!((meters_json["comp_gr_db"].as_f64().unwrap() - 3.5).abs() < 1e-3);
    assert!((meters_json["lim_gr_db"].as_f64().unwrap() - 1.2).abs() < 1e-3);
    assert!((meters_json["integrated_lufs"].as_f64().unwrap() - (-14.1)).abs() < 1e-3);
    assert!((meters_json["out_l_dbfs"].as_f64().unwrap() - (-3.01)).abs() < 0.1);
}

#[test]
fn test_remote_messages_serde_protocol() {
    // 1. SetGain message
    let gain_json = r#"{"type":"set_gain","input":0,"gain_db":55}"#;
    let msg: RemoteMessage = serde_json::from_str(gain_json).unwrap();
    match msg {
        RemoteMessage::SetGain { input, gain_db } => {
            assert_eq!(input, 0);
            assert_eq!(gain_db, 55);
        }
        _ => panic!("Expected SetGain"),
    }

    // 2. SetPhantom message
    let phantom_json = r#"{"type":"set_phantom","input":1,"enabled":true}"#;
    let msg: RemoteMessage = serde_json::from_str(phantom_json).unwrap();
    match msg {
        RemoteMessage::SetPhantom { input, enabled } => {
            assert_eq!(input, 1);
            assert!(enabled);
        }
        _ => panic!("Expected SetPhantom"),
    }

    // 3. SetPhase message
    let phase_json = r#"{"type":"set_phase","input":0,"enabled":true}"#;
    let msg: RemoteMessage = serde_json::from_str(phase_json).unwrap();
    match msg {
        RemoteMessage::SetPhase { input, enabled } => {
            assert_eq!(input, 0);
            assert!(enabled);
        }
        _ => panic!("Expected SetPhase"),
    }

    // 4. SetMode message
    let mode_json = r#"{"type":"set_mode","input":0,"mode":"HiZ"}"#;
    let msg: RemoteMessage = serde_json::from_str(mode_json).unwrap();
    match msg {
        RemoteMessage::SetMode { input, mode } => {
            assert_eq!(input, 0);
            assert_eq!(mode, "HiZ");
        }
        _ => panic!("Expected SetMode"),
    }

    // 5. SetMonitorVolume and SetMonitorMute
    let mon_json = r#"{"type":"set_monitor_volume","step":18}"#;
    let msg: RemoteMessage = serde_json::from_str(mon_json).unwrap();
    match msg {
        RemoteMessage::SetMonitorVolume { step } => assert_eq!(step, 18),
        _ => panic!("Expected SetMonitorVolume"),
    }

    let mute_json = r#"{"type":"set_monitor_mute","enabled":true}"#;
    let msg: RemoteMessage = serde_json::from_str(mute_json).unwrap();
    match msg {
        RemoteMessage::SetMonitorMute { enabled } => assert!(enabled),
        _ => panic!("Expected SetMonitorMute"),
    }

    // 6. HP1 & HP2 Volume
    let hp1_json = r#"{"type":"set_hp1_volume","step":24}"#;
    let msg: RemoteMessage = serde_json::from_str(hp1_json).unwrap();
    match msg {
        RemoteMessage::SetHp1Volume { step } => assert_eq!(step, 24),
        _ => panic!("Expected SetHp1Volume"),
    }

    let hp2_json = r#"{"type":"set_hp2_volume","step":30}"#;
    let msg: RemoteMessage = serde_json::from_str(hp2_json).unwrap();
    match msg {
        RemoteMessage::SetHp2Volume { step } => assert_eq!(step, 30),
        _ => panic!("Expected SetHp2Volume"),
    }

    // 7. StateSync serialization
    let sync_msg = RemoteMessage::StateSync {
        input1: PreampChannelState {
            gain_db: 42,
            mode: "Mic".into(),
            phantom: true,
            phase_invert: false,
        },
        input2: PreampChannelState {
            gain_db: 20,
            mode: "Line".into(),
            phantom: false,
            phase_invert: true,
        },
        outputs: OutputLevelsState {
            monitor_step: 12,
            monitor_mute: false,
            hp1_step: 15,
            hp2_step: 22,
        },
        meters: LiveMetersState {
            in_l_dbfs: -18.2,
            in_r_dbfs: -22.4,
            out_l_dbfs: -12.1,
            out_r_dbfs: -12.1,
            comp_gr_db: 2.1,
            lim_gr_db: 0.0,
        },
    };

    let serialized = serde_json::to_string(&sync_msg).unwrap();
    assert!(serialized.contains(r#""type":"state_sync""#));
    assert!(serialized.contains(r#""gain_db":42"#));
    assert!(serialized.contains(r#""mode":"Mic""#));
    assert!(serialized.contains(r#""phantom":true"#));
    assert!(serialized.contains(r#""hp2_step":22"#));
}
