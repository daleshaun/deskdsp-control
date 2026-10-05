use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use tower::ServiceExt; // for `oneshot`

use deskdsp_control::audio::AudioMeters;
use deskdsp_control::remote::{LiveMetersState, OutputLevelsState, PreampChannelState, RemoteMessage, TabletRemoteServer};

#[tokio::test]
async fn test_tablet_remote_serves_html_touch_ui() {
    let meters = Arc::new(AudioMeters::default());
    let server = TabletRemoteServer::new(None, meters, 8080, None);
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

    // Verify key touch elements and styles from tablet_v2.html are present
    assert!(html.contains("THE"));
    assert!(html.contains("DESK"));
    assert!(html.contains("ZEN GO · CONTROL"));
    assert!(html.contains("PROGRAM · LIVE"));
    assert!(html.contains("data-knob=\"1\""));
    assert!(html.contains("data-knob=\"2\""));
    assert!(html.contains("data-fader=\"mon\""));
    assert!(html.contains("data-fader=\"hp1\""));
    assert!(html.contains("data-fader=\"hp2\""));
    assert!(html.contains("data-tog=\"48v\""));
    assert!(html.contains("data-tog=\"mute\""));
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

    let server = TabletRemoteServer::new(None, meters, 8080, None);

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

#[tokio::test]
async fn test_tablet_remote_token_authentication_gate() {
    let meters = Arc::new(AudioMeters::default());
    let token = "studio-safe-key-99".to_string();
    let server = TabletRemoteServer::new(None, meters, 8080, Some(token.clone()));

    // 1. Unauthenticated request to /api/status -> 401 Unauthorized
    let unauth_req = Request::builder()
        .uri("/api/status")
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let res = server.router().oneshot(unauth_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // 2. Wrong token in query -> 401 Unauthorized
    let wrong_req = Request::builder()
        .uri("/api/status?token=wrongpassword")
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let res = server.router().oneshot(wrong_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // 3. Valid token via Query Parameter ?token=... -> 200 OK
    let query_req = Request::builder()
        .uri(format!("/api/status?token={token}"))
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let res = server.router().oneshot(query_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 4. Valid token via Authorization: Bearer <TOKEN> -> 200 OK
    let bearer_req = Request::builder()
        .uri("/api/status")
        .method("GET")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let res = server.router().oneshot(bearer_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 5. Valid token via X-Remote-Token header -> 200 OK
    let custom_hdr_req = Request::builder()
        .uri("/api/status")
        .method("GET")
        .header("x-remote-token", &token)
        .body(Body::empty())
        .unwrap();
    let res = server.router().oneshot(custom_hdr_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 6. Test /api/auth endpoint
    let auth_valid = Request::builder()
        .uri(format!("/api/auth?token={token}"))
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let res = server.router().oneshot(auth_valid).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let auth_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(auth_json["authenticated"], true);
    assert_eq!(auth_json["token_required"], true);
}

/// Verification test for the REAL startup path used in main.rs:
/// A standard synchronous OS thread initializes Tokio, constructs the server
/// inside `rt.block_on(async move { ... })`, binds to an actual TCP socket,
/// and verifies HTTP responses over live wire TCP.
#[test]
fn test_tablet_remote_real_startup_thread_bind_and_respond() {
    let (tx_ready, rx_ready) = std::sync::mpsc::channel();

    let thread_handle = std::thread::Builder::new()
        .name("test-remote-startup".into())
        .spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("Failed to initialize tokio runtime");
            rt.block_on(async move {
                let meters = Arc::new(AudioMeters::default());
                // Construct inside rt.block_on exactly as main.rs does
                let server = TabletRemoteServer::new(None, meters, 0, None);
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                    .await
                    .expect("Failed to bind ephemeral test socket");
                let bound_addr = listener.local_addr().expect("Failed to query local_addr");

                // Signal readiness to caller test thread
                tx_ready.send(bound_addr).expect("Failed to send bound addr");

                let app = server.router();
                axum::serve(listener, app).await.expect("Server serve error");
            });
        })
        .expect("Failed to spawn test startup thread");

    let bound_addr = rx_ready
        .recv_timeout(Duration::from_secs(5))
        .expect("Server failed to bind and signal readiness within 5 seconds");

    // Connect via standard TCP and send a raw HTTP GET request to verify it is live and serving
    let mut stream = TcpStream::connect(bound_addr)
        .expect("Failed to connect to bound remote server socket");
    stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();

    let raw_request = format!("GET /api/status HTTP/1.1\r\nHost: {bound_addr}\r\nConnection: close\r\n\r\n");
    stream.write_all(raw_request.as_bytes()).unwrap();

    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();

    assert!(response.starts_with("HTTP/1.1 200 OK"), "Unexpected HTTP response: {response}");
    assert!(response.contains(r#""status":"online""#));
    assert!(response.contains(r#""sample_rate":48000"#));

    // Also verify root index HTML is served over the live wire
    let mut stream_html = TcpStream::connect(bound_addr)
        .expect("Failed to connect for index HTML");
    stream_html.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let raw_req_html = format!("GET / HTTP/1.1\r\nHost: {bound_addr}\r\nConnection: close\r\n\r\n");
    stream_html.write_all(raw_req_html.as_bytes()).unwrap();

    let mut html_response = String::new();
    stream_html.read_to_string(&mut html_response).unwrap();
    assert!(html_response.starts_with("HTTP/1.1 200 OK"));
    assert!(html_response.contains("THE"));
    assert!(html_response.contains("DESK"));
    assert!(html_response.contains("PROGRAM · LIVE"));

    // Cleanup: thread terminates with test completion
    drop(thread_handle);
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
