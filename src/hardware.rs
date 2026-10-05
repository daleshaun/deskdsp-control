#![allow(dead_code)]
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use anyhow::{anyhow, Result};
use hidapi::{HidApi, HidDevice};

use antelope_protocol::{
    encode_command, Command, DeviceStateSnapshot, EncodeResult, Frame, OutputTarget, PreampMode,
};

pub const ZEN_GO_VID: u16 = 0x23e5;
pub const ZEN_GO_PID: u16 = 0xa015;

#[derive(Clone)]
pub struct HardwareController {
    device: Arc<Mutex<HidDevice>>,
    latest_snapshot: Arc<RwLock<Option<DeviceStateSnapshot>>>,
    _reader_thread: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl HardwareController {
    pub fn connect() -> Result<Self> {
        let api = HidApi::new()?;
        let candidates: Vec<_> = api
            .device_list()
            .filter(|d| d.vendor_id() == ZEN_GO_VID && d.product_id() == ZEN_GO_PID)
            .collect();

        if candidates.is_empty() {
            return Err(anyhow!("No Zen Go Synergy Core detected (VID: 0x{:04x}, PID: 0x{:04x})", ZEN_GO_VID, ZEN_GO_PID));
        }

        let mut opened = None;
        for c in candidates {
            if let Ok(dev) = c.open_device(&api) {
                opened = Some(dev);
                break;
            }
        }

        let device = opened.ok_or_else(|| anyhow!("Found Zen Go, but could not open USB HID interface"))?;
        let device = Arc::new(Mutex::new(device));
        let latest_snapshot = Arc::new(RwLock::new(None));

        // Start background reader thread for real-time snapshots & metering
        let dev_clone = Arc::clone(&device);
        let snap_clone = Arc::clone(&latest_snapshot);
        let reader_thread = thread::spawn(move || {
            let mut buf = [0u8; 320];
            loop {
                let read_res = {
                    if let Ok(dev) = dev_clone.lock() {
                        // Short 5ms timeout so writes from the tablet or UI never wait on long lock holds
                        dev.read_timeout(&mut buf, 5)
                    } else {
                        break;
                    }
                };

                match read_res {
                    Ok(len) if len >= 6 => {
                        if let Ok(frame) = Frame::parse(&buf[..len]) {
                            if let Frame::Snapshot { snapshot, .. } = frame {
                                if let Ok(mut snap) = snap_clone.write() {
                                    *snap = Some(snapshot);
                                }
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(_) => {
                        thread::sleep(Duration::from_millis(5));
                    }
                }

                // Yield 1ms between reads to give writing threads immediate, uncontested lock acquisition
                thread::sleep(Duration::from_millis(1));
            }
        });

        Ok(Self {
            device,
            latest_snapshot,
            _reader_thread: Arc::new(Mutex::new(Some(reader_thread))),
        })
    }

    pub fn send(&self, command: Command) -> Result<()> {
        let enc = encode_command(command);
        let dev = self.device.lock().map_err(|_| anyhow!("HID lock poisoned"))?;

        match enc {
            EncodeResult::Single(frame) => {
                dev.write(&frame)?;
            }
            EncodeResult::Multi(frames) => {
                for f in frames.iter() {
                    dev.write(f)?;
                }
            }
            EncodeResult::WithCompanion { companion, main } => {
                dev.write(companion.as_ref())?;
                dev.write(main.as_ref())?;
            }
            EncodeResult::WithRefresh(frame) => {
                dev.write(&frame)?;
            }
            EncodeResult::MixerAssignment { .. } => {
                // Not needed for core preamp/volume/DSP controls
            }
        }
        Ok(())
    }

    pub fn set_preamp_gain(&self, input: u8, gain_db: u8) -> Result<()> {
        self.send(Command::SetPreampGain {
            input: input.min(1),
            raw: gain_db.min(65),
        })
    }

    pub fn set_preamp_mode(&self, input: u8, mode: PreampMode) -> Result<()> {
        self.send(Command::SetPreampMode {
            input: input.min(1),
            mode,
        })
    }

    pub fn set_phantom(&self, input: u8, enabled: bool) -> Result<()> {
        self.send(Command::SetPreampPhantom {
            input: input.min(1),
            enabled,
        })
    }

    pub fn set_phase(&self, input: u8, enabled: bool) -> Result<()> {
        self.send(Command::SetPreampPhase {
            input: input.min(1),
            enabled,
        })
    }

    pub fn set_monitor_volume(&self, step: u8) -> Result<()> {
        self.send(Command::SetOutputVolume {
            target: OutputTarget::Monitor,
            step: step.min(127),
        })
    }

    pub fn set_hp1_volume(&self, step: u8) -> Result<()> {
        self.send(Command::SetOutputVolume {
            target: OutputTarget::Hp1,
            step: step.min(127),
        })
    }

    pub fn set_hp2_volume(&self, step: u8) -> Result<()> {
        self.send(Command::SetOutputVolume {
            target: OutputTarget::Hp2,
            step: step.min(127),
        })
    }

    pub fn set_monitor_mute(&self, enabled: bool) -> Result<()> {
        self.send(Command::SetOutputMute {
            target: OutputTarget::Monitor,
            enabled,
        })
    }

    pub fn get_snapshot(&self) -> Option<DeviceStateSnapshot> {
        self.latest_snapshot.read().ok().and_then(|s| s.clone())
    }
}
