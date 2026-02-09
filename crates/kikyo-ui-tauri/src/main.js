const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
let globalEnabledCb;
let statusMsg;

// Elements
let layoutEntryListEl, addLayoutEntryBtn;
let layoutEntries = [];
let activeLayoutEntryId = null;
let layoutPointerDragState = null;

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

async function openLayoutFileDialog(defaultPath = null) {
  const { open } = window.__TAURI_PLUGIN_DIALOG__;
  const selected = await open({
    multiple: false,
    defaultPath: defaultPath || undefined,
    filters: [{
      name: "Yab Layout",
      extensions: ["yab", "bnz"],
    }]
  });
  return typeof selected === "string" ? selected : null;
}

function normalizeLayoutEntry(entry) {
  return {
    id: entry?.id ?? "",
    alias: entry?.alias ?? "",
    path: entry?.path ?? "",
  };
}

function moveLayoutEntryInMemory(draggedId, targetId) {
  const fromIndex = layoutEntries.findIndex((entry) => entry.id === draggedId);
  const toIndex = layoutEntries.findIndex((entry) => entry.id === targetId);
  if (fromIndex < 0 || toIndex < 0 || fromIndex === toIndex) return;
  const [moved] = layoutEntries.splice(fromIndex, 1);
  layoutEntries.splice(toIndex, 0, moved);
}

function clearLayoutEntryDragOverState() {
  document.querySelectorAll(".layout-entry-row.is-drag-over")
    .forEach((el) => el.classList.remove("is-drag-over"));
}

function cleanupLayoutPointerDrag() {
  if (!layoutPointerDragState) return;
  const {
    sourceRow,
    sourceHandle,
    pointerId,
    onPointerMove,
    onPointerUp,
  } = layoutPointerDragState;
  if (sourceRow) {
    sourceRow.classList.remove("is-dragging");
    sourceRow.style.removeProperty("transform");
    sourceRow.style.removeProperty("will-change");
  }
  if (
    sourceHandle
    && pointerId !== undefined
    && sourceHandle.hasPointerCapture
    && sourceHandle.hasPointerCapture(pointerId)
  ) {
    sourceHandle.releasePointerCapture(pointerId);
  }
  clearLayoutEntryDragOverState();
  document.body.classList.remove("layout-entry-dragging");
  if (onPointerMove) {
    window.removeEventListener("pointermove", onPointerMove);
  }
  if (onPointerUp) {
    window.removeEventListener("pointerup", onPointerUp);
    window.removeEventListener("pointercancel", onPointerUp);
  }
  layoutPointerDragState = null;
}

function findLayoutEntryRowAtPoint(clientX, clientY) {
  const target = document.elementFromPoint(clientX, clientY);
  return target ? target.closest(".layout-entry-row") : null;
}

async function persistLayoutEntryOrder() {
  try {
    const orderedIds = layoutEntries.map((entry) => entry.id);
    await invoke("reorder_layout_entries", {
      orderedIds,
      ordered_ids: orderedIds,
    });
  } catch (e) {
    statusMsg.innerText = "鬯ｮ・｣陋ｹ繝ｻ・ｽ・ｽ繝ｻ・ｳ驛｢譎｢・ｽ・ｻ郢晢ｽｻ繝ｻ・ｦ鬯ｩ謳ｾ・ｽ・ｵ郢晢ｽｻ繝ｻ・ｺ驛｢譎｢・ｽ・ｻ郢晢ｽｻ繝ｻ・ｳ鬯ｮ・ｫ繝ｻ・ｴ髯ｷ・ｴ郢晢ｽｻ繝ｻ・ｽ繝ｻ・ｽ郢晢ｽｻ繝ｻ・ｿ鬯ｩ謳ｾ・ｽ・ｵ郢晢ｽｻ繝ｻ・ｺ鬮ｯ蜈ｷ・ｽ・ｹ郢晢ｽｻ繝ｻ・ｻ鬩搾ｽｵ繝ｻ・ｺ鬯ｯ蛟ｩ・ｲ・ｻ繝ｻ・ｽ繝ｻ・ｹ髫ｴ雜｣・ｽ・｢郢晢ｽｻ繝ｻ・ｽ郢晢ｽｻ繝ｻ・ｩ鬯ｩ蟷｢・ｽ・｢髫ｴ雜｣・ｽ・｢郢晢ｽｻ繝ｻ・ｽ郢晢ｽｻ繝ｻ・ｼ: " + e;
    await refreshLayoutEntries();
  }
}

