pub const TABLET_TOUCH_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0, maximum-scale=1.0, user-scalable=no">
  <title>DeskDSP Remote — Zen Go Synergy Core</title>
  <style>
    :root {
      --bg: #0c0e12;
      --card-bg: rgba(26, 30, 38, 0.85);
      --card-border: rgba(255, 255, 255, 0.07);
      --card-inner: #12151b;
      --gold: #f5a623;
      --gold-glow: rgba(245, 166, 35, 0.4);
      --cyan: #00d2ff;
      --cyan-glow: rgba(0, 210, 255, 0.35);
      --red: #ff3b30;
      --red-glow: rgba(255, 59, 48, 0.45);
      --green: #30d158;
      --green-glow: rgba(48, 209, 88, 0.4);
      --text: #f0f2f5;
      --text-dim: #8a92a0;
      --fader-track: #181c24;
    }

    * {
      box-sizing: border-box;
      margin: 0;
      padding: 0;
      user-select: none;
      -webkit-user-select: none;
      touch-action: manipulation;
    }

    body {
      background: radial-gradient(circle at 50% 0%, #161b24 0%, #0a0c10 100%);
      color: var(--text);
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
      min-height: 100vh;
      overflow-x: hidden;
      padding: 16px;
      display: flex;
      flex-direction: column;
      gap: 16px;
    }

    /* Header & Status Bar */
    header {
      display: flex;
      justify-content: space-between;
      align-items: center;
      background: var(--card-bg);
      border: 1px solid var(--card-border);
      border-radius: 14px;
      padding: 12px 20px;
      backdrop-filter: blur(12px);
    }

    .brand {
      display: flex;
      align-items: center;
      gap: 12px;
    }

    .brand-logo {
      width: 14px;
      height: 14px;
      background: var(--gold);
      border-radius: 3px;
      box-shadow: 0 0 10px var(--gold-glow);
    }

    .brand-title {
      font-size: 1.1rem;
      font-weight: 700;
      letter-spacing: 1px;
      color: #fff;
    }

    .brand-sub {
      font-size: 0.75rem;
      font-weight: 500;
      color: var(--text-dim);
      letter-spacing: 0.5px;
    }

    .status-pill {
      display: flex;
      align-items: center;
      gap: 8px;
      font-size: 0.8rem;
      font-weight: 600;
      background: rgba(48, 209, 88, 0.1);
      border: 1px solid rgba(48, 209, 88, 0.25);
      color: var(--green);
      padding: 6px 14px;
      border-radius: 20px;
      transition: all 0.3s ease;
    }

    .status-pill.offline {
      background: rgba(255, 59, 48, 0.1);
      border-color: rgba(255, 59, 48, 0.25);
      color: var(--red);
    }

    .status-dot {
      width: 8px;
      height: 8px;
      border-radius: 50%;
      background: currentColor;
      box-shadow: 0 0 8px currentColor;
    }

    /* Main Console Grid */
    .console-grid {
      display: grid;
      grid-template-columns: 1fr 1fr 1.15fr;
      gap: 16px;
      flex: 1;
    }

    @media (max-width: 850px) {
      .console-grid {
        grid-template-columns: 1fr;
      }
    }

    .channel-strip {
      background: var(--card-bg);
      border: 1px solid var(--card-border);
      border-radius: 18px;
      padding: 20px;
      display: flex;
      flex-direction: column;
      gap: 18px;
      box-shadow: 0 12px 30px rgba(0, 0, 0, 0.4);
      backdrop-filter: blur(14px);
    }

    .strip-header {
      display: flex;
      justify-content: space-between;
      align-items: baseline;
      border-bottom: 1px solid rgba(255, 255, 255, 0.05);
      padding-bottom: 10px;
    }

    .strip-title {
      font-size: 1.15rem;
      font-weight: 800;
      letter-spacing: 0.5px;
      color: var(--gold);
    }

    .strip-badge {
      font-size: 0.75rem;
      font-weight: 700;
      color: var(--text-dim);
      background: rgba(255, 255, 255, 0.05);
      padding: 3px 8px;
      border-radius: 6px;
    }

    /* Button Row (Phantom, Phase, Mode) */
    .btn-row {
      display: grid;
      grid-template-columns: 1fr 1fr 1.2fr;
      gap: 10px;
    }

    .touch-btn {
      background: #181d26;
      border: 1px solid rgba(255, 255, 255, 0.1);
      color: var(--text-dim);
      border-radius: 12px;
      padding: 12px 6px;
      font-size: 0.85rem;
      font-weight: 700;
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: center;
      gap: 6px;
      cursor: pointer;
      transition: all 0.15s ease;
      touch-action: manipulation;
    }

    .touch-btn:active {
      transform: scale(0.96);
    }

    .btn-led {
      width: 8px;
      height: 8px;
      border-radius: 50%;
      background: #333a46;
      transition: all 0.2s ease;
    }

    .touch-btn.active-phantom {
      color: #fff;
      border-color: var(--red);
      background: rgba(255, 59, 48, 0.15);
    }
    .touch-btn.active-phantom .btn-led {
      background: var(--red);
      box-shadow: 0 0 10px var(--red-glow);
    }

    .touch-btn.active-phase {
      color: #fff;
      border-color: var(--gold);
      background: rgba(245, 166, 35, 0.15);
    }
    .touch-btn.active-phase .btn-led {
      background: var(--gold);
      box-shadow: 0 0 10px var(--gold-glow);
    }

    .touch-btn.mode-btn {
      color: var(--cyan);
      border-color: rgba(0, 210, 255, 0.25);
      background: rgba(0, 210, 255, 0.08);
    }

    /* Touch Fader / Slider Assembly */
    .fader-group {
      display: flex;
      flex-direction: column;
      gap: 10px;
      flex: 1;
      justify-content: center;
    }

    .fader-header {
      display: flex;
      justify-content: space-between;
      font-size: 0.85rem;
      font-weight: 600;
    }

    .fader-val {
      font-family: monospace;
      font-size: 1.25rem;
      font-weight: 800;
      color: #fff;
    }

    .fader-container {
      position: relative;
      width: 100%;
      height: 48px;
      background: var(--fader-track);
      border-radius: 12px;
      border: 1px solid rgba(255, 255, 255, 0.08);
      overflow: hidden;
      touch-action: none;
      cursor: ew-resize;
    }

    .fader-fill {
      position: absolute;
      left: 0;
      top: 0;
      bottom: 0;
      width: 50%;
      background: linear-gradient(90deg, #1b3d54 0%, var(--cyan) 100%);
      opacity: 0.8;
      border-radius: 12px;
      pointer-events: none;
      transition: width 0.05s linear;
    }

    .fader-thumb {
      position: absolute;
      top: 4px;
      bottom: 4px;
      width: 24px;
      border-radius: 8px;
      background: #e2e8f0;
      box-shadow: 0 0 12px rgba(0, 0, 0, 0.7);
      margin-left: -12px;
      pointer-events: none;
      transition: left 0.05s linear;
    }

    /* Master Section */
    .master-section {
      background: linear-gradient(180deg, rgba(30, 36, 48, 0.9) 0%, rgba(20, 24, 32, 0.9) 100%);
      border-color: rgba(245, 166, 35, 0.2);
    }

    .master-title {
      color: var(--cyan);
    }

    .master-fader .fader-fill {
      background: linear-gradient(90deg, #422915 0%, var(--gold) 100%);
    }

    /* Live LED Peak Meters */
    .meter-cluster {
      display: flex;
      flex-direction: column;
      gap: 8px;
      background: var(--card-inner);
      border: 1px solid rgba(255, 255, 255, 0.05);
      border-radius: 12px;
      padding: 12px;
    }

    .meter-label-row {
      display: flex;
      justify-content: space-between;
      font-size: 0.7rem;
      font-weight: 700;
      color: var(--text-dim);
    }

    .meter-bar {
      height: 10px;
      background: #1c222c;
      border-radius: 5px;
      overflow: hidden;
      position: relative;
    }

    .meter-level {
      height: 100%;
      width: 0%;
      background: linear-gradient(90deg, #30d158 0%, #30d158 70%, #ffd60a 85%, #ff3b30 100%);
      border-radius: 5px;
      transition: width 0.08s ease-out;
    }

    .gr-row {
      display: flex;
      justify-content: space-between;
      font-size: 0.75rem;
      font-weight: 700;
      color: var(--gold);
    }
  </style>
</head>
<body>

  <header>
    <div class="brand">
      <div class="brand-logo"></div>
      <div>
        <div class="brand-title">DESKDSP CONTROL</div>
        <div class="brand-sub">Antelope Zen Go Wireless Touch Remote</div>
      </div>
    </div>
    <div id="statusPill" class="status-pill offline">
      <div class="status-dot"></div>
      <span id="statusText">CONNECTING...</span>
    </div>
  </header>

  <div class="console-grid">

    <!-- Channel Strip 1 -->
    <div class="channel-strip">
      <div class="strip-header">
        <span class="strip-title">INPUT 1</span>
        <span id="ch1ModeBadge" class="strip-badge">MIC</span>
      </div>

      <div class="btn-row">
        <button id="ch1PhantomBtn" class="touch-btn">
          <div class="btn-led"></div>
          +48V
        </button>
        <button id="ch1PhaseBtn" class="touch-btn">
          <div class="btn-led"></div>
          PHASE
        </button>
        <button id="ch1ModeBtn" class="touch-btn mode-btn">
          MODE
        </button>
      </div>

      <div class="fader-group">
        <div class="fader-header">
          <span>PREAMP GAIN</span>
          <span id="ch1GainVal" class="fader-val">30 dB</span>
        </div>
        <div id="ch1Fader" class="fader-container" data-param="gain" data-channel="0">
          <div id="ch1FaderFill" class="fader-fill" style="width: 46%;"></div>
          <div id="ch1FaderThumb" class="fader-thumb" style="left: 46%;"></div>
        </div>
      </div>

      <div class="meter-cluster">
        <div class="meter-label-row">
          <span>INPUT PEAK</span>
          <span id="ch1PeakText">-inf dBFS</span>
        </div>
        <div class="meter-bar">
          <div id="ch1MeterBar" class="meter-level"></div>
        </div>
        <div class="gr-row">
          <span>VOCAL GR</span>
          <span id="ch1GrText">0.0 dB</span>
        </div>
      </div>
    </div>

    <!-- Channel Strip 2 -->
    <div class="channel-strip">
      <div class="strip-header">
        <span class="strip-title">INPUT 2</span>
        <span id="ch2ModeBadge" class="strip-badge">MIC</span>
      </div>

      <div class="btn-row">
        <button id="ch2PhantomBtn" class="touch-btn">
          <div class="btn-led"></div>
          +48V
        </button>
        <button id="ch2PhaseBtn" class="touch-btn">
          <div class="btn-led"></div>
          PHASE
        </button>
        <button id="ch2ModeBtn" class="touch-btn mode-btn">
          MODE
        </button>
      </div>

      <div class="fader-group">
        <div class="fader-header">
          <span>PREAMP GAIN</span>
          <span id="ch2GainVal" class="fader-val">30 dB</span>
        </div>
        <div id="ch2Fader" class="fader-container" data-param="gain" data-channel="1">
          <div id="ch2FaderFill" class="fader-fill" style="width: 46%;"></div>
          <div id="ch2FaderThumb" class="fader-thumb" style="left: 46%;"></div>
        </div>
      </div>

      <div class="meter-cluster">
        <div class="meter-label-row">
          <span>INPUT PEAK</span>
          <span id="ch2PeakText">-inf dBFS</span>
        </div>
        <div class="meter-bar">
          <div id="ch2MeterBar" class="meter-level"></div>
        </div>
      </div>
    </div>

    <!-- Master Monitor & Headphones -->
    <div class="channel-strip master-section">
      <div class="strip-header">
        <span class="strip-title master-title">MASTER OUT</span>
        <button id="monitorMuteBtn" class="touch-btn" style="padding: 4px 12px; flex-direction: row;">
          <div class="btn-led"></div>
          MUTE
        </button>
      </div>

      <!-- Monitor Out Volume -->
      <div class="fader-group master-fader">
        <div class="fader-header">
          <span>MONITOR ATTENUATION</span>
          <span id="monVolVal" class="fader-val">0 dB</span>
        </div>
        <div id="monFader" class="fader-container" data-param="monitor">
          <div id="monFaderFill" class="fader-fill" style="width: 75%;"></div>
          <div id="monFaderThumb" class="fader-thumb" style="left: 75%;"></div>
        </div>
      </div>

      <!-- HP1 Volume -->
      <div class="fader-group">
        <div class="fader-header">
          <span>HEADPHONE 1</span>
          <span id="hp1VolVal" class="fader-val">0 dB</span>
        </div>
        <div id="hp1Fader" class="fader-container" data-param="hp1">
          <div id="hp1FaderFill" class="fader-fill" style="width: 75%;"></div>
          <div id="hp1FaderThumb" class="fader-thumb" style="left: 75%;"></div>
        </div>
      </div>

      <!-- HP2 Volume -->
      <div class="fader-group">
        <div class="fader-header">
          <span>HEADPHONE 2</span>
          <span id="hp2VolVal" class="fader-val">0 dB</span>
        </div>
        <div id="hp2Fader" class="fader-container" data-param="hp2">
          <div id="hp2FaderFill" class="fader-fill" style="width: 75%;"></div>
          <div id="hp2FaderThumb" class="fader-thumb" style="left: 75%;"></div>
        </div>
      </div>

      <div class="meter-cluster">
        <div class="meter-label-row">
          <span>MASTER PEAK (L/R)</span>
          <span id="masterPeakText">-inf dBFS</span>
        </div>
        <div class="meter-bar">
          <div id="masterMeterBar" class="meter-level"></div>
        </div>
        <div class="gr-row">
          <span>LIMITER GR</span>
          <span id="limiterGrText">0.0 dB</span>
        </div>
      </div>
    </div>

  </div>

  <script>
    let socket = null;
    let isDragging = { gain0: false, gain1: false, monitor: false, hp1: false, hp2: false };
    let currentState = {
      input1: { gain_db: 30, mode: "Mic", phantom: false, phase_invert: false },
      input2: { gain_db: 30, mode: "Mic", phantom: false, phase_invert: false },
      outputs: { monitor_step: 32, monitor_mute: false, hp1_step: 32, hp2_step: 32 }
    };

    function getToken() {
      const urlParams = new URLSearchParams(window.location.search);
      return urlParams.get('token') || sessionStorage.getItem('deskdsp_token') || '';
    }

    function setToken(tok) {
      sessionStorage.setItem('deskdsp_token', tok);
      const url = new URL(window.location);
      url.searchParams.set('token', tok);
      window.history.replaceState({}, '', url);
    }

    function connectWebSocket() {
      const token = getToken();
      const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
      const url = `${proto}//${location.host}/ws` + (token ? `?token=${encodeURIComponent(token)}` : '');
      socket = new WebSocket(url);

      socket.onopen = () => {
        document.getElementById('statusPill').classList.remove('offline');
        document.getElementById('statusText').textContent = 'ONLINE (WIFI)';
      };

      socket.onclose = async () => {
        document.getElementById('statusPill').classList.add('offline');
        try {
          const res = await fetch('/api/auth' + (token ? `?token=${encodeURIComponent(token)}` : ''));
          if (res.status === 401) {
            document.getElementById('statusText').textContent = 'AUTH REQUIRED';
            const entered = prompt('DeskDSP Remote: Enter access security token:');
            if (entered) {
              setToken(entered);
              setTimeout(connectWebSocket, 300);
              return;
            }
          } else {
            document.getElementById('statusText').textContent = 'DISCONNECTED';
          }
        } catch (e) {
          document.getElementById('statusText').textContent = 'DISCONNECTED';
        }
        setTimeout(connectWebSocket, 1500);
      };

      socket.onerror = () => {
        socket.close();
      };

      socket.onmessage = (event) => {
        try {
          const msg = JSON.parse(event.data);
          if (msg.type === 'state_sync') {
            applyStateSync(msg);
          }
        } catch (e) {}
      };
    }

    function sendCmd(cmd) {
      if (socket && socket.readyState === WebSocket.OPEN) {
        socket.send(JSON.stringify(cmd));
      }
    }

    function applyStateSync(msg) {
      currentState = msg;

      // Input 1 Sync
      if (!isDragging.gain0) {
        const ratio1 = Math.min(msg.input1.gain_db / 65, 1.0);
        document.getElementById('ch1FaderFill').style.width = `${ratio1 * 100}%`;
        document.getElementById('ch1FaderThumb').style.left = `${ratio1 * 100}%`;
        document.getElementById('ch1GainVal').textContent = `${msg.input1.gain_db} dB`;
      }
      document.getElementById('ch1PhantomBtn').classList.toggle('active-phantom', msg.input1.phantom);
      document.getElementById('ch1PhaseBtn').classList.toggle('active-phase', msg.input1.phase_invert);
      document.getElementById('ch1ModeBadge').textContent = msg.input1.mode.toUpperCase();

      // Input 2 Sync
      if (!isDragging.gain1) {
        const ratio2 = Math.min(msg.input2.gain_db / 65, 1.0);
        document.getElementById('ch2FaderFill').style.width = `${ratio2 * 100}%`;
        document.getElementById('ch2FaderThumb').style.left = `${ratio2 * 100}%`;
        document.getElementById('ch2GainVal').textContent = `${msg.input2.gain_db} dB`;
      }
      document.getElementById('ch2PhantomBtn').classList.toggle('active-phantom', msg.input2.phantom);
      document.getElementById('ch2PhaseBtn').classList.toggle('active-phase', msg.input2.phase_invert);
      document.getElementById('ch2ModeBadge').textContent = msg.input2.mode.toUpperCase();

      // Master Monitor & Mute
      if (!isDragging.monitor) {
        const monRatio = 1.0 - (msg.outputs.monitor_step / 127);
        document.getElementById('monFaderFill').style.width = `${monRatio * 100}%`;
        document.getElementById('monFaderThumb').style.left = `${monRatio * 100}%`;
        document.getElementById('monVolVal').textContent = `-${msg.outputs.monitor_step} dB`;
      }
      document.getElementById('monitorMuteBtn').classList.toggle('active-phantom', msg.outputs.monitor_mute);

      // HP1
      if (!isDragging.hp1) {
        const hp1Ratio = 1.0 - (msg.outputs.hp1_step / 127);
        document.getElementById('hp1FaderFill').style.width = `${hp1Ratio * 100}%`;
        document.getElementById('hp1FaderThumb').style.left = `${hp1Ratio * 100}%`;
        document.getElementById('hp1VolVal').textContent = `-${msg.outputs.hp1_step} dB`;
      }

      // HP2
      if (!isDragging.hp2) {
        const hp2Ratio = 1.0 - (msg.outputs.hp2_step / 127);
        document.getElementById('hp2FaderFill').style.width = `${hp2Ratio * 100}%`;
        document.getElementById('hp2FaderThumb').style.left = `${hp2Ratio * 100}%`;
        document.getElementById('hp2VolVal').textContent = `-${msg.outputs.hp2_step} dB`;
      }

      // Meters
      if (msg.meters) {
        const inL = msg.meters.in_l_dbfs;
        const outL = msg.meters.out_l_dbfs;
        const inRatio = Math.max(0, Math.min(1, (inL + 60) / 60));
        const outRatio = Math.max(0, Math.min(1, (outL + 60) / 60));

        document.getElementById('ch1MeterBar').style.width = `${inRatio * 100}%`;
        document.getElementById('ch1PeakText').textContent = inL > -70 ? `${inL.toFixed(1)} dBFS` : '-inf dBFS';

        document.getElementById('masterMeterBar').style.width = `${outRatio * 100}%`;
        document.getElementById('masterPeakText').textContent = outL > -70 ? `${outL.toFixed(1)} dBFS` : '-inf dBFS';

        document.getElementById('ch1GrText').textContent = `-${msg.meters.comp_gr_db.toFixed(1)} dB`;
        document.getElementById('limiterGrText').textContent = `-${msg.meters.lim_gr_db.toFixed(1)} dB`;
      }
    }

    // Touch & Pointer Slider Setup
    function setupFader(id, dragKey, onUpdate) {
      const el = document.getElementById(id);
      let dragging = false;

      function updateFromPointer(e) {
        const rect = el.getBoundingClientRect();
        const clientX = e.touches ? e.touches[0].clientX : e.clientX;
        const ratio = Math.max(0, Math.min(1, (clientX - rect.left) / rect.width));
        onUpdate(ratio);
      }

      el.addEventListener('pointerdown', (e) => {
        dragging = true;
        isDragging[dragKey] = true;
        el.setPointerCapture(e.pointerId);
        updateFromPointer(e);
      });

      el.addEventListener('pointermove', (e) => {
        if (dragging) updateFromPointer(e);
      });

      function stopDrag() {
        if (dragging) {
          dragging = false;
          setTimeout(() => { isDragging[dragKey] = false; }, 150);
        }
      }

      el.addEventListener('pointerup', stopDrag);
      el.addEventListener('pointercancel', stopDrag);
    }

    // Input 1 Gain
    setupFader('ch1Fader', 'gain0', (ratio) => {
      const gain = Math.round(ratio * 65);
      document.getElementById('ch1FaderFill').style.width = `${ratio * 100}%`;
      document.getElementById('ch1FaderThumb').style.left = `${ratio * 100}%`;
      document.getElementById('ch1GainVal').textContent = `${gain} dB`;
      sendCmd({ type: 'set_gain', input: 0, gain_db: gain });
    });

    // Input 2 Gain
    setupFader('ch2Fader', 'gain1', (ratio) => {
      const gain = Math.round(ratio * 65);
      document.getElementById('ch2FaderFill').style.width = `${ratio * 100}%`;
      document.getElementById('ch2FaderThumb').style.left = `${ratio * 100}%`;
      document.getElementById('ch2GainVal').textContent = `${gain} dB`;
      sendCmd({ type: 'set_gain', input: 1, gain_db: gain });
    });

    // Monitor Volume (Step 0 = unity, 127 = mute)
    setupFader('monFader', 'monitor', (ratio) => {
      const step = Math.round((1.0 - ratio) * 127);
      document.getElementById('monFaderFill').style.width = `${ratio * 100}%`;
      document.getElementById('monFaderThumb').style.left = `${ratio * 100}%`;
      document.getElementById('monVolVal').textContent = `-${step} dB`;
      sendCmd({ type: 'set_monitor_volume', step });
    });

    // HP1 Volume
    setupFader('hp1Fader', 'hp1', (ratio) => {
      const step = Math.round((1.0 - ratio) * 127);
      document.getElementById('hp1FaderFill').style.width = `${ratio * 100}%`;
      document.getElementById('hp1FaderThumb').style.left = `${ratio * 100}%`;
      document.getElementById('hp1VolVal').textContent = `-${step} dB`;
      sendCmd({ type: 'set_hp1_volume', step });
    });

    // HP2 Volume
    setupFader('hp2Fader', 'hp2', (ratio) => {
      const step = Math.round((1.0 - ratio) * 127);
      document.getElementById('hp2FaderFill').style.width = `${ratio * 100}%`;
      document.getElementById('hp2FaderThumb').style.left = `${ratio * 100}%`;
      document.getElementById('hp2VolVal').textContent = `-${step} dB`;
      sendCmd({ type: 'set_hp2_volume', step });
    });

    // Toggle Buttons
    document.getElementById('ch1PhantomBtn').addEventListener('click', () => {
      const next = !currentState.input1.phantom;
      sendCmd({ type: 'set_phantom', input: 0, enabled: next });
    });

    document.getElementById('ch1PhaseBtn').addEventListener('click', () => {
      const next = !currentState.input1.phase_invert;
      sendCmd({ type: 'set_phase', input: 0, enabled: next });
    });

    document.getElementById('ch1ModeBtn').addEventListener('click', () => {
      const modes = ['Mic', 'Line', 'HiZ'];
      const curIdx = modes.indexOf(currentState.input1.mode);
      const next = modes[(curIdx + 1) % modes.length];
      sendCmd({ type: 'set_mode', input: 0, mode: next });
    });

    document.getElementById('ch2PhantomBtn').addEventListener('click', () => {
      const next = !currentState.input2.phantom;
      sendCmd({ type: 'set_phantom', input: 1, enabled: next });
    });

    document.getElementById('ch2PhaseBtn').addEventListener('click', () => {
      const next = !currentState.input2.phase_invert;
      sendCmd({ type: 'set_phase', input: 1, enabled: next });
    });

    document.getElementById('ch2ModeBtn').addEventListener('click', () => {
      const modes = ['Mic', 'Line', 'HiZ'];
      const curIdx = modes.indexOf(currentState.input2.mode);
      const next = modes[(curIdx + 1) % modes.length];
      sendCmd({ type: 'set_mode', input: 1, mode: next });
    });

    document.getElementById('monitorMuteBtn').addEventListener('click', () => {
      const next = !currentState.outputs.monitor_mute;
      sendCmd({ type: 'set_monitor_mute', enabled: next });
    });

    // Initialize Connection
    connectWebSocket();
  </script>
</body>
</html>
"#;
