const { invoke } = window.__TAURI__.core;

// Elements
let layoutPathInput, loadLayoutBtn;
let globalEnabledCb;
let statusMsg;

// Sidebar
let navItems, sections;

// Profile Inputs
// Array
let charRepeatAssignedCb, charRepeatUnassignedCb;
// Thumb
let thumbKeyModeSel, thumbContinuousCb, thumbSinglePressSel, thumbRepeatCb, thumbOverlapRatioInput, thumbOverlapVal;
// Chord
let charContinuousCb, charOverlapRatioInput, charOverlapVal;

let currentProfile = null;
let imeModeSel;

async function loadLayout() {
  if (!layoutPathInput) return;
  const path = layoutPathInput.value;
  try {
    statusMsg.innerText = "Loading...";
    const res = await invoke("load_yab", { path });
    statusMsg.innerText = "成功: " + res;
    localStorage.setItem("kikyo_path", path);
  } catch (e) {
    statusMsg.innerText = "エラー: " + e;
  }
}

async function toggleEnabled() {
  if (!globalEnabledCb) return;
  const val = globalEnabledCb.checked;
  await invoke("set_enabled", { enabled: val });
  statusMsg.innerText = val ? "有効" : "無効";
}

async function loadProfile() {
  try {
    let profile = await invoke("get_profile");
    console.log("Loaded profile:", profile);

    // Migration helper: if old field exists in JSON but new one doesn't (unlikely with struct, but for safety)
    // Actually backend returns default struct if missing fields, so we rely on backend default.

    currentProfile = profile;
    updateUI(profile);
  } catch (e) {
    statusMsg.innerText = "プロファイル読み込みエラー: " + e;
  }
}

function updateUI(profile) {
  if (!profile) return;

  // Boolean checkboxes
  if (charRepeatAssignedCb) charRepeatAssignedCb.checked = profile.char_key_repeat_assigned;
  if (charRepeatUnassignedCb) charRepeatUnassignedCb.checked = profile.char_key_repeat_unassigned;

  if (thumbContinuousCb) thumbContinuousCb.checked = profile.thumb_shift_continuous;
  if (thumbRepeatCb) thumbRepeatCb.checked = profile.thumb_shift_repeat;

  if (charContinuousCb) charContinuousCb.checked = profile.char_key_continuous;

  // Enums (Selects) - Backend returns string if Serialize is correctly set up with Enums
  if (thumbKeyModeSel) thumbKeyModeSel.value = profile.thumb_shift_key_mode;
  if (thumbSinglePressSel) thumbSinglePressSel.value = profile.thumb_shift_single_press;
  if (imeModeSel) imeModeSel.value = profile.ime_mode || "Auto";

  // Ranges (Floats 0.0-1.0 to 0-100)
  if (thumbOverlapRatioInput) {
    const val = Math.round(profile.thumb_shift_overlap_ratio * 100);
    thumbOverlapRatioInput.value = val;
    if (thumbOverlapVal) thumbOverlapVal.innerText = val + "%";
  }
  if (charOverlapRatioInput) {
    const val = Math.round(profile.char_key_overlap_ratio * 100);
    charOverlapRatioInput.value = val;
    if (charOverlapVal) charOverlapVal.innerText = val + "%";
  }
}

async function saveProfile() {
  if (!currentProfile) return;

  // Gather values
  currentProfile.char_key_repeat_assigned = charRepeatAssignedCb.checked;
  currentProfile.char_key_repeat_unassigned = charRepeatUnassignedCb.checked;

  currentProfile.thumb_shift_key_mode = thumbKeyModeSel.value;
  currentProfile.thumb_shift_continuous = thumbContinuousCb.checked;
  currentProfile.thumb_shift_single_press = thumbSinglePressSel.value;
  currentProfile.thumb_shift_repeat = thumbRepeatCb.checked;
  currentProfile.thumb_shift_overlap_ratio = parseInt(thumbOverlapRatioInput.value, 10) / 100.0;

  currentProfile.char_key_continuous = charContinuousCb.checked;
  currentProfile.char_key_overlap_ratio = parseInt(charOverlapRatioInput.value, 10) / 100.0;
  if (imeModeSel) currentProfile.ime_mode = imeModeSel.value;

  try {
    console.log("Saving profile:", currentProfile);
    await invoke("set_profile", { profile: currentProfile });
    statusMsg.innerText = "設定を保存しました";

    // Save minimal local storage if needed, or just rely on backend? 
    // User didn't ask for persistence beyond session/backend, but we probably should.
    // For now, relying on backend (in-memory) and localStorage for PATH.
    // Ideally backend should save to disk.
  } catch (e) {
    statusMsg.innerText = "保存エラー: " + e;
  }
}