async function activateLayoutEntry(entryId) {
  if (!entryId) return;
  try {
    statusMsg.innerText = "Loading...";
    const res = await invoke("activate_layout_entry", { id: entryId });
    activeLayoutEntryId = entryId;
    statusMsg.innerText = "鬯ｯ・ｮ繝ｻ・ｫ郢晢ｽｻ繝ｻ・ｱ驛｢譎｢・ｽ・ｻ郢晢ｽｻ繝ｻ・ｭ鬯ｩ謳ｾ・ｽ・ｵ郢晢ｽｻ繝ｻ・ｺ驛｢譎｢・ｽ・ｻ郢晢ｽｻ繝ｻ・ｿ鬯ｯ・ｮ繝ｻ・ｴ鬮ｮ諛ｶ・ｽ・｣郢晢ｽｻ繝ｻ・ｽ郢晢ｽｻ繝ｻ・ｼ鬯ｩ謳ｾ・ｽ・ｵ郢晢ｽｻ繝ｻ・ｺ驛｢譎｢・ｽ・ｻ郢晢ｽｻ繝ｻ・ｿ: " + res;
    renderLayoutEntryList();
  } catch (e) {
    statusMsg.innerText = "鬯ｩ蟷｢・ｽ・｢郢晢ｽｻ繝ｻ・ｧ驛｢譎｢・ｽ・ｻ郢晢ｽｻ繝ｻ・ｨ鬯ｩ蟷｢・ｽ・｢髫ｴ雜｣・ｽ・｢郢晢ｽｻ繝ｻ・ｽ郢晢ｽｻ繝ｻ・ｩ鬯ｩ蟷｢・ｽ・｢髫ｴ雜｣・ｽ・｢郢晢ｽｻ繝ｻ・ｽ郢晢ｽｻ繝ｻ・ｼ: " + e;
    renderLayoutEntryList();
  }
}

async function updateLayoutEntryState(entryId, alias, path) {
  try {
    await invoke("update_layout_entry", { id: entryId, alias, path });
    const entry = layoutEntries.find((item) => item.id === entryId);
    if (entry) {
      entry.alias = alias;
      entry.path = path;
    }
  } catch (e) {
    statusMsg.innerText = "鬯ｮ・ｫ繝ｻ・ｴ髯ｷ・ｴ郢晢ｽｻ繝ｻ・ｽ繝ｻ・ｽ郢晢ｽｻ繝ｻ・ｴ鬯ｮ・ｫ繝ｻ・ｴ驛｢譎｢・ｽ・ｻ郢晢ｽｻ繝ｻ・ｽ郢晢ｽｻ繝ｻ・ｰ鬯ｩ蟷｢・ｽ・｢郢晢ｽｻ繝ｻ・ｧ驛｢譎｢・ｽ・ｻ郢晢ｽｻ繝ｻ・ｨ鬯ｩ蟷｢・ｽ・｢髫ｴ雜｣・ｽ・｢郢晢ｽｻ繝ｻ・ｽ郢晢ｽｻ繝ｻ・ｩ鬯ｩ蟷｢・ｽ・｢髫ｴ雜｣・ｽ・｢郢晢ｽｻ繝ｻ・ｽ郢晢ｽｻ繝ｻ・ｼ: " + e;
    await refreshLayoutEntries();
  }
}

