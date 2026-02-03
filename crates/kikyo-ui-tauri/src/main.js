const { invoke } = window.__TAURI__.core;

// Elements
let layoutPathInput, loadLayoutBtn, browseLayoutBtn;
let globalEnabledCb;
let statusMsg;

// Sidebar
let navItems, sections;

// Profile Inputs
// Array
let charRepeatAssignedCb, charRepeatUnassignedCb;
let currentProfile = null;
let thumbLeftRepeatSetting = null;
let thumbRightRepeatSetting = null;

// Thumb Left
let thumbLeftKeySel, thumbLeftContinuousCb, thumbLeftSinglePressSel, thumbLeftRepeatCb;
// Thumb Right
let thumbRightKeySel, thumbRightContinuousCb, thumbRightSinglePressSel, thumbRightRepeatCb;
let thumbLeftRepeatLabel, thumbRightRepeatLabel;

// Thumb Common
let thumbOverlapRatioInput, thumbOverlapVal;

// Chord
let charContinuousCb, charOverlapRatioInput, charOverlapVal;

// Operation
let imeModeSel, suspendKeySel;

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

async function browseLayout() {
  try {
    // Use the global Tauri dialog API
    const { open } = window.__TAURI_PLUGIN_DIALOG__;
    const selected = await open({
      multiple: false,
      filters: [{
        name: 'Yab Layout',
        extensions: ['yab', 'bnz']
      }]
    });

    if (selected) {
      layoutPathInput.value = selected;
    }
  } catch (e) {
    console.error("File dialog error:", e);
    statusMsg.innerText = "ファイル選択エラー: " + e;
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
    currentProfile = profile;
    updateUI(profile);
  } catch (e) {
    statusMsg.innerText = "プロファイル読み込みエラー: " + e;
  }
}

function singlePressAllowsRepeat(value) {
  return value === "Enable" || value === "SpaceKey";
}

function getThumbRepeatSetting(side) {
  return side === "left" ? thumbLeftRepeatSetting : thumbRightRepeatSetting;
}

function setThumbRepeatSetting(side, value) {
  if (side === "left") {
    thumbLeftRepeatSetting = value;
  } else {
    thumbRightRepeatSetting = value;
  }
}

function syncThumbRepeatUI(side) {
  const singlePressSel = side === "left" ? thumbLeftSinglePressSel : thumbRightSinglePressSel;
  const repeatCb = side === "left" ? thumbLeftRepeatCb : thumbRightRepeatCb;
  const repeatLabel = side === "left" ? thumbLeftRepeatLabel : thumbRightRepeatLabel;
  if (!singlePressSel || !repeatCb) return;

  const allowRepeat = singlePressAllowsRepeat(singlePressSel.value);
  if (allowRepeat) {
    repeatCb.disabled = false;
    const stored = getThumbRepeatSetting(side);
    if (typeof stored === "boolean") repeatCb.checked = stored;
    if (repeatLabel) repeatLabel.classList.remove("is-disabled");
  } else {
    repeatCb.checked = false;
    repeatCb.disabled = true;
    if (repeatLabel) repeatLabel.classList.add("is-disabled");
  }
}

function updateUI(profile) {
  if (!profile) return;

  // Boolean checkboxes
  if (charRepeatAssignedCb) charRepeatAssignedCb.checked = profile.char_key_repeat_assigned;
  if (charRepeatUnassignedCb) charRepeatUnassignedCb.checked = profile.char_key_repeat_unassigned;

  if (charContinuousCb) charContinuousCb.checked = profile.char_key_continuous;

  // Left Thumb
  if (profile.thumb_left) {
    if (thumbLeftKeySel) thumbLeftKeySel.value = profile.thumb_left.key;
    if (thumbLeftContinuousCb) thumbLeftContinuousCb.checked = profile.thumb_left.continuous;
    if (thumbLeftSinglePressSel) thumbLeftSinglePressSel.value = profile.thumb_left.single_press;
    if (thumbLeftRepeatCb) thumbLeftRepeatCb.checked = profile.thumb_left.repeat;
    setThumbRepeatSetting("left", profile.thumb_left.repeat);
  }
  // Right Thumb
  if (profile.thumb_right) {
    if (thumbRightKeySel) thumbRightKeySel.value = profile.thumb_right.key;
    if (thumbRightContinuousCb) thumbRightContinuousCb.checked = profile.thumb_right.continuous;
    if (thumbRightSinglePressSel) thumbRightSinglePressSel.value = profile.thumb_right.single_press;
    if (thumbRightRepeatCb) thumbRightRepeatCb.checked = profile.thumb_right.repeat;
    setThumbRepeatSetting("right", profile.thumb_right.repeat);
  }

  // Common
  if (imeModeSel) imeModeSel.value = profile.ime_mode || "Auto";
  if (suspendKeySel) suspendKeySel.value = profile.suspend_key || "None";

  // Ranges
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

  syncThumbRepeatUI("left");
  syncThumbRepeatUI("right");
}

