import re

with open(r'd:\Study\Kikyo\crates\kikyo-ui-tauri\src\main.js', 'r', encoding='utf-8') as f:
    content = f.read()

content = content.replace('let analyticsHeatmapLayoutSel;', 'let analyticsLayoutSel;')
content = content.replace('analyticsHeatmapLayoutSel = document.querySelector("#analytics-heatmap-layout");', 'analyticsLayoutSel = document.querySelector("#analytics-layout-filter");')

# Change event listener variable
content = content.replace('if (analyticsHeatmapLayoutSel) {', 'if (analyticsLayoutSel) {')
content = content.replace('analyticsHeatmapLayoutSel.addEventListener("change", () => {', 'analyticsLayoutSel.addEventListener("change", () => {\n      updateEfficiencyDisplay();')

# Change populate code
content = content.replace('analyticsHeatmapLayoutSel.value', 'analyticsLayoutSel.value')
content = content.replace('analyticsHeatmapLayoutSel.innerHTML = \'<option value="all">全入力</option>\';', 'analyticsLayoutSel.innerHTML = \'<option value="all">全配列</option>\';')
content = content.replace('analyticsHeatmapLayoutSel.appendChild(opt);', 'analyticsLayoutSel.appendChild(opt);')

# In updateEfficiencyDisplay() we need to filter the records just like in renderHeatmap
new_efficiency = '''function updateEfficiencyDisplay() {
  if (!analyticsData) return;

  const layoutFilter = analyticsLayoutSel?.value || "all";
  let records = analyticsData.records || [];
  if (layoutFilter !== "all") {
    records = records.filter(r => r.layout_name === layoutFilter);
  }

  let totalPhysical = 0;'''

content = re.sub(r'function updateEfficiencyDisplay\(\) \{\s*if \(\!analyticsData\) return;\s*const records = analyticsData\.records \|\| \[\];\s*let totalPhysical = 0;', new_efficiency, content)

# In renderHeatmap, update to use analyticsLayoutSel
content = content.replace('const layoutFilter = analyticsHeatmapLayoutSel?.value || "all";', 'const layoutFilter = analyticsLayoutSel?.value || "all";')

with open(r'd:\Study\Kikyo\crates\kikyo-ui-tauri\src\main.js', 'w', encoding='utf-8') as f:
    f.write(content)