async function deleteLayoutEntry(entryId) {
  const deletingActive = entryId === activeLayoutEntryId;
  try {
    await invoke("delete_layout_entry", { id: entryId });
    await refreshLayoutEntries();
    if (deletingActive && activeLayoutEntryId) {
      await activateLayoutEntry(activeLayoutEntryId);
    }
  } catch (e) {
    statusMsg.innerText = "鬯ｮ・ｯ繝ｻ・ｷ髯ｷ・ｿ繝ｻ・ｰ郢晢ｽｻ繝ｻ・ｼ驕ｶ荵怜・繝ｻ・ｱ郢ｧ蜍､譁舌・繝ｻ・ｧ驛｢譎｢・ｽ・ｻ郢晢ｽｻ繝ｻ・ｨ鬯ｩ蟷｢・ｽ・｢髫ｴ雜｣・ｽ・｢郢晢ｽｻ繝ｻ・ｽ郢晢ｽｻ繝ｻ・ｩ鬯ｩ蟷｢・ｽ・｢髫ｴ雜｣・ｽ・｢郢晢ｽｻ繝ｻ・ｽ郢晢ｽｻ繝ｻ・ｼ: " + e;
  }
}

function buildLayoutEntryRow(entry) {
  const row = document.createElement("div");
  row.className = "layout-entry-row";
  row.dataset.entryId = entry.id;

  const handle = document.createElement("div");
  handle.className = "layout-entry-handle";
  handle.textContent = "\u22EE\u22EE";
  handle.title = "Drag to reorder";
  handle.addEventListener("pointerdown", (event) => {
    if (event.button !== 0) return;
    event.preventDefault();
    cleanupLayoutPointerDrag();
    const pointerId = event.pointerId;
    if (handle.setPointerCapture) {
      handle.setPointerCapture(pointerId);
    }

    const onPointerMove = (moveEvent) => {
      if (moveEvent.pointerId !== pointerId) return;
      if (!layoutPointerDragState || layoutPointerDragState.draggedId !== entry.id) return;
      const offsetY = moveEvent.clientY - layoutPointerDragState.startClientY;
      if (layoutPointerDragState.sourceRow) {
        layoutPointerDragState.sourceRow.style.transform = `translateY(${offsetY}px)`;
      }
      const targetRow = findLayoutEntryRowAtPoint(moveEvent.clientX, moveEvent.clientY);
      const targetId = targetRow ? targetRow.dataset.entryId : null;
      clearLayoutEntryDragOverState();
      if (!targetId || targetId === entry.id) {
        layoutPointerDragState.targetId = null;
        return;
      }
      targetRow.classList.add("is-drag-over");
      layoutPointerDragState.targetId = targetId;
    };

    const onPointerUp = async (upEvent) => {
      if (upEvent.pointerId !== pointerId) return;
      if (!layoutPointerDragState || layoutPointerDragState.draggedId !== entry.id) return;
      const draggedId = layoutPointerDragState.draggedId;
      const currentTargetId = layoutPointerDragState.targetId;
      const fallbackTargetRow = findLayoutEntryRowAtPoint(upEvent.clientX, upEvent.clientY);
      const fallbackTargetId = fallbackTargetRow ? fallbackTargetRow.dataset.entryId : null;
      const targetId = currentTargetId || fallbackTargetId;
      cleanupLayoutPointerDrag();
      if (!targetId || draggedId === targetId) return;
      moveLayoutEntryInMemory(draggedId, targetId);
      renderLayoutEntryList();
      await persistLayoutEntryOrder();
    };

    layoutPointerDragState = {
      draggedId: entry.id,
      targetId: null,
      sourceRow: row,
      sourceHandle: handle,
      pointerId,
      startClientY: event.clientY,
      onPointerMove,
      onPointerUp,
    };
    row.classList.add("is-dragging");
    row.style.willChange = "transform";
    document.body.classList.add("layout-entry-dragging");
    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", onPointerUp);
    window.addEventListener("pointercancel", onPointerUp);
  });

  const main = document.createElement("div");
  main.className = "layout-entry-main";

  const top = document.createElement("div");
  top.className = "layout-entry-top";

  const radio = document.createElement("input");
  radio.type = "radio";
  radio.className = "layout-entry-radio";
  radio.name = "active-layout-entry";
  radio.checked = entry.id === activeLayoutEntryId;
  radio.addEventListener("change", async () => {
    if (radio.checked) {
      await activateLayoutEntry(entry.id);
    }
  });

  const aliasInput = document.createElement("input");
  aliasInput.type = "text";
  aliasInput.className = "layout-entry-alias";
  aliasInput.placeholder = "Alias";
  aliasInput.value = entry.alias || "";

  const pathRow = document.createElement("div");
  pathRow.className = "layout-entry-path-row";

  const pathInput = document.createElement("input");
  pathInput.type = "text";
  pathInput.className = "layout-entry-path";
  pathInput.placeholder = "C:\\path\\to\\layout.yab";
  pathInput.value = entry.path || "";

  aliasInput.addEventListener("change", async () => {
    await updateLayoutEntryState(entry.id, aliasInput.value, pathInput.value);
  });

  pathInput.addEventListener("change", async () => {
    await updateLayoutEntryState(entry.id, aliasInput.value, pathInput.value);
  });

  const browseBtn = document.createElement("button");
  browseBtn.type = "button";
  browseBtn.className = "layout-entry-browse-btn";
  browseBtn.textContent = "\u53C2\u7167";
  browseBtn.addEventListener("click", async () => {
    try {
      const selected = await openLayoutFileDialog(pathInput.value);
      if (!selected) return;
      pathInput.value = selected;
      await updateLayoutEntryState(entry.id, aliasInput.value, pathInput.value);
    } catch (e) {
      statusMsg.innerText = "鬩幢ｽ｢隴弱・・ｽ・ｼ隴∵腸・ｼ諞ｺﾎ斐・・ｧ郢晢ｽｻ繝ｻ・､鬩幢ｽ｢隴趣ｽ｢繝ｻ・ｽ繝ｻ・ｫ鬯ｯ・ｩ陋ｹ繝ｻ・ｽ・ｽ繝ｻ・ｸ鬮ｫ・ｰ陞｢・ｽ繝ｻ・ｧ繝ｻ・ｭ驍ｵ・ｺ鬯倩ｲｻ・ｽ・ｹ隴趣ｽ｢繝ｻ・ｽ繝ｻ・ｩ鬩幢ｽ｢隴趣ｽ｢繝ｻ・ｽ繝ｻ・ｼ: " + e;
    }
  });

  const deleteBtn = document.createElement("button");
  deleteBtn.type = "button";
  deleteBtn.className = "layout-entry-delete-btn";
  deleteBtn.title = "Delete";
  deleteBtn.setAttribute("aria-label", "Delete");
  deleteBtn.innerHTML = `
    <svg class="layout-entry-delete-icon" viewBox="0 0 24 24" aria-hidden="true">
      <path fill="currentColor" d="M9 3h6l1 2h4v2H4V5h4l1-2Zm-2 6h2v9H7V9Zm4 0h2v9h-2V9Zm4 0h2v9h-2V9ZM6 21h12a2 2 0 0 0 2-2V8H4v11a2 2 0 0 0 2 2Z"/>
    </svg>
  `;
  deleteBtn.addEventListener("click", async () => {
    await deleteLayoutEntry(entry.id);
  });

  top.appendChild(radio);
  top.appendChild(aliasInput);
  pathRow.appendChild(pathInput);
  pathRow.appendChild(browseBtn);
  main.appendChild(top);
  main.appendChild(pathRow);
  row.appendChild(handle);
  row.appendChild(main);
  row.appendChild(deleteBtn);
  return row;
}

