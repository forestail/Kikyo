const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
let globalEnabledCb;
let statusMsg;

// Elements
let layoutPathInput, loadLayoutBtn, browseLayoutBtn;

// Sidebar
let navItems, sections;

// Profile Inputs
// Array
let charRepeatAssignedCb, charRepeatUnassignedCb;
let currentProfile = null;
let thumbLeftRepeatSetting = null;
let thumbRightRepeatSetting = null;
let extThumb1RepeatSetting = null;
let extThumb2RepeatSetting = null;

// Thumb Left
let thumbLeftKeySel, thumbLeftContinuousCb, thumbLeftSinglePressSel, thumbLeftRepeatCb;
// Thumb Right
let thumbRightKeySel, thumbRightContinuousCb, thumbRightSinglePressSel, thumbRightRepeatCb;
let thumbLeftRepeatLabel, thumbRightRepeatLabel;
// Extended Thumb 1
let extThumb1KeySel, extThumb1ContinuousCb, extThumb1SinglePressSel, extThumb1RepeatCb;
let extThumb1RepeatLabel;
// Extended Thumb 2
let extThumb2KeySel, extThumb2ContinuousCb, extThumb2SinglePressSel, extThumb2RepeatCb;
let extThumb2RepeatLabel;

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
  if (side === "left") return thumbLeftRepeatSetting;
  if (side === "right") return thumbRightRepeatSetting;
  if (side === "ext1") return extThumb1RepeatSetting;
  return extThumb2RepeatSetting;
}

function setThumbRepeatSetting(side, value) {
  if (side === "left") {
    thumbLeftRepeatSetting = value;
  } else if (side === "right") {
    thumbRightRepeatSetting = value;
  } else if (side === "ext1") {
    extThumb1RepeatSetting = value;
  } else {
    extThumb2RepeatSetting = value;
  }
}

