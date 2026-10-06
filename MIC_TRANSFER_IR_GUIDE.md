# DeskDSP — Microphone Transfer IR Capture & Deconvolution Guide

This guide walks you through capturing and generating authentic **Transfer Impulse Responses** ($H_{\text{transfer}} = \frac{H_{\text{target}}}{H_{\text{reference}}}$) for DeskDSP's **`MicImage`** convolution engine.

---

## 🎯 The Core Concept: Transfer IR vs. Raw IR

A standard impulse response captured from a microphone contains:
$$H_{\text{recorded}} = H_{\text{speaker}} \cdot H_{\text{room}} \cdot H_{\text{mic}}$$

If you load a raw target mic IR directly into your chain, your reference mic's frequency curve gets **double-applied** on top of the speaker and room coloration:
$$\text{Output} = H_{\text{source\_mic}} \cdot (H_{\text{speaker}} \cdot H_{\text{room}} \cdot H_{\text{target\_mic}})$$

To transform your reference mic into the target mic cleanly, you must compute the **Transfer IR**:
$$H_{\text{transfer}} = \frac{H_{\text{target}}}{H_{\text{reference}}} = \frac{H_{\text{speaker}} \cdot H_{\text{room}} \cdot H_{\text{target}}}{H_{\text{speaker}} \cdot H_{\text{room}} \cdot H_{\text{reference}}}$$

By placing both capsules side-by-side and exciting them with the same acoustic signal simultaneously, **the room reflections and speaker distortions completely cancel out**, leaving the pure acoustic differential between the two capsules!

---

## 📐 Physical Capture Setup

```
     [ Studio Monitor Speaker ]
                │
                │ ~ 1.0 - 1.5 meters (on-axis)
                ▼
        ┌──────────────┐
        │  Coincident  │
        │   Capsules   │
        └──────┬───────┘
         ┌─────┴─────┐
         │           │
     [Ref Mic]   [Target Mic]
     (Your Mic)   (Borrowed Mic)
         │           │
         ▼           ▼
     Zen Go Ch1  Zen Go Ch2
```

1. **Capsule Alignment (Coincident):**
   - Align the diaphragms of both microphones physically as close as possible (capsule-to-capsule distance $< 1\text{ cm}$).
   - Both grilles must face directly towards the studio monitor on-axis.
2. **Acoustic Positioning:**
   - Place the microphones 1.0 to 1.5 meters away from the monitor at ear level.
   - Avoid placing them directly against a wall or desk surface.
3. **Gain Matching:**
   - On the Zen Go hardware or touch remote, adjust preamp gain so both mics produce healthy levels (peaking around $-12\text{ dBFS}$ to $-6\text{ dBFS}$) during the sweep.
   - Disable all EQ, compression, and high-pass filters on the interface preamps during capture.

---

## 🚀 Step 1: Generate the Test Sweep

DeskDSP provides a built-in CLI tool with zero external dependencies:

```bash
# In the deskdsp-control repository:
target/release/ir_tool generate-sweep --output mic_sweep_48k.wav --duration 10.0
```

This generates a 10-second logarithmic sine sweep (20 Hz to 24 kHz) at 48 kHz / 24-bit PCM with raised-cosine anti-click fade windows and a 1-second silence tail.

---

## 🎙️ Step 2: Record the Test Sweep

1. Open your DAW (or a simple audio recorder) at **48 kHz / 24-bit**.
2. Put `mic_sweep_48k.wav` on a playback track routed to your studio monitors.
3. Arm two record tracks:
   - **Track 1 (Input 1):** Reference Mic (the mic you own, e.g. Rode NT1, Shure SM58)
   - **Track 2 (Input 2):** Target Mic (the mic you want to clone, e.g. Shure SM7B, Neumann U87)
4. Play the sweep and record both channels simultaneously.
5. Export both tracks as 48 kHz WAV files:
   - `my_nt1_rec.wav`
   - `target_sm7b_rec.wav`
   *(Alternatively, export a single stereo file `rec_stereo.wav` with Left = Reference, Right = Target).*

---

## ⚡ Step 3: Deconvolve the Transfer IR

Run the deconvolution tool:

```bash
target/release/ir_tool deconvolve \
    --reference my_nt1_rec.wav \
    --target target_sm7b_rec.wav \
    --output sm7b_from_nt1.wav \
    --taps 512
```

### What `ir_tool` Does Automatically:
- **Tikhonov Regularization:** Automatically computes $\frac{T \cdot R^*}{|R|^2 + \epsilon \cdot \max(|R|^2)}$ to prevent sub-bass or ultra-high frequency division blowup.
- **Out-of-Band Windowing:** Tapers below 30 Hz and above 20 kHz to eliminate ultrasonic and room rumble noise.
- **Peak Alignment:** Detects the direct acoustic arrival time and centers the main impulse with 16 samples of pre-arrival headroom.
- **Zero-Click Tail Fade:** Applies a raised-cosine decay on the final 15% of the 512 taps.
- **Normalization:** Peaks the resulting IR cleanly at $-1.0\text{ dBFS}$.

---

## 📱 Step 4: Load into DeskDSP

1. Open the Wireless Touch Tablet Remote at **`http://localhost:8080`** (or over USB reverse bridge on your tablet).
2. Ensure the channel preset is set to **Vocal**.
3. Under the **Tier B: Mic Image (Convolution)** section, tap **`LOAD IR`**.
4. Select `sm7b_from_nt1.wav` from your device.
5. The button indicator switches to **`IR: sm7b_from_nt1` (Green Active)**.
6. The audio engine immediately swaps in the `MicImage` convolution rack off-thread with zero glitching or buffer dropouts.

---

## 🔬 Honest Physics Note

An acoustic Transfer Impulse Response models the **linear on-axis transfer function** of the microphone:
- ✅ Captures precise frequency response curves, presence peaks, and capsule resonances.
- ✅ Captures phase response and group delay characteristics.
- ❌ Does **not** alter off-axis polar pattern rejection (e.g., an omni mic will not become a tight hypercardioid).
- ❌ Does **not** alter physical capsule proximity effect (the bass boost from singing 1 inch away is governed by diaphragm physics).
- ❌ Does **not** change physical capsule analog saturation or self-noise.