function renderLayoutEntryList() {
  if (!layoutEntryListEl) return;
  cleanupLayoutPointerDrag();
  layoutEntryListEl.innerHTML = "";

  if (!layoutEntries.length) {
    const empty = document.createElement("div");
    empty.className = "layout-entry-empty";
    empty.innerText = "No layout entries";
    layoutEntryListEl.appendChild(empty);
    return;
  }

  layoutEntries.forEach((entry) => {
    layoutEntryListEl.appendChild(buildLayoutEntryRow(entry));
  });
}

async function refreshLayoutEntries() {
  try {
    const res = await invoke("get_layout_entries");
    layoutEntries = (res?.entries || []).map(normalizeLayoutEntry);
    activeLayoutEntryId = res?.active_layout_id || null;
    renderLayoutEntryList();
  } catch (e) {
    statusMsg.innerText = "鬯ｯ・ｯ繝ｻ・ｩ髮倶ｼ∝ｱｮ繝ｻ・ｽ繝ｻ・ｦ鬩怜遜・ｽ・ｫ驛｢譎｢・ｽ・ｻ鬯ｮ・｣陋ｹ繝ｻ・ｽ・ｽ繝ｻ・ｳ郢晢ｽｻ邵ｺ・､・つ鬯ｯ・ｮ繝ｻ・ｫ髯具ｽｹ郢晢ｽｻ繝ｻ・ｽ繝ｻ・ｽ郢晢ｽｻ繝ｻ・ｧ鬯ｮ・ｯ繝ｻ・ｷ郢晢ｽｻ繝ｻ・ｿ鬯ｯ・ｮ繝ｻ・｢繝ｻ縺､ﾂ驛｢譎｢・ｽ・ｻ郢晢ｽｻ繝ｻ・ｾ鬮ｯ・ｷ闔ｨ螟ｲ・ｽ・ｽ繝ｻ・ｱ鬩搾ｽｵ繝ｻ・ｺ鬯ｯ蛟ｩ・ｲ・ｻ繝ｻ・ｽ繝ｻ・ｹ髫ｴ雜｣・ｽ・｢郢晢ｽｻ繝ｻ・ｽ郢晢ｽｻ繝ｻ・ｩ鬯ｩ蟷｢・ｽ・｢髫ｴ雜｣・ｽ・｢郢晢ｽｻ繝ｻ・ｽ郢晢ｽｻ繝ｻ・ｼ: " + e;
  }
}