function syncThumbRepeatUI(side) {
  let singlePressSel, repeatCb, repeatLabel;
  if (side === "left") {
    singlePressSel = thumbLeftSinglePressSel;
    repeatCb = thumbLeftRepeatCb;
    repeatLabel = thumbLeftRepeatLabel;
  } else if (side === "right") {
    singlePressSel = thumbRightSinglePressSel;
    repeatCb = thumbRightRepeatCb;
    repeatLabel = thumbRightRepeatLabel;
  } else if (side === "ext1") {
    singlePressSel = extThumb1SinglePressSel;
    repeatCb = extThumb1RepeatCb;
    repeatLabel = extThumb1RepeatLabel;
  } else {
    singlePressSel = extThumb2SinglePressSel;
    repeatCb = extThumb2RepeatCb;
    repeatLabel = extThumb2RepeatLabel;
  }
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
  // Extended Thumb 1
  if (profile.extended_thumb1) {
    if (extThumb1KeySel) extThumb1KeySel.value = profile.extended_thumb1.key;
    if (extThumb1ContinuousCb) extThumb1ContinuousCb.checked = profile.extended_thumb1.continuous;
    if (extThumb1SinglePressSel) extThumb1SinglePressSel.value = profile.extended_thumb1.single_press;
    if (extThumb1RepeatCb) extThumb1RepeatCb.checked = profile.extended_thumb1.repeat;
    setThumbRepeatSetting("ext1", profile.extended_thumb1.repeat);
  }
  // Extended Thumb 2
  if (profile.extended_thumb2) {
    if (extThumb2KeySel) extThumb2KeySel.value = profile.extended_thumb2.key;
    if (extThumb2ContinuousCb) extThumb2ContinuousCb.checked = profile.extended_thumb2.continuous;
    if (extThumb2SinglePressSel) extThumb2SinglePressSel.value = profile.extended_thumb2.single_press;
    if (extThumb2RepeatCb) extThumb2RepeatCb.checked = profile.extended_thumb2.repeat;
    setThumbRepeatSetting("ext2", profile.extended_thumb2.repeat);
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
  syncThumbRepeatUI("ext1");
  syncThumbRepeatUI("ext2");
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

  // Extended Thumb 1
  if (!currentProfile.extended_thumb1) currentProfile.extended_thumb1 = {};
  currentProfile.extended_thumb1.key = extThumb1KeySel.value;
  currentProfile.extended_thumb1.continuous = extThumb1ContinuousCb.checked;
  currentProfile.extended_thumb1.single_press = extThumb1SinglePressSel.value;
  const ext1AllowsRepeat = singlePressAllowsRepeat(extThumb1SinglePressSel.value);
  if (ext1AllowsRepeat) {
    setThumbRepeatSetting("ext1", extThumb1RepeatCb.checked);
  }
  const ext1StoredRepeat = getThumbRepeatSetting("ext1");
  currentProfile.extended_thumb1.repeat =
    typeof ext1StoredRepeat === "boolean" ? ext1StoredRepeat : false;

  // Extended Thumb 2
  if (!currentProfile.extended_thumb2) currentProfile.extended_thumb2 = {};
  currentProfile.extended_thumb2.key = extThumb2KeySel.value;
  currentProfile.extended_thumb2.continuous = extThumb2ContinuousCb.checked;
  currentProfile.extended_thumb2.single_press = extThumb2SinglePressSel.value;
  const ext2AllowsRepeat = singlePressAllowsRepeat(extThumb2SinglePressSel.value);
  if (ext2AllowsRepeat) {
    setThumbRepeatSetting("ext2", extThumb2RepeatCb.checked);
  }
  const ext2StoredRepeat = getThumbRepeatSetting("ext2");
  currentProfile.extended_thumb2.repeat =
    typeof ext2StoredRepeat === "boolean" ? ext2StoredRepeat : false;

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
    extThumb1ContinuousCb, extThumb1RepeatCb,
    extThumb2ContinuousCb, extThumb2RepeatCb,
    charContinuousCb,
  ];
  changeTargets.forEach((el) => {
    if (el) el.addEventListener("change", saveProfile);
  });
  if (charContinuousCb) charContinuousCb.addEventListener("input", saveProfile);

  const selectTargets = [
    thumbLeftKeySel,
    thumbRightKeySel,
    extThumb1KeySel,
    extThumb2KeySel,
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
  if (extThumb1SinglePressSel) {
    extThumb1SinglePressSel.addEventListener("change", () => {
      syncThumbRepeatUI("ext1");
      saveProfile();
    });
  }
  if (extThumb2SinglePressSel) {
    extThumb2SinglePressSel.addEventListener("change", () => {
      syncThumbRepeatUI("ext2");
      saveProfile();
    });
  }

  const rangeTargets = [thumbOverlapRatioInput, charOverlapRatioInput];
  rangeTargets.forEach((el) => {
    if (el) el.addEventListener("input", saveProfile);
  });
}

function keyOptionsWithoutNone(selectEl) {
  if (!selectEl) return "";
  const clone = selectEl.cloneNode(true);
  clone.querySelectorAll('option[value="None"]').forEach((el) => el.remove());
  return clone.innerHTML;
}

function ensureExtendedThumbSection() {
  const thumbSection = document.querySelector("#section-thumb");
  const chordSection = document.querySelector("#section-chord");
  if (!thumbSection || !chordSection) return;

  const navRoot = document.querySelector(".sidebar-nav");
  const chordNav = navRoot?.querySelector('[data-target="section-chord"]');
  if (navRoot && chordNav && !navRoot.querySelector('[data-target="section-extended-thumb"]')) {
    const navItem = document.createElement("li");
    navItem.className = "nav-item";
    navItem.dataset.target = "section-extended-thumb";
    navItem.innerText = "拡張親指シフト";
    navRoot.insertBefore(navItem, chordNav);
  }

  if (document.querySelector("#section-extended-thumb")) return;

  const keyOptions = keyOptionsWithoutNone(document.querySelector("#thumb-left-key"));
  const singlePressOptions = document.querySelector("#thumb-left-single-press")?.innerHTML || "";

  const section = document.createElement("div");
  section.id = "section-extended-thumb";
  section.className = "settings-section";
  section.innerHTML = `
      <h2>拡張親指シフト</h2>
      <div class="thumb-columns" style="display: flex; gap: 20px;">
        <div class="thumb-col" style="flex: 1;">
          <div class="setting-item">
            <div class="setting-label">拡張親指シフト1</div>
            <div class="setting-control">
              <select id="ext-thumb-1-key">${keyOptions}</select>
            </div>
          </div>
          <div class="setting-item">
            <div class="setting-label">連続シフト</div>
            <div class="setting-control">
              <label class="toggle-switch">
                <input type="checkbox" id="ext-thumb-1-continuous">
                <span class="slider"></span>
              </label>
            </div>
          </div>
          <div class="setting-item">
            <div class="setting-label">単打鍵</div>
            <div class="setting-control">
              <select id="ext-thumb-1-single-press">${singlePressOptions}</select>
            </div>
          </div>
          <div class="setting-item">
            <div class="setting-label" id="ext-thumb-1-repeat-label">キーリピート</div>
            <div class="setting-control">
              <label class="toggle-switch">
                <input type="checkbox" id="ext-thumb-1-repeat">
                <span class="slider"></span>
              </label>
            </div>
          </div>
        </div>
        <div class="thumb-col" style="flex: 1;">
          <div class="setting-item">
            <div class="setting-label">拡張親指シフト2</div>
            <div class="setting-control">
              <select id="ext-thumb-2-key">${keyOptions}</select>
            </div>
          </div>
          <div class="setting-item">
            <div class="setting-label">連続シフト</div>
            <div class="setting-control">
              <label class="toggle-switch">
                <input type="checkbox" id="ext-thumb-2-continuous">
                <span class="slider"></span>
              </label>
            </div>
          </div>
          <div class="setting-item">
            <div class="setting-label">単打鍵</div>
            <div class="setting-control">
              <select id="ext-thumb-2-single-press">${singlePressOptions}</select>
            </div>
          </div>
          <div class="setting-item">
            <div class="setting-label" id="ext-thumb-2-repeat-label">キーリピート</div>
            <div class="setting-control">
              <label class="toggle-switch">
                <input type="checkbox" id="ext-thumb-2-repeat">
                <span class="slider"></span>
              </label>
            </div>
          </div>
        </div>
      </div>`;
  chordSection.parentNode.insertBefore(section, chordSection);
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

  ensureExtendedThumbSection();

  // Extended Thumb 1
  extThumb1KeySel = document.querySelector("#ext-thumb-1-key");
  extThumb1ContinuousCb = document.querySelector("#ext-thumb-1-continuous");
  extThumb1SinglePressSel = document.querySelector("#ext-thumb-1-single-press");
  extThumb1RepeatCb = document.querySelector("#ext-thumb-1-repeat");
  extThumb1RepeatLabel = document.querySelector("#ext-thumb-1-repeat-label");

  // Extended Thumb 2
  extThumb2KeySel = document.querySelector("#ext-thumb-2-key");
  extThumb2ContinuousCb = document.querySelector("#ext-thumb-2-continuous");
  extThumb2SinglePressSel = document.querySelector("#ext-thumb-2-single-press");
  extThumb2RepeatCb = document.querySelector("#ext-thumb-2-repeat");
  extThumb2RepeatLabel = document.querySelector("#ext-thumb-2-repeat-label");

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

  // Autostart init
  initAutoLaunch();
  initVersion();
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

async function initVersion() {
  const el = document.getElementById("app-version-label");
  if (!el) return;
  try {
    const ver = await invoke("get_app_version");
    el.innerText = "Version " + ver;
  } catch (e) {
    console.error("Failed to get version:", e);
  }
}

async function initAutoLaunch() {
  const autoLaunchCb = document.querySelector("#auto-launch");
  if (!autoLaunchCb) return;

  try {
    const cur = await invoke('plugin:autostart|is_enabled');
    autoLaunchCb.checked = cur;
  } catch (e) {
    console.error("Autostart check failed:", e);
  }

  autoLaunchCb.addEventListener("change", async () => {
    try {
      if (autoLaunchCb.checked) {
        await invoke('plugin:autostart|enable');
      } else {
        await invoke('plugin:autostart|disable');
      }
    } catch (e) {
      console.error("Autostart toggle failed:", e);
      statusMsg.innerText = "自動起動設定エラー: " + e;
      // Revert
      autoLaunchCb.checked = !autoLaunchCb.checked;
    }
  });
}