async function saveProfile() {
  if (!currentProfile) {
    try {
      currentProfile = await invoke("get_profile");
    } catch (e) {
      if (statusMsg) statusMsg.innerText = "プロファイル取得エラー: " + e;
      return;
    }
  }

  // Gather values
  currentProfile.char_key_repeat_assigned = charRepeatAssignedCb.checked;
  currentProfile.char_key_repeat_unassigned = charRepeatUnassignedCb.checked;

  // Left Thumb
  if (!currentProfile.thumb_left) currentProfile.thumb_left = {};
  currentProfile.thumb_left.key = thumbLeftKeySel.value;
  currentProfile.thumb_left.continuous = thumbLeftContinuousCb.checked;
  currentProfile.thumb_left.single_press = thumbLeftSinglePressSel.value;
  const leftAllowsRepeat = singlePressAllowsRepeat(thumbLeftSinglePressSel.value);
  if (leftAllowsRepeat) {
    setThumbRepeatSetting("left", thumbLeftRepeatCb.checked);
  }
  const leftStoredRepeat = getThumbRepeatSetting("left");
  currentProfile.thumb_left.repeat = typeof leftStoredRepeat === "boolean" ? leftStoredRepeat : false;

  // Right Thumb
  if (!currentProfile.thumb_right) currentProfile.thumb_right = {};
  currentProfile.thumb_right.key = thumbRightKeySel.value;
  currentProfile.thumb_right.continuous = thumbRightContinuousCb.checked;
  currentProfile.thumb_right.single_press = thumbRightSinglePressSel.value;
  const rightAllowsRepeat = singlePressAllowsRepeat(thumbRightSinglePressSel.value);
  if (rightAllowsRepeat) {
    setThumbRepeatSetting("right", thumbRightRepeatCb.checked);
  }
  const rightStoredRepeat = getThumbRepeatSetting("right");
  currentProfile.thumb_right.repeat = typeof rightStoredRepeat === "boolean" ? rightStoredRepeat : false;

  // Common
  if (thumbOverlapRatioInput) {
    currentProfile.thumb_shift_overlap_ratio =
      parseInt(thumbOverlapRatioInput.value, 10) / 100.0;
  }

  if (charContinuousCb) currentProfile.char_key_continuous = charContinuousCb.checked;
  if (charOverlapRatioInput) {
    currentProfile.char_key_overlap_ratio =
      parseInt(charOverlapRatioInput.value, 10) / 100.0;
  }
  if (imeModeSel) currentProfile.ime_mode = imeModeSel.value;
  if (suspendKeySel) currentProfile.suspend_key = suspendKeySel.value;

  try {
    console.log("Saving profile:", currentProfile);
    await invoke("set_profile", { profile: currentProfile });
    statusMsg.innerText = "設定を保存しました";
  } catch (e) {
    statusMsg.innerText = "保存エラー: " + e;
  }
}

function setupAutoSave() {
  const changeTargets = [
    charRepeatAssignedCb,
    charRepeatUnassignedCb,
    thumbLeftContinuousCb, thumbLeftRepeatCb,
    thumbRightContinuousCb, thumbRightRepeatCb,
    charContinuousCb,
  ];
  changeTargets.forEach((el) => {
    if (el) el.addEventListener("change", saveProfile);
  });
  if (charContinuousCb) charContinuousCb.addEventListener("input", saveProfile);

  const selectTargets = [
    thumbLeftKeySel,
    thumbRightKeySel,
    imeModeSel, suspendKeySel
  ];
  selectTargets.forEach((el) => {
    if (el) el.addEventListener("change", saveProfile);
  });

  if (thumbLeftSinglePressSel) {
    thumbLeftSinglePressSel.addEventListener("change", () => {
      syncThumbRepeatUI("left");
      saveProfile();
    });
  }
  if (thumbRightSinglePressSel) {
    thumbRightSinglePressSel.addEventListener("change", () => {
      syncThumbRepeatUI("right");
      saveProfile();
    });
  }

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
  browseLayoutBtn = document.querySelector("#browse-layout-btn");
  globalEnabledCb = document.querySelector("#global-enabled");

  // Arr
  charRepeatAssignedCb = document.querySelector("#char-repeat-assigned");
  charRepeatUnassignedCb = document.querySelector("#char-repeat-unassigned");

  // Thumb Left
  thumbLeftKeySel = document.querySelector("#thumb-left-key");
  thumbLeftContinuousCb = document.querySelector("#thumb-left-continuous");
  thumbLeftSinglePressSel = document.querySelector("#thumb-left-single-press");
  thumbLeftRepeatCb = document.querySelector("#thumb-left-repeat");
  thumbLeftRepeatLabel = document.querySelector("#thumb-left-repeat-label");

  // Thumb Right
  thumbRightKeySel = document.querySelector("#thumb-right-key");
  thumbRightContinuousCb = document.querySelector("#thumb-right-continuous");
  thumbRightSinglePressSel = document.querySelector("#thumb-right-single-press");
  thumbRightRepeatCb = document.querySelector("#thumb-right-repeat");
  thumbRightRepeatLabel = document.querySelector("#thumb-right-repeat-label");

  // Reset old binding if any
  thumbOverlapRatioInput = document.querySelector("#thumb-overlap-ratio");
  thumbOverlapVal = document.querySelector("#thumb-overlap-val");

  // Chord
  charContinuousCb = document.querySelector("#char-continuous");
  charOverlapRatioInput = document.querySelector("#char-overlap-ratio");
  charOverlapVal = document.querySelector("#char-overlap-val");

  // Op
  imeModeSel = document.querySelector("#ime-mode");
  suspendKeySel = document.querySelector("#suspend-key");

  // Sidebar
  navItems = document.querySelectorAll(".nav-item");
  sections = document.querySelectorAll(".settings-section");
  setupSidebar();

  // Listeners
  loadLayoutBtn.addEventListener("click", loadLayout);
  if (browseLayoutBtn) {
    browseLayoutBtn.addEventListener("click", browseLayout);
  }
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