async function addLayoutEntry() {
  try {
    const selected = await openLayoutFileDialog();
    if (!selected) return;
    const wasEmpty = layoutEntries.length === 0;
    const created = await invoke("create_layout_entry_from_path", { path: selected });
    await refreshLayoutEntries();
    if (wasEmpty && created?.id) {
      await activateLayoutEntry(created.id);
    }
  } catch (e) {
    statusMsg.innerText = "鬯ｯ・ｮ繝ｻ・ｴ髫ｰ繝ｻ竏槭・・ｽ繝ｻ・ｽ郢晢ｽｻ繝ｻ・ｽ鬯ｮ・ｯ繝ｻ・ｷ髣費ｽｨ陞滂ｽｲ繝ｻ・ｽ繝ｻ・｣郢晢ｽｻ繝ｻ・ｰ鬯ｩ蟷｢・ｽ・｢郢晢ｽｻ繝ｻ・ｧ驛｢譎｢・ｽ・ｻ郢晢ｽｻ繝ｻ・ｨ鬯ｩ蟷｢・ｽ・｢髫ｴ雜｣・ｽ・｢郢晢ｽｻ繝ｻ・ｽ郢晢ｽｻ繝ｻ・ｩ鬯ｩ蟷｢・ｽ・｢髫ｴ雜｣・ｽ・｢郢晢ｽｻ繝ｻ・ｽ郢晢ｽｻ繝ｻ・ｼ: " + e;
  }
}

async function toggleEnabled() {
  if (!globalEnabledCb) return;
  const val = globalEnabledCb.checked;
  await invoke("set_enabled", { enabled: val });
  statusMsg.innerText = val ? "Enabled" : "Disabled";
}