function setupAutoSave() {
  const changeTargets = [
    charRepeatAssignedCb,
    charRepeatUnassignedCb,
    thumbContinuousCb,
    thumbRepeatCb,
    charContinuousCb,
  ];
  changeTargets.forEach((el) => {
    if (el) el.addEventListener("change", saveProfile);
  });

  const selectTargets = [thumbKeyModeSel, thumbSinglePressSel, imeModeSel];
  selectTargets.forEach((el) => {
    if (el) el.addEventListener("change", saveProfile);
  });

  const rangeTargets = [thumbOverlapRatioInput, charOverlapRatioInput];
  rangeTargets.forEach((el) => {
    if (el) el.addEventListener("input", saveProfile);
  });
}

// Sidebar logic
function setupSidebar() {
  navItems.forEach(item => {
    item.addEventListener("click", () => {
      // Remove active class
      navItems.forEach(n => n.classList.remove("active"));
      sections.forEach(s => s.classList.remove("active"));

      // Add active
      item.classList.add("active");
      const targetId = item.dataset.target;
      document.getElementById(targetId).classList.add("active");
    });
  });
}

window.addEventListener("DOMContentLoaded", () => {
  // Elements binding
  statusMsg = document.querySelector("#status-msg");

  layoutPathInput = document.querySelector("#layout-path");
  loadLayoutBtn = document.querySelector("#load-layout-btn");
  globalEnabledCb = document.querySelector("#global-enabled");

  // Arr
  charRepeatAssignedCb = document.querySelector("#char-repeat-assigned");
  charRepeatUnassignedCb = document.querySelector("#char-repeat-unassigned");

  // Thumb
  thumbKeyModeSel = document.querySelector("#thumb-key-mode");
  thumbContinuousCb = document.querySelector("#thumb-continuous");
  thumbSinglePressSel = document.querySelector("#thumb-single-press");
  thumbRepeatCb = document.querySelector("#thumb-repeat");
  thumbOverlapRatioInput = document.querySelector("#thumb-overlap-ratio");
  thumbOverlapVal = document.querySelector("#thumb-overlap-val");

  // Chord
  charContinuousCb = document.querySelector("#char-continuous");
  charOverlapRatioInput = document.querySelector("#char-overlap-ratio");
  charOverlapVal = document.querySelector("#char-overlap-val");
  imeModeSel = document.querySelector("#ime-mode");

  // Sidebar
  navItems = document.querySelectorAll(".nav-item");
  sections = document.querySelectorAll(".settings-section");
  setupSidebar();

  // Listeners
  loadLayoutBtn.addEventListener("click", loadLayout);
  globalEnabledCb.addEventListener("change", toggleEnabled);
  // Range Listeners for value update
  thumbOverlapRatioInput.addEventListener("input", (e) => {
    if (thumbOverlapVal) thumbOverlapVal.innerText = e.target.value + "%";
  });
  charOverlapRatioInput.addEventListener("input", (e) => {
    if (charOverlapVal) charOverlapVal.innerText = e.target.value + "%";
  });
  setupAutoSave();

  // Init
  const savedPath = localStorage.getItem("kikyo_path");
  if (savedPath) layoutPathInput.value = savedPath;

  loadProfile();
  refreshEnabledState();

  window.addEventListener("focus", () => {
    refreshEnabledState();
    loadProfile();
  });

  window.__TAURI__.event.listen("enabled-state-changed", (event) => {
    const enabled = event.payload;
    if (globalEnabledCb) globalEnabledCb.checked = enabled;
    statusMsg.innerText = enabled ? "有効" : "無効";
  });
});

async function refreshEnabledState() {
  if (!globalEnabledCb) return;
  try {
    const enabled = await invoke("get_enabled");
    globalEnabledCb.checked = enabled;
    statusMsg.innerText = enabled ? "有効" : "無効";
  } catch (e) {
    console.error(e);
  }
}
