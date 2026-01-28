const { invoke } = window.__TAURI__.core;

let pathInput;
let statusMsg;
let enabledCb;
let chordWindowInput;
let minOverlapInput;
let maxChordSizeInput;
let currentProfile = null;

async function loadLayout() {
  if (!pathInput) return;
  const path = pathInput.value;
  try {
    statusMsg.textContent = "Loading...";
    const res = await invoke("load_yab", { path });
    statusMsg.textContent = "Success: " + res;
    // Save to localStorage
    localStorage.setItem("kikyo_path", path);
  } catch (e) {
    statusMsg.textContent = "Error: " + e;
  }
}

async function toggleEnabled() {
  if (!enabledCb) return;
  const val = enabledCb.checked;
  await invoke("set_enabled", { enabled: val });
  statusMsg.textContent = val ? "Enabled" : "Disabled";
}

async function loadProfile() {
  try {
    // 1. Get default/current from backend
    let profile = await invoke("get_profile");

    // 2. Check localStorage
    const saved = localStorage.getItem("kikyo_profile");
    if (saved) {
      try {
        const savedProfile = JSON.parse(saved);
        // Merge saved fields into profile
        profile.chord_window_ms = savedProfile.chord_window_ms;
        profile.min_overlap_ms = savedProfile.min_overlap_ms;
        profile.max_chord_size = savedProfile.max_chord_size;

        // Apply back to backend immediately
        await invoke("set_profile", { profile });
        console.log("Applied saved profile:", profile);
      } catch (e) {
        console.error("Failed to parse saved profile", e);
      }
    }

    currentProfile = profile;
    updateUI(profile);
  } catch (e) {
    statusMsg.textContent = "Error loading profile: " + e;
  }
}

function updateUI(profile) {
  if (!profile) return;
  if (chordWindowInput) chordWindowInput.value = profile.chord_window_ms;
  if (minOverlapInput) minOverlapInput.value = profile.min_overlap_ms;
  if (maxChordSizeInput) maxChordSizeInput.value = profile.max_chord_size;
}

async function applyProfile() {
  if (!currentProfile) return;

  // Update from inputs
  currentProfile.chord_window_ms = parseInt(chordWindowInput.value, 10);
  currentProfile.min_overlap_ms = parseInt(minOverlapInput.value, 10);
  currentProfile.max_chord_size = parseInt(maxChordSizeInput.value, 10);

  try {
    await invoke("set_profile", { profile: currentProfile });
    localStorage.setItem("kikyo_profile", JSON.stringify({
      chord_window_ms: currentProfile.chord_window_ms,
      min_overlap_ms: currentProfile.min_overlap_ms,
      max_chord_size: currentProfile.max_chord_size
    }));
    statusMsg.textContent = "Settings Applied";
  } catch (e) {
    statusMsg.textContent = "Error applying settings: " + e;
  }
}

window.addEventListener("DOMContentLoaded", () => {
  pathInput = document.querySelector("#path-input");
  statusMsg = document.querySelector("#status-msg");
  enabledCb = document.querySelector("#enabled-cb");
  const loadBtn = document.querySelector("#load-btn");

  chordWindowInput = document.querySelector("#chord-window");
  minOverlapInput = document.querySelector("#min-overlap");
  maxChordSizeInput = document.querySelector("#max-chord-size");
  const applyBtn = document.querySelector("#apply-settings-btn");

  loadBtn.addEventListener("click", loadLayout);
  enabledCb.addEventListener("change", toggleEnabled);
  applyBtn.addEventListener("click", applyProfile);

  // Load saved path
  const saved = localStorage.getItem("kikyo_path");
  if (saved) {
    pathInput.value = saved;
  }

  // Initialize Profile
  loadProfile();
});
