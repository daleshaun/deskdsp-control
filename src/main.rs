mod audio;
mod dsp;
mod hardware;
mod presets;
mod remote;
mod ui;

use std::sync::Arc;
use anyhow::Result;
use clap::Parser;

use audio::AudioEngine;
use hardware::HardwareController;
use ui::WorkstationApp;

#[derive(Parser, Debug)]
#[command(name = "deskdsp-control")]
#[command(about = "DeskDSP Control — Unified Vocal Channel Strip & Master Chain DSP Suite for Antelope Zen Go", long_about = None)]
struct Args {
    /// Run quick status check and exit
    #[arg(short, long)]
    status: bool,

    /// Set preamp gain for input 1 or 2 (e.g. --gain 1:40)
    #[arg(long)]
    gain: Option<String>,

    /// Toggle +48V phantom power for input (e.g. --phantom 1)
    #[arg(long)]
    phantom: Option<u8>,

    /// Set monitor output volume step (0 = Unity, 127 = Mute)
    #[arg(long)]
    monitor_vol: Option<u8>,

    /// Run non-interactive audio streaming test for N seconds
    #[arg(long)]
    test_audio: Option<u64>,

    /// Enable wireless touch tablet remote server (axum + WebSockets)
    #[arg(long, default_value_t = true)]
    remote: bool,

    /// Port for the wireless touch tablet remote server
    #[arg(long, default_value_t = 8080)]
    remote_port: u16,

    /// Run only the wireless touch tablet remote server (headless daemon mode)
    #[arg(long)]
    remote_only: bool,

    /// Optional access security token for the wireless tablet remote
    #[arg(long)]
    remote_token: Option<String>,

    /// Audio input device name (e.g. "BlackHole 2ch", "Zen Go")
    #[arg(long)]
    input_device: Option<String>,

    /// Audio output device name (e.g. "Zen Go", "BlackHole 2ch")
    #[arg(long)]
    output_device: Option<String>,

    /// List all available audio input and output devices and exit
    #[arg(long)]
    list_devices: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    if args.list_devices {
        return AudioEngine::list_devices();
    }

    println!("⚡ DeskDSP Control — Initializing Zen Go Hardware Controller...");
    let hw = match HardwareController::connect() {
        Ok(h) => {
            println!("✓ Zen Go Synergy Core connected via USB HID.");
            Some(h)
        }
        Err(e) => {
            eprintln!("Notice: Hardware HID: {e}");
            eprintln!("Proceeding in audio-only mode...");
            None
        }
    };

    // Quick CLI commands
    if let Some(gain_str) = args.gain {
        if let Some((input_str, val_str)) = gain_str.split_once(':') {
            let input = input_str.parse::<u8>().unwrap_or(1).saturating_sub(1);
            let gain: u8 = val_str.parse().unwrap_or(0);
            if let Some(h) = &hw {
                h.set_preamp_gain(input, gain)?;
                println!("Preamp {} gain set to {} dB", input + 1, gain);
            }
        }
        return Ok(());
    }

    if let Some(phantom_input) = args.phantom {
        let input = phantom_input.saturating_sub(1);
        if let Some(h) = &hw {
            h.set_phantom(input, true)?;
            println!("Preamp {} phantom power enabled (+48V)", input + 1);
        }
        return Ok(());
    }

    if let Some(vol) = args.monitor_vol {
        if let Some(h) = &hw {
            h.set_monitor_volume(vol)?;
            println!("Monitor volume set to step {}", vol);
        }
        return Ok(());
    }

    if args.status {
        if let Some(h) = &hw {
            for _ in 0..10 {
                if h.get_snapshot().is_some() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            if let Some(snap) = h.get_snapshot() {
                let pre = &snap.preamp;
                println!("--- Zen Go Hardware Status ---");
                println!("Input 1: {:?} | Gain: {} dB | 48V: {} | Phase: {}", 
                    pre.input1.mode, pre.input1.gain_raw, pre.input1.phantom_on, (pre.input1.mode_raw & 0x40) != 0);
                println!("Input 2: {:?} | Gain: {} dB | 48V: {} | Phase: {}", 
                    pre.input2.mode, pre.input2.gain_raw, pre.input2.phantom_on, (pre.input2.mode_raw & 0x40) != 0);
                println!("Monitor Vol: {}", snap.outputs[0].volume);
                println!("HP1 Vol:     {}", snap.outputs[1].volume);
                println!("HP2 Vol:     {}", snap.outputs[2].volume);
            } else {
                println!("Zen Go connected. Awaiting initial frame snapshot...");
            }
        }
        return Ok(());
    }

    println!("Initializing Real-Time Audio Engine (cpal: CoreAudio / ALSA)...");
    let audio = AudioEngine::new(args.input_device.clone(), args.output_device.clone())?;
    println!("✓ Audio Engine running at {} Hz.", audio.sample_rate);

    if let Some(secs) = args.test_audio {
        println!("Streaming real-time audio through Vocal Channel Strip & Master Chain for {} seconds...", secs);
        for i in 1..=secs {
            std::thread::sleep(std::time::Duration::from_secs(1));
            let in_l = audio::AudioMeters::load_f32(&audio.meters.in_l_peak);
            let out_l = audio::AudioMeters::load_f32(&audio.meters.out_l_peak);
            let in_db = if in_l > 1e-4 { 20.0 * in_l.log10() } else { -80.0 };
            let out_db = if out_l > 1e-4 { 20.0 * out_l.log10() } else { -80.0 };
            let comp_gr = audio::AudioMeters::load_f32(&audio.meters.comp_gr_db);
            let lim_gr = audio::AudioMeters::load_f32(&audio.meters.master_limiter_gr_db);
            let lufs = audio::AudioMeters::load_f32(&audio.meters.integrated_lufs);
            let pitch = audio::AudioMeters::load_f32(&audio.meters.tuner_detected_freq);
            
            println!(
                "[T+{i}s] In: {:>5.1} dBFS | Out: {:>5.1} dBFS | Comp GR: -{:>4.1} dB | Lim GR: -{:>4.1} dB | LUFS: {:>5.1} | Pitch: {:>5.1} Hz",
                in_db, out_db, comp_gr, lim_gr, lufs, pitch
            );
        }
        println!("✓ Audio streaming test completed successfully.");
        return Ok(());
    }

    if args.remote || args.remote_only {
        let hw_remote = hw.clone();
        let meters_remote = Arc::clone(&audio.meters);
        let bypass_remote = Arc::clone(&audio.global_bypass);
        let port = args.remote_port;
        let token = args.remote_token.clone();

        std::thread::Builder::new()
            .name("tablet-remote-server".into())
            .spawn(move || {
                let rt = tokio::runtime::Runtime::new().expect("Failed to initialize tokio runtime");
                rt.block_on(async move {
                    let server = remote::TabletRemoteServer::new(hw_remote, meters_remote, bypass_remote, port, token);
                    if let Err(e) = server.run().await {
                        eprintln!("Tablet remote server error: {e}");
                    }
                });
            })
            .expect("Failed to spawn tablet remote thread");

        println!("📡 Wireless Touch Tablet Remote running at: http://0.0.0.0:{}", args.remote_port);
    }

    if args.remote_only {
        println!("Running in wireless tablet remote headless mode. Open http://localhost:{} on tablet.", args.remote_port);
        println!("Press Ctrl+C to exit.");
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }

    println!("Starting DeskDSP Control Terminal Workstation...");

    let mut app = WorkstationApp::new(hw, audio);
    app.run()?;

    println!("DeskDSP Control closed gracefully.");
    Ok(())
}