async function loadProfile() {
  try {
    let profile = await invoke("get_profile");
    console.log("Loaded profile:", profile);
    currentProfile = profile;
    updateUI(profile);
  } catch (e) {
    statusMsg.innerText = "鬯ｩ蟷｢・ｽ・｢髫ｴ諠ｹ・ｸ讖ｸ・ｽ・ｹ繝ｻ・ｲ郢晢ｽｻ陷ｿ蜴・ｽｽ・ｺ繝ｻ・ｽ郢晢ｽｻ繝ｻ・ｹ髫ｴ蠑ｱ繝ｻ繝ｻ・ｽ繝ｻ・ｼ髫ｴ竏ｵ閻ｸ繝ｻ・ｼ隲橸ｽｺ・取鱒繝ｻ繝ｻ・ｧ驛｢譎｢・ｽ・ｻ郢晢ｽｻ繝ｻ・､鬯ｩ蟷｢・ｽ・｢髫ｴ雜｣・ｽ・｢郢晢ｽｻ繝ｻ・ｽ郢晢ｽｻ繝ｻ・ｫ鬯ｯ・ｮ繝ｻ・ｫ郢晢ｽｻ繝ｻ・ｱ驛｢譎｢・ｽ・ｻ郢晢ｽｻ繝ｻ・ｭ鬯ｩ謳ｾ・ｽ・ｵ郢晢ｽｻ繝ｻ・ｺ驛｢譎｢・ｽ・ｻ郢晢ｽｻ繝ｻ・ｿ鬯ｯ・ｮ繝ｻ・ｴ鬮ｮ諛ｶ・ｽ・｣郢晢ｽｻ繝ｻ・ｽ郢晢ｽｻ繝ｻ・ｼ鬯ｩ謳ｾ・ｽ・ｵ郢晢ｽｻ繝ｻ・ｺ驛｢譎｢・ｽ・ｻ郢晢ｽｻ繝ｻ・ｿ鬯ｩ蟷｢・ｽ・｢郢晢ｽｻ繝ｻ・ｧ驛｢譎｢・ｽ・ｻ郢晢ｽｻ繝ｻ・ｨ鬯ｩ蟷｢・ｽ・｢髫ｴ雜｣・ｽ・｢郢晢ｽｻ繝ｻ・ｽ郢晢ｽｻ繝ｻ・ｩ鬯ｩ蟷｢・ｽ・｢髫ｴ雜｣・ｽ・｢郢晢ｽｻ繝ｻ・ｽ郢晢ｽｻ繝ｻ・ｼ: " + e;
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
      if (statusMsg) statusMsg.innerText = "鬯ｩ蟷｢・ｽ・｢髫ｴ諠ｹ・ｸ讖ｸ・ｽ・ｹ繝ｻ・ｲ郢晢ｽｻ陷ｿ蜴・ｽｽ・ｺ繝ｻ・ｽ郢晢ｽｻ繝ｻ・ｹ髫ｴ蠑ｱ繝ｻ繝ｻ・ｽ繝ｻ・ｼ髫ｴ竏ｵ閻ｸ繝ｻ・ｼ隲橸ｽｺ・取鱒繝ｻ繝ｻ・ｧ驛｢譎｢・ｽ・ｻ郢晢ｽｻ繝ｻ・､鬯ｩ蟷｢・ｽ・｢髫ｴ雜｣・ｽ・｢郢晢ｽｻ繝ｻ・ｽ郢晢ｽｻ繝ｻ・ｫ鬯ｮ・ｯ繝ｻ・ｷ郢晢ｽｻ繝ｻ・ｿ鬯ｯ・ｮ繝ｻ・｢繝ｻ縺､ﾂ驛｢譎｢・ｽ・ｻ郢晢ｽｻ繝ｻ・ｾ鬮ｯ・ｷ闔ｨ螟ｲ・ｽ・ｽ繝ｻ・ｱ鬩搾ｽｵ繝ｻ・ｺ鬯ｯ蛟ｩ・ｲ・ｻ繝ｻ・ｽ繝ｻ・ｹ髫ｴ雜｣・ｽ・｢郢晢ｽｻ繝ｻ・ｽ郢晢ｽｻ繝ｻ・ｩ鬯ｩ蟷｢・ｽ・｢髫ｴ雜｣・ｽ・｢郢晢ｽｻ繝ｻ・ｽ郢晢ｽｻ繝ｻ・ｼ: " + e;
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
    statusMsg.innerText = "Profile saved";
  } catch (e) {
    statusMsg.innerText = "鬯ｮ・｣陷ｴ繝ｻ・ｽ・ｽ繝ｻ・ｫ鬮ｫ・ｴ陷ｿ髢・ｾ蜉ｱ繝ｻ繝ｻ・ｽ郢晢ｽｻ繝ｻ・ｭ鬮ｯ蛹ｺ・ｻ繧托ｽｽ・ｽ繝ｻ・･鬩搾ｽｵ繝ｻ・ｺ鬯ｯ蛟ｩ・ｲ・ｻ繝ｻ・ｽ繝ｻ・ｹ髫ｴ雜｣・ｽ・｢郢晢ｽｻ繝ｻ・ｽ郢晢ｽｻ繝ｻ・ｩ鬯ｩ蟷｢・ｽ・｢髫ｴ雜｣・ｽ・｢郢晢ｽｻ繝ｻ・ｽ郢晢ｽｻ繝ｻ・ｼ: " + e;
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
          <h3>拡張1</h3>
          <div class="setting-item">
            <div class="setting-label">シフトキー</div>
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
            <div class="setting-label">単独打鍵</div>
            <div class="setting-control">
              <select id="ext-thumb-1-single-press">${singlePressOptions}</select>
            </div>
          </div>
          <div class="setting-item">
            <div class="setting-label" id="ext-thumb-1-repeat-label">親指シフトキーリピート</div>
            <div class="setting-control">
              <label class="toggle-switch">
                <input type="checkbox" id="ext-thumb-1-repeat">
                <span class="slider"></span>
              </label>
            </div>
          </div>
        </div>
        <div class="thumb-col" style="flex: 1;">
          <h3>拡張2</h3>
          <div class="setting-item">
            <div class="setting-label">シフトキー</div>
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
            <div class="setting-label">単独打鍵</div>
            <div class="setting-control">
              <select id="ext-thumb-2-single-press">${singlePressOptions}</select>
            </div>
          </div>
          <div class="setting-item">
            <div class="setting-label" id="ext-thumb-2-repeat-label">親指シフトキーリピート</div>
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

  layoutEntryListEl = document.querySelector("#layout-entry-list");
  addLayoutEntryBtn = document.querySelector("#add-layout-entry-btn");
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
  if (addLayoutEntryBtn) {
    addLayoutEntryBtn.addEventListener("click", addLayoutEntry);
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
  refreshLayoutEntries();
  loadProfile();
  refreshEnabledState();

  window.addEventListener("focus", () => {
    refreshLayoutEntries();
    refreshEnabledState();
    loadProfile();
  });

  window.__TAURI__.event.listen("enabled-state-changed", (event) => {
    const enabled = event.payload;
    if (globalEnabledCb) globalEnabledCb.checked = enabled;
    statusMsg.innerText = enabled ? "Enabled" : "Disabled";
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
    statusMsg.innerText = enabled ? "Enabled" : "Disabled";
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
      statusMsg.innerText = "鬯ｯ・ｮ繝ｻ・｢郢晢ｽｻ繝ｻ・ｾ驛｢譎｢・ｽ・ｻ郢晢ｽｻ繝ｻ・ｪ鬯ｮ・ｯ繝ｻ・ｷ鬮｢・ｧ繝ｻ・ｴ驛｢譎｢・ｽ・ｻ驛｢譎｢・ｽ・ｻ郢晢ｽｻ繝ｻ・ｵ驛｢譎｢・ｽ・ｻ郢晢ｽｻ繝ｻ・ｷ鬯ｮ・ｯ繝ｻ・ｷ鬮｢・ｧ繝ｻ・ｴ驛｢譎｢・ｽ・ｻ驛｢譎｢・ｽ・ｻ郢晢ｽｻ繝ｻ・ｨ驛｢譎｢・ｽ・ｻ郢晢ｽｻ繝ｻ・ｭ鬯ｮ・ｯ隶厄ｽｸ繝ｻ・ｽ繝ｻ・ｳ鬮ｯ讖ｸ・ｽ・｢郢晢ｽｻ繝ｻ・ｹ鬩搾ｽｵ繝ｻ・ｺ鬯ｯ蛟ｩ・ｲ・ｻ繝ｻ・ｽ繝ｻ・ｹ髫ｴ雜｣・ｽ・｢郢晢ｽｻ繝ｻ・ｽ郢晢ｽｻ繝ｻ・ｩ鬯ｩ蟷｢・ｽ・｢髫ｴ雜｣・ｽ・｢郢晢ｽｻ繝ｻ・ｽ郢晢ｽｻ繝ｻ・ｼ: " + e;
      // Revert
      autoLaunchCb.checked = !autoLaunchCb.checked;
    }
  });
}
